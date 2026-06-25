import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { NgClass } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

import { RouterLink } from '@angular/router';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

@Component({
  selector: 'app-databases',
  imports: [NgClass, FormsModule, RouterLink, Pagination],
  templateUrl: './databases.html',
  styleUrl: './databases.css',
})
export class Databases implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  readonly page = signal(1);
  readonly pageSize = signal(DEFAULT_PAGE_SIZE);
  readonly total = signal(0);

  // Form states
  readonly showCreateForm = signal(false);
  readonly provisioning = signal(false);
  
  readonly dbName = signal('');
  readonly dbType = signal<'postgres' | 'redis' | 'mongodb'>('postgres');
  readonly cpuLimit = signal(0); // Millicores
  readonly memLimit = signal(0); // Megabytes
  readonly storageGb = signal(1); // GB (PVC size, set at creation)
  readonly isExternal = signal(false);
  readonly externalPort = signal(5432);

  // Publish the connection string into the project env pool (with optional custom key).
  readonly publishToEnv = signal(false);
  readonly envKeyName = signal('');

  // Map of databaseId -> revealed credentials
  readonly revealedCreds = signal<Record<string, { connectionUrl: string; databaseUser?: string; databasePassword?: string }>>({});

  private wsSubscription: Subscription | null = null;

  constructor() {
    effect(() => {
      const appId = this.parent.projectId();
      if (appId) {
        this.loadDatabases();
        this.setupWsSubscription();
      }
    });
  }

  readonly timeTicker = signal<number>(Date.now());
  private tickerInterval: any = null;
  private pollInterval: any = null;

  ngOnInit(): void {
    this.loadDatabases();
    this.setupWsSubscription();
  }

  ngOnDestroy(): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
    this.stopTicker();
  }

  setupWsSubscription(): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
    this.wsSubscription = this.wsService.onEvent<any>('database_status_changed').subscribe(payload => {
      this.loadDatabases(true);
    });
  }

  startTicker(): void {
    if (!this.tickerInterval) {
      this.tickerInterval = setInterval(() => {
        this.timeTicker.set(Date.now());
      }, 1000);
    }
    // `database_status_changed` (WS, see setupWsSubscription) drives instant
    // updates; this poll is only a safety-net while a DB is still provisioning,
    // so a slower 15s tick is enough.
    if (!this.pollInterval) {
      this.pollInterval = setInterval(() => {
        this.loadDatabases(true);
      }, 15000);
    }
  }

  stopTicker(): void {
    if (this.tickerInterval) {
      clearInterval(this.tickerInterval);
      this.tickerInterval = null;
    }
    if (this.pollInterval) {
      clearInterval(this.pollInterval);
      this.pollInterval = null;
    }
  }

  getLiveDuration(createdAt: string): string {
    const elapsed = Math.floor((this.timeTicker() - new Date(createdAt).getTime()) / 1000);
    if (elapsed < 0) return '0s';
    if (elapsed < 60) return `${elapsed}s`;
    const mins = Math.floor(elapsed / 60);
    const secs = elapsed % 60;
    return `${mins}m ${secs}s`;
  }

  loadDatabases(silent = false): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    if (!silent) this.loading.set(true);
    this.error.set(null);

    this.dbService.listDatabases(projectId, this.page(), this.pageSize()).subscribe({
      next: (res) => {
        const items = res?.items || [];
        this.databases.set(items);
        this.total.set(res?.total || 0);
        this.loading.set(false);
        const hasProvisioning = items.some(db => db.status === 'provisioning');
        if (hasProvisioning) {
          this.startTicker();
        } else {
          this.stopTicker();
        }
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Failed to load databases.');
        this.loading.set(false);
        this.stopTicker();
      }
    });
  }

  onPageChange(page: number): void {
    this.page.set(page);
    this.loadDatabases();
  }

  onTypeChange(type: 'postgres' | 'redis' | 'mongodb'): void {
    this.dbType.set(type);
    const ports = {
      postgres: 5432,
      redis: 6379,
      mongodb: 27017
    };
    this.externalPort.set(ports[type]);
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
        this.dbName.set('');
        this.envKeyName.set('');
        this.publishToEnv.set(false);
        this.showCreateForm.set(false);
        this.provisioning.set(false);
        this.toast.success('Database created successfully!');
        this.loadDatabases();
      },
      error: (err) => {
        const msg = err.error?.error?.message || err.error?.message || 'Failed to create database.';
        this.error.set(msg);
        this.toast.error(msg);
        this.provisioning.set(false);
      }
    });
  }

  onRevealCredentials(dbId: string): void {
    // If already revealed, toggle off (hide)
    if (this.revealedCreds()[dbId]) {
      const updated = { ...this.revealedCreds() };
      delete updated[dbId];
      this.revealedCreds.set(updated);
      return;
    }

    this.dbService.revealCredentials(dbId).subscribe({
      next: (res: any) => {
        this.revealedCreds.update(creds => ({
          ...creds,
          [dbId]: {
            connectionUrl: res.connectionUrl,
            databaseUser: res.databaseUser,
            databasePassword: res.databasePassword
          }
        }));
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'You do not have permission to decrypt the credentials.');
      }
    });
  }

  async onDeleteDatabase(dbId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Database',
      message: 'Are you sure you want to delete this database? All stored data will be permanently destroyed!',
      confirmText: 'Delete permanently',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.dbService.deleteDatabase(dbId).subscribe({
      next: () => {
        this.toast.success('Database deleted.');
        this.loadDatabases();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete database.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copied to clipboard!');
    });
  }
}
