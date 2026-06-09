import { Component, inject, signal, OnInit, OnDestroy, effect, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { ProjectService, CronJob, CronJobLog } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-project-cron',
  standalone: true,
  imports: [CommonModule, FormsModule, DatePipe],
  templateUrl: './cron.html',
})
export class CronComponent implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);

  readonly cronJobs = signal<CronJob[]>([]);
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

    this.projectService.listProjectCronJobs(projId).subscribe({
      next: (res) => {
        this.cronJobs.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea joburilor cron.');
        this.loading.set(false);
      }
    });
  }

  // Helper to find the app name by app_id
  getAppName(appId: string | undefined): string {
    if (!appId) return 'Nespecificată';
    const app = this.parent.apps().find(a => a.id === appId);
    return app ? app.name : appId.substring(0, 8);
  }

  onOpenCreateModal(): void {
    const apps = this.parent.apps();
    if (apps.length === 0) {
      this.toast.error('Creați mai întâi o aplicație în acest proiect pentru a putea asocia un job cron.');
      return;
    }
    this.selectedAppId.set(apps[0].id);
    this.newCronName.set('');
    this.newCronSchedule.set('*/5 * * * *');
    this.newCronCommand.set('echo "Hello World"');
    this.showCreateModal.set(true);
  }

  onCreateCronJob(): void {
    const projId = this.parent.projectId();
    const appId = this.selectedAppId();
    if (!projId || !appId) return;

    const name = this.newCronName().trim();
    const schedule = this.newCronSchedule().trim();
    const command = this.newCronCommand().trim();

    if (!name || !schedule || !command) {
      this.toast.error('Toate câmpurile sunt obligatorii.');
      return;
    }

    this.creatingCron.set(true);
    this.projectService.createCronJob({
      projectId: projId,
      appId,
      name,
      schedule,
      command
    }).subscribe({
      next: () => {
        this.creatingCron.set(false);
        this.showCreateModal.set(false);
        this.toast.success('Jobul cron a fost creat cu succes!');
        this.loadCronJobs();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea jobului cron.');
        this.creatingCron.set(false);
      }
    });
  }

  async onDeleteCronJob(jobId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Job Cron',
      message: 'Sigur doriți să ștergeți acest job cron?',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteCronJob(jobId).subscribe({
      next: () => {
        this.toast.success('Jobul cron a fost șters.');
        if (this.selectedCronJob()?.id === jobId) {
          this.deselectCronJob();
        }
        this.loadCronJobs();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea jobului cron.');
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
        this.toast.error(err.error?.message || 'Eroare la încărcarea logurilor.');
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
    const appId = this.editAppId();
    const status = this.editStatus();

    if (!name || !schedule || !command || !appId) {
      this.toast.error('Toate câmpurile sunt obligatorii.');
      return;
    }

    this.savingSettings.set(true);
    this.projectService.updateCronJob(job.id, {
      name,
      schedule,
      command,
      appId,
      status
    }).subscribe({
      next: (updated) => {
        this.toast.success('Setările jobului cron au fost salvate cu succes.');
        this.selectedCronJob.set(updated);
        this.savingSettings.set(false);
        this.loadCronJobs();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea setărilor.');
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
          console.log('[Cron] Job updated in WS:', job);
          
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
        console.log('[Cron] Job deleted in WS:', jobId);
        
        this.cronJobs.update(jobs => jobs.filter(j => j.id !== jobId));

        if (this.selectedCronJob()?.id === jobId) {
          this.deselectCronJob();
          this.toast.info('Acest job cron a fost șters.');
        }
      })
    );

    // 3. Cron Job Log Created
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('cron_job_log_created').subscribe(payload => {
        const jobId = payload.job_id;
        const log: CronJobLog = payload.log;

        console.log('[Cron] Log created in WS for job:', jobId, log);
        
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
