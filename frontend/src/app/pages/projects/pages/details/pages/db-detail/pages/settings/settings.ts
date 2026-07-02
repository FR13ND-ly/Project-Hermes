import { Component, inject, signal, effect } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { DbDetailComponent } from '../../db-detail';
import { DatabaseService, BackupCron } from '../../../../../../../../core/services/database.service';
import { ToastService } from '../../../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-db-settings',
  imports: [FormsModule, DatePipe],
  templateUrl: './settings.html',
  styles: ``,
})
export class DbSettingsComponent {
  readonly dbDetail = inject(DbDetailComponent);
  private readonly dbService = inject(DatabaseService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly router = inject(Router);

  // Edit settings signals
  readonly cpuLimit = signal(0); // mCPU — 0 = unlimited
  readonly memLimit = signal(0); // MB — 0 = unlimited
  readonly backupEnabled = signal(false);
  readonly backupCount = signal(7);
  readonly savingSettings = signal(false);
  readonly saveSettingsSuccess = signal(false);

  // Auto-backup as a real cron, managed from settings
  readonly backupCron = signal<BackupCron | null>(null);
  readonly showAddBackup = signal(false);
  readonly newBackupCount = signal(7);
  readonly savingBackup = signal(false);

  private initialized = false;

  constructor() {
    effect(() => {
      // Reset initialization when dbId changes
      const id = this.dbDetail.dbId();
      if (id) {
        this.initialized = false;
      }
    });

    effect(() => {
      const res = this.dbDetail.db();
      if (res && !this.initialized) {
        this.cpuLimit.set(res.cpuLimit ?? 0);
        this.memLimit.set(res.memoryLimitMb ?? 0);
        this.backupEnabled.set(res.backupEnabled || false);
        this.backupCount.set(res.backupCount || 7);
        this.loadBackupCron(res.id);
        this.initialized = true;
      }
    });
  }

  // Save Settings
  onSaveSettings(): void {
    const id = this.dbDetail.dbId();
    if (!id) return;

    this.savingSettings.set(true);
    this.saveSettingsSuccess.set(false);

    this.dbService.updateSettings(id, {
      cpuLimit: this.cpuLimit(),
      memoryLimitMb: this.memLimit(),
      backupEnabled: this.backupEnabled(),
      backupCount: this.backupCount()
    }).subscribe({
      next: () => {
        this.savingSettings.set(false);
        this.saveSettingsSuccess.set(true);
        this.toast.success('Resource limits saved. The pod will redeploy automatically!');
        this.dbDetail.loadDatabaseDetails(id, true);
        setTimeout(() => this.saveSettingsSuccess.set(false), 3000);
      },
      error: (err: any) => {
        this.savingSettings.set(false);
        this.toast.error(err.error?.message || 'Failed to save settings.');
      }
    });
  }

  // --- Auto-backup (managed cron) ---
  loadBackupCron(dbId: string): void {
    this.dbService.getBackupCron(dbId).subscribe({
      next: (res) => this.backupCron.set(res),
      error: () => this.backupCron.set(null)
    });
  }

  openAddBackup(): void {
    this.newBackupCount.set(this.backupCount() || 7);
    this.showAddBackup.set(true);
  }

  // Persist backup settings (cpu/mem kept as-is) and refresh the linked cron
  private persistBackup(enabled: boolean, count: number, okMsg: string): void {
    const id = this.dbDetail.dbId();
    if (!id) return;
    this.savingBackup.set(true);
    this.dbService.updateSettings(id, {
      cpuLimit: this.cpuLimit(),
      memoryLimitMb: this.memLimit(),
      backupEnabled: enabled,
      backupCount: count
    }).subscribe({
      next: () => {
        this.savingBackup.set(false);
        this.showAddBackup.set(false);
        this.backupEnabled.set(enabled);
        this.backupCount.set(count);
        this.toast.success(okMsg);
        this.loadBackupCron(id);
      },
      error: (err: any) => {
        this.savingBackup.set(false);
        this.toast.error(err.error?.message || 'Failed to update auto-backup.');
      }
    });
  }

  onEnableBackup(): void {
    const count = Math.max(1, Math.min(30, this.newBackupCount() || 7));
    this.persistBackup(true, count, 'Auto-backup enabled — an editable cron job has been created.');
  }

  onUpdateBackupRetention(): void {
    const count = Math.max(1, Math.min(30, this.backupCount() || 7));
    this.persistBackup(true, count, 'Backup retention count updated.');
  }

  async onDisableBackup(): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Disable Auto-Backup',
      message: 'Are you sure you want to disable auto-backup? The associated cron job will be deleted. Already created backups will remain.',
      confirmText: 'Disable',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;
    this.persistBackup(false, this.backupCount() || 7, 'Auto-backup disabled.');
  }

  goToBackupCron(): void {
    this.router.navigate(['/projects', this.dbDetail.parent.projectId(), 'cron']);
  }

  async onDeleteDatabase(): Promise<void> {
    const id = this.dbDetail.dbId();
    const dbData = this.dbDetail.db();
    if (!id || !dbData) return;

    const confirmed = await this.confirm.ask({
      title: 'Delete Database',
      message: `Are you sure you want to delete the database "${dbData.name}"? All stored data will be permanently destroyed!`,
      confirmText: 'Delete permanently',
      cancelText: 'Cancel',
      isDanger: true,
      matchText: dbData.name
    });
    if (!confirmed) return;

    this.dbService.deleteDatabase(id).subscribe({
      next: () => {
        this.toast.success('Database deleted.');
        this.router.navigate(['/projects', this.dbDetail.parent.projectId(), 'databases']);
      },
      error: (err: any) => {
        this.toast.error(err.error?.message || 'Failed to delete database.');
      }
    });
  }
}
