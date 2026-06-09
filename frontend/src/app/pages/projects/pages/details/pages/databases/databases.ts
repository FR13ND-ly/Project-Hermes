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

@Component({
  selector: 'app-databases',
  imports: [NgClass, FormsModule, RouterLink],
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

  // Form states
  readonly showCreateForm = signal(false);
  readonly provisioning = signal(false);
  
  readonly dbName = signal('');
  readonly dbType = signal<'postgres' | 'redis' | 'mysql' | 'mongodb'>('postgres');
  readonly cpuLimit = signal(250); // Millicores
  readonly memLimit = signal(512); // Megabytes
  readonly isExternal = signal(false);
  readonly externalPort = signal(5432);

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
      console.log('[Databases] Database status changed in WS, reloading list silently:', payload);
      this.loadDatabases(true);
    });
  }

  startTicker(): void {
    if (this.tickerInterval) return;
    this.tickerInterval = setInterval(() => {
      this.timeTicker.set(Date.now());
    }, 1000);
  }

  stopTicker(): void {
    if (this.tickerInterval) {
      clearInterval(this.tickerInterval);
      this.tickerInterval = null;
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

    this.dbService.listDatabases(projectId).subscribe({
      next: (res) => {
        this.databases.set(res || []);
        this.loading.set(false);
        const hasProvisioning = (res || []).some(db => db.status === 'provisioning');
        if (hasProvisioning) {
          this.startTicker();
        } else {
          this.stopTicker();
        }
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea bazelor de date.');
        this.loading.set(false);
        this.stopTicker();
      }
    });
  }

  onTypeChange(type: 'postgres' | 'redis' | 'mysql' | 'mongodb'): void {
    this.dbType.set(type);
    const ports = {
      postgres: 5432,
      redis: 6379,
      mysql: 3306,
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
      isExternal: this.isExternal(),
      externalPort: this.isExternal() ? this.externalPort() : undefined
    }).subscribe({
      next: () => {
        this.dbName.set('');
        this.showCreateForm.set(false);
        this.provisioning.set(false);
        this.toast.success('Baza de date a fost adăugată cu succes!');
        this.loadDatabases();
      },
      error: (err) => {
        const msg = err.error?.error?.message || err.error?.message || 'Eroare la crearea bazei de date.';
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
        this.toast.error(err.error?.message || 'Nu aveți permisiunea de a decripta credențialele.');
      }
    });
  }

  async onDeleteDatabase(dbId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Bază de Date',
      message: 'Sigur doriți să ștergeți această bază de date? Datele stocate vor fi șterse definitiv!',
      confirmText: 'Șterge definitiv',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.dbService.deleteDatabase(dbId).subscribe({
      next: () => {
        this.toast.success('Baza de date a fost ștearsă.');
        this.loadDatabases();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea bazei de date.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }
}
