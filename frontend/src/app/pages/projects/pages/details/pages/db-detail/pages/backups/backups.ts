import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { DatePipe, DecimalPipe } from '@angular/common';
import { DbDetailComponent } from '../../db-detail';
import { DatabaseService, DbBackup } from '../../../../../../../../core/services/database.service';
import { ToastService } from '../../../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-db-backups',
  imports: [DatePipe, DecimalPipe],
  templateUrl: './backups.html',
  styles: ``,
})
export class DbBackupsComponent implements OnInit {
  readonly dbDetail = inject(DbDetailComponent);
  private readonly dbService = inject(DatabaseService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly backups = signal<DbBackup[]>([]);
  readonly loadingBackups = signal(false);

  constructor() {
    effect(() => {
      const id = this.dbDetail.dbId();
      if (id) {
        this.loadBackups(id);
      }
    });
  }

  ngOnInit(): void {
    // Initial fetch managed by effect when dbId is resolved
  }

  loadBackups(dbId: string): void {
    this.loadingBackups.set(true);
    this.dbService.listBackups(dbId).subscribe({
      next: (res) => {
        this.backups.set(res);
        this.loadingBackups.set(false);
      },
      error: (err: any) => {
        this.toast.error(err.error?.message || 'Failed to load backups.');
        this.loadingBackups.set(false);
      }
    });
  }

  onCreateBackup(): void {
    const id = this.dbDetail.dbId();
    if (!id) return;

    this.toast.info('Initializing backup creation...');
    this.dbService.createBackup(id).subscribe({
      next: () => {
        this.toast.success('Backup created successfully.');
        this.loadBackups(id);
      },
      error: (err: any) => {
        this.toast.error(err.error?.message || 'Failed to create backup.');
      }
    });
  }

  async onRestoreBackup(backup: DbBackup): Promise<void> {
    const id = this.dbDetail.dbId();
    if (!id) return;

    const confirmed = await this.confirm.ask({
      title: 'Restore Database',
      message: `Are you sure you want to restore the database from the backup created at ${new Date(backup.createdAt).toLocaleString()}? Current data will be overwritten!`,
      confirmText: 'Restore',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.toast.info('Restoring database...');
    this.dbService.restoreBackup(id, backup.id).subscribe({
      next: () => {
        this.toast.success('Database restored successfully.');
        this.dbDetail.loadDatabaseDetails(id, true);
      },
      error: (err: any) => {
        this.toast.error(err.error?.message || 'Failed to restore database.');
      }
    });
  }

  async onDeleteBackup(backup: DbBackup): Promise<void> {
    const id = this.dbDetail.dbId();
    if (!id) return;

    const confirmed = await this.confirm.ask({
      title: 'Delete Backup',
      message: `Are you sure you want to delete the backup "${backup.filename}"? This action is irreversible!`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.dbService.deleteBackup(id, backup.id).subscribe({
      next: () => {
        this.toast.success('Backup deleted.');
        this.loadBackups(id);
      },
      error: (err: any) => {
        this.toast.error(err.error?.message || 'Failed to delete backup.');
      }
    });
  }
}
