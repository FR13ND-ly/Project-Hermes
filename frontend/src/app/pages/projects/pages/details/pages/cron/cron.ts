import { Component, inject, signal, OnInit, OnDestroy, effect, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { ProjectService, CronJob, CronJobLog, EnvVarInput, ProjectEnvResponse } from '../../../../../../core/services/project.service';
import { DatabaseService } from '../../../../../../core/services/database.service';
import { StorageService } from '../../../../../../core/services/storage.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { EnvLinkModal } from '../../../../../../shared/components/env-link-modal/env-link-modal';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

type CronTargetType = 'app' | 'database' | 'storage';

@Component({
  selector: 'app-project-cron',
  standalone: true,
  imports: [CommonModule, FormsModule, DatePipe, Pagination, EnvLinkModal],
  templateUrl: './cron.html',
})
export class CronComponent implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly dbService = inject(DatabaseService);
  private readonly storageService = inject(StorageService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);

  readonly cronJobs = signal<CronJob[]>([]);
  readonly cronPage = signal(1);
  readonly cronListPageSize = signal(DEFAULT_PAGE_SIZE);
  readonly cronTotal = signal(0);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Logs modal signals (compatibility)
  readonly cronLogs = signal<CronJobLog[]>([]);
  readonly cronLogsLoading = signal(false);
  readonly selectedCronJob = signal<CronJob | null>(null);
  readonly showCronLogsModal = signal(false);
  
  // Pagination signals
  readonly currentPage = signal(1);
  readonly pageSize = signal(10);
  readonly totalLogs = signal(0);
  readonly totalPages = signal(0);
  private wsSubscriptions = new Subscription();

  // Tab navigation inside selected cron details
  readonly activeTab = signal<'details' | 'logs' | 'settings'>('details');

  // Edit fields
  readonly editName = signal('');
  readonly editSchedule = signal('');
  readonly editCommand = signal('');
  readonly editAppId = signal('');
  readonly editStatus = signal<'active' | 'paused' | 'failed'>('active');
  readonly savingSettings = signal(false);

  // Creation modal signals
  readonly showCreateModal = signal(false);
  readonly creatingCron = signal(false);

  // Form Fields
  readonly newCronName = signal('');
  readonly newCronSchedule = signal('*/5 * * * *');
  readonly newCronCommand = signal('echo "Hello World"');
  readonly selectedAppId = signal('');

  // --- Env for the cron run: custom rows + project-pool links (mirrors app creation) ---
  readonly newCronEnvRows = signal<EnvVarInput[]>([]);
  readonly projectEnvPool = signal<ProjectEnvResponse[]>([]);
  readonly selectedProjectEnvIds = signal<string[]>([]);
  readonly showCreateEnvModal = signal(false);

  // Pool vars decorated with a `linked` flag reflecting the local selection, so the
  // shared modal renders them with the same toggle UI as app creation.
  readonly projectEnvForModal = computed<ProjectEnvResponse[]>(() => {
    const selected = this.selectedProjectEnvIds();
    return this.projectEnvPool().map(v => ({ ...v, linked: selected.includes(v.id) }));
  });

  addEnvRow(): void {
    this.newCronEnvRows.update(rows => [...rows, { key: '', value: '', isSecret: false }]);
  }

  removeEnvRow(index: number): void {
    this.newCronEnvRows.update(rows => rows.filter((_, i) => i !== index));
  }

  updateEnvRow(index: number, field: 'key' | 'value', value: string): void {
    this.newCronEnvRows.update(rows =>
      rows.map((row, i) => (i === index ? { ...row, [field]: value } : row))
    );
  }

  toggleEnvRowSecret(index: number): void {
    this.newCronEnvRows.update(rows =>
      rows.map((row, i) => (i === index ? { ...row, isSecret: !row.isSecret } : row))
    );
  }

  openCreateEnvModal(): void {
    const projectId = this.parent.projectId();
    if (projectId) {
      this.projectService.listProjectEnv(projectId).subscribe({
        next: (res) => this.projectEnvPool.set(res || []),
        error: () => this.projectEnvPool.set([])
      });
    }
    this.showCreateEnvModal.set(true);
  }

  toggleCreateEnvSelection(env: ProjectEnvResponse): void {
    this.selectedProjectEnvIds.update(ids =>
      ids.includes(env.id) ? ids.filter(id => id !== env.id) : [...ids, env.id]
    );
  }

  // Target selection (app / database / storage)
  readonly newCronTargetType = signal<CronTargetType>('app');
  readonly selectedTargetId = signal('');
  readonly databases = signal<{ id: string; name: string }[]>([]);
  readonly storages = signal<{ id: string; name: string }[]>([]);

  // Options for the resource dropdown, derived from the chosen target type.
  readonly targetOptions = computed<{ id: string; name: string }[]>(() => {
    switch (this.newCronTargetType()) {
      case 'database': return this.databases();
      case 'storage': return this.storages();
      default: return this.parent.apps().map(a => ({ id: a.id, name: a.name }));
    }
  });

  private defaultCommandFor(type: CronTargetType): string {
    switch (type) {
      case 'database': return 'psql "$DATABASE_URL" -c "SELECT now();"';
      case 'storage': return 'curl -s -H "Authorization: Bearer $BUCKET_TOKEN" "$BUCKET_URL"';
      default: return 'echo "Hello World"';
    }
  }

  onChangeTargetType(type: string): void {
    const t = type as CronTargetType;
    this.newCronTargetType.set(t);
    const first = this.targetOptions()[0];
    this.selectedTargetId.set(first ? first.id : '');
    this.newCronCommand.set(this.defaultCommandFor(t));
  }

  targetTypeLabel(type: string | undefined): string {
    switch (type) {
      case 'database': return 'Database';
      case 'storage': return 'Storage';
      default: return 'Application';
    }
  }

  constructor() {
    effect(() => {
      const projId = this.parent.projectId();
      if (projId) {
        this.loadCronJobs();
      }
    });
  }

  ngOnInit(): void {
    this.loadCronJobs();
    this.setupWsSubscriptions();
  }

  ngOnDestroy(): void {
    this.wsSubscriptions.unsubscribe();
  }

  loadCronJobs(): void {
    const projId = this.parent.projectId();
    if (!projId) return;

    this.loading.set(true);
    this.error.set(null);

    this.projectService.listProjectCronJobs(projId, this.cronPage(), this.cronListPageSize()).subscribe({
      next: (res) => {
        this.cronJobs.set(res?.items || []);
        this.cronTotal.set(res?.total || 0);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Failed to load cron jobs.');
        this.loading.set(false);
      }
    });
  }

  onCronPageChange(page: number): void {
    this.cronPage.set(page);
    this.loadCronJobs();
  }

  // Helper to find the app name by app_id
  getAppName(appId: string | undefined): string {
    if (!appId) return 'Unspecified';
    const app = this.parent.apps().find(a => a.id === appId);
    return app ? app.name : appId.substring(0, 8);
  }

  onOpenCreateModal(): void {
    const projId = this.parent.projectId();
    // Load database + storage targets for the resource dropdown.
    if (projId) {
      this.dbService.listDatabases(projId, 1, 1000).subscribe({
        next: (res) => this.databases.set((res?.items || []).map(d => ({ id: d.id, name: d.name }))),
        error: () => this.databases.set([])
      });
      this.storageService.listBuckets().subscribe({
        next: (res) => this.storages.set((res || []).map(b => ({ id: b.id, name: b.name }))),
        error: () => this.storages.set([])
      });
    }
    this.newCronTargetType.set('app');
    const apps = this.parent.apps();
    this.selectedTargetId.set(apps[0]?.id || '');
    this.newCronName.set('');
    this.newCronSchedule.set('*/5 * * * *');
    this.newCronCommand.set('echo "Hello World"');
    this.newCronEnvRows.set([]);
    this.selectedProjectEnvIds.set([]);
    this.projectEnvPool.set([]);
    this.showCreateModal.set(true);
  }

  onCreateCronJob(): void {
    const projId = this.parent.projectId();
    const targetType = this.newCronTargetType();
    const targetId = this.selectedTargetId();
    if (!projId) return;
    if (!targetId) {
      this.toast.error('Select a target resource.');
      return;
    }

    const name = this.newCronName().trim();
    const schedule = this.newCronSchedule().trim();
    const command = this.newCronCommand().trim();

    if (!name || !schedule || !command) {
      this.toast.error('All fields are required.');
      return;
    }

    const envVariables = this.newCronEnvRows()
      .map(row => ({ key: row.key.trim(), value: row.value, isSecret: row.isSecret }))
      .filter(row => row.key.length > 0);

    this.creatingCron.set(true);
    this.projectService.createCronJob({
      projectId: projId,
      targetType,
      targetId,
      name,
      schedule,
      command,
      envVariables: envVariables.length > 0 ? envVariables : undefined,
      linkedProjectEnvIds: this.selectedProjectEnvIds().length > 0 ? this.selectedProjectEnvIds() : undefined
    }).subscribe({
      next: () => {
        this.creatingCron.set(false);
        this.showCreateModal.set(false);
        this.toast.success('Cron job created successfully!');
        this.loadCronJobs();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to create cron job.');
        this.creatingCron.set(false);
      }
    });
  }

  async onDeleteCronJob(jobId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Cron Job',
      message: 'Are you sure you want to delete this cron job?',
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteCronJob(jobId).subscribe({
      next: () => {
        this.toast.success('Cron job deleted.');
        if (this.selectedCronJob()?.id === jobId) {
          this.deselectCronJob();
        }
        this.loadCronJobs();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete cron job.');
      }
    });
  }

  selectCronJob(job: CronJob): void {
    this.selectedCronJob.set(job);
    this.activeTab.set('details');

    // Populate editing fields
    this.editName.set(job.name);
    this.editSchedule.set(job.schedule);
    this.editCommand.set(job.command);
    this.editAppId.set(job.appId || job.app_id || '');
    this.editStatus.set((job.status?.toLowerCase() as any) || 'active');

    // Load logs for page 1
    this.currentPage.set(1);
    this.loadCronLogs(1, this.pageSize());
  }

  deselectCronJob(): void {
    this.selectedCronJob.set(null);
    this.cronLogs.set([]);
  }

  loadCronLogs(page: number = 1, limit: number = 10, silent: boolean = false): void {
    const job = this.selectedCronJob();
    if (!job) return;

    if (!silent) {
      this.cronLogsLoading.set(true);
    }

    this.projectService.listCronJobLogs(job.id, page, limit).subscribe({
      next: (res) => {
        // Compatibility check: res can be a paginated object or flat array
        if (res && 'logs' in res) {
          this.cronLogs.set(res.logs || []);
          this.totalLogs.set(res.total || 0);
          this.currentPage.set(res.page || 1);
          this.pageSize.set(res.limit || 10);
          this.totalPages.set(res.pages || 0);
        } else {
          // Fallback if API returned flat array
          const arr = (res as any) || [];
          this.cronLogs.set(arr);
          this.totalLogs.set(arr.length);
          this.currentPage.set(1);
          this.totalPages.set(1);
        }
        if (!silent) {
          this.cronLogsLoading.set(false);
        }
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to load logs.');
        if (!silent) {
          this.cronLogsLoading.set(false);
        }
      }
    });
  }

  onPageChange(page: number): void {
    if (page < 1 || page > this.totalPages()) return;
    this.loadCronLogs(page, this.pageSize());
  }

  onTabChange(tab: 'details' | 'logs' | 'settings'): void {
    this.activeTab.set(tab);
    if (tab === 'logs') {
      this.currentPage.set(1);
      this.loadCronLogs(1, this.pageSize());
    }
  }

  onSaveSettings(): void {
    const job = this.selectedCronJob();
    if (!job) return;

    const name = this.editName().trim();
    const schedule = this.editSchedule().trim();
    const command = this.editCommand().trim();
    const status = this.editStatus();
    const isAppTarget = (job.targetType || job.target_type || 'app') === 'app';
    const appId = this.editAppId();

    if (!name || !schedule || !command || (isAppTarget && !appId)) {
      this.toast.error('All fields are required.');
      return;
    }

    this.savingSettings.set(true);
    this.projectService.updateCronJob(job.id, {
      name,
      schedule,
      command,
      appId: isAppTarget ? appId : undefined,
      status
    }).subscribe({
      next: (updated) => {
        this.toast.success('Cron job settings saved successfully.');
        this.selectedCronJob.set(updated);
        this.savingSettings.set(false);
        this.loadCronJobs();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to save settings.');
        this.savingSettings.set(false);
      }
    });
  }

  getDurationSec(startedAt: string | undefined, finishedAt: string | undefined): string {
    if (!startedAt || !finishedAt) return '0.0s';
    const start = new Date(startedAt).getTime();
    const end = new Date(finishedAt).getTime();
    if (isNaN(start) || isNaN(end)) return '0.0s';
    const diff = Math.max(0, (end - start) / 1000);
    return diff.toFixed(1) + 's';
  }

  onViewCronLogs(job: CronJob): void {
    this.selectedCronJob.set(job);
    this.currentPage.set(1);
    this.showCronLogsModal.set(true);
    this.loadCronLogs(1, this.pageSize());
  }

  private setupWsSubscriptions(): void {
    this.wsSubscriptions.unsubscribe();
    this.wsSubscriptions = new Subscription();

    // 1. Cron Job Updated
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('cron_job_updated').subscribe(payload => {
        const job: CronJob = payload.job;
        const currentProjId = this.parent.projectId();
        
        if (job && job.project_id === currentProjId) {
          
          this.cronJobs.update(jobs => {
            const idx = jobs.findIndex(j => j.id === job.id);
            if (idx !== -1) {
              const updated = [...jobs];
              updated[idx] = job;
              return updated;
            } else {
              return [job, ...jobs];
            }
          });

          const selected = this.selectedCronJob();
          if (selected && selected.id === job.id) {
            this.selectedCronJob.set(job);
            
            if (this.activeTab() !== 'settings') {
              this.editName.set(job.name);
              this.editSchedule.set(job.schedule);
              this.editCommand.set(job.command);
              this.editAppId.set(job.appId || job.app_id || '');
              this.editStatus.set((job.status?.toLowerCase() as any) || 'active');
            }
          }
        }
      })
    );

    // 2. Cron Job Deleted
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('cron_job_deleted').subscribe(payload => {
        const jobId = payload.job_id;
        
        this.cronJobs.update(jobs => jobs.filter(j => j.id !== jobId));

        if (this.selectedCronJob()?.id === jobId) {
          this.deselectCronJob();
          this.toast.info('This cron job was deleted.');
        }
      })
    );

    // 3. Cron Job Log Created
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('cron_job_log_created').subscribe(payload => {
        const jobId = payload.job_id;
        const log: CronJobLog = payload.log;

        
        const selected = this.selectedCronJob();
        if (selected && selected.id === jobId) {
          // Prepend new log and keep reactive
          this.cronLogs.update(logs => [log, ...logs]);
          this.totalLogs.update(t => t + 1);
          
          // Reload the current page silently to keep proper pagination order
          this.loadCronLogs(this.currentPage(), this.pageSize(), true);
        }
      })
    );
  }
}
