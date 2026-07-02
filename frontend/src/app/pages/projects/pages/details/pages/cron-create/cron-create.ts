import { Component, inject, signal, computed, OnInit } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, EnvVarInput, ProjectEnvResponse } from '../../../../../../core/services/project.service';
import { DatabaseService } from '../../../../../../core/services/database.service';
import { StorageService } from '../../../../../../core/services/storage.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { EnvLinkModal } from '../../../../../../shared/components/env-link-modal/env-link-modal';

type CronTargetType = 'app' | 'database' | 'storage';

@Component({
  selector: 'app-cron-create',
  imports: [FormsModule, EnvLinkModal],
  templateUrl: './cron-create.html',
  styleUrl: './cron-create.css',
})
export class CronCreate implements OnInit {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly dbService = inject(DatabaseService);
  private readonly storageService = inject(StorageService);
  private readonly toast = inject(ToastService);
  private readonly router = inject(Router);

  readonly creatingCron = signal(false);

  // Form Fields
  readonly newCronName = signal('');
  readonly newCronSchedule = signal('*/5 * * * *');
  readonly newCronCommand = signal('echo "Hello World"');

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

  // --- Env for the cron run: custom rows + project-pool links (mirrors app creation) ---
  readonly newCronEnvRows = signal<EnvVarInput[]>([]);
  readonly projectEnvPool = signal<ProjectEnvResponse[]>([]);
  readonly selectedProjectEnvIds = signal<string[]>([]);
  readonly showCreateEnvModal = signal(false);

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

  ngOnInit(): void {
    const projId = this.parent.projectId();
    if (projId) {
      this.dbService.listDatabases(projId, 1, 1000).subscribe({
        next: (res) => {
          this.databases.set((res?.items || []).map(d => ({ id: d.id, name: d.name })));
          if (this.newCronTargetType() === 'database') {
            const first = this.databases()[0];
            if (first) this.selectedTargetId.set(first.id);
          }
        },
        error: () => this.databases.set([])
      });
      this.storageService.listBuckets().subscribe({
        next: (res) => {
          this.storages.set((res || []).map(b => ({ id: b.id, name: b.name })));
          if (this.newCronTargetType() === 'storage') {
            const first = this.storages()[0];
            if (first) this.selectedTargetId.set(first.id);
          }
        },
        error: () => this.storages.set([])
      });
    }
    const apps = this.parent.apps();
    if (this.newCronTargetType() === 'app' && apps.length > 0) {
      this.selectedTargetId.set(apps[0].id);
    }
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
        this.toast.success('Cron job created successfully!');
        this.router.navigate(['/projects', projId, 'cron']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to create cron job.');
        this.creatingCron.set(false);
      }
    });
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'cron']);
  }
}
