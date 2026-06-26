import { Component, inject, signal } from '@angular/core';
import { NgClass } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService } from '../../../../../../core/services/database.service';
import { ToastService } from '../../../../../../core/services/toast.service';

@Component({
  selector: 'app-db-create',
  imports: [NgClass, FormsModule],
  templateUrl: './db-create.html',
  styleUrl: './db-create.css',
})
export class DbCreate {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);

  readonly provisioning = signal(false);
  readonly error = signal<string | null>(null);

  readonly dbName = signal('');
  readonly dbType = signal<'postgres' | 'redis' | 'mongodb'>('postgres');
  readonly cpuLimit = signal(0); // Millicores
  readonly memLimit = signal(0); // Megabytes
  readonly storageGb = signal(1); // GB
  readonly isExternal = signal(false);
  readonly externalPort = signal(5432);

  readonly publishToEnv = signal(false);
  readonly envKeyName = signal('');

  onTypeChange(type: 'postgres' | 'redis' | 'mongodb'): void {
    this.dbType.set(type);
    if (type === 'postgres') this.externalPort.set(5432);
    else if (type === 'redis') this.externalPort.set(6379);
    else if (type === 'mongodb') this.externalPort.set(27017);
  }

  onCreateDatabase(): void {
    const projectId = this.parent.projectId();
    if (!projectId || !this.dbName()) return;

    this.provisioning.set(true);
    this.error.set(null);

    this.dbService.createDatabase({
      projectId,
      name: this.dbName(),
      type: this.dbType(),
      cpuLimit: this.cpuLimit(),
      memoryLimitMb: this.memLimit(),
      storageSizeGb: this.storageGb(),
      isExternal: this.isExternal(),
      externalPort: this.isExternal() ? this.externalPort() : undefined,
      publishToEnv: this.publishToEnv(),
      envKey: this.publishToEnv() && this.envKeyName().trim() ? this.envKeyName().trim() : undefined
    }).subscribe({
      next: () => {
        this.provisioning.set(false);
        this.toast.success('Database created successfully!');
        this.router.navigate(['/projects', projectId, 'databases']);
      },
      error: (err) => {
        const msg = err.error?.error?.message || err.error?.message || 'Failed to create database.';
        this.error.set(msg);
        this.toast.error(msg);
        this.provisioning.set(false);
      }
    });
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'databases']);
  }
}
