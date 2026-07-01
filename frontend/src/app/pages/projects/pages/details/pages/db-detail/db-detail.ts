import { Component, inject, signal, computed, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { ActivatedRoute, Router, RouterLink, RouterOutlet, RouterLinkActive, NavigationEnd } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-db-detail',
  standalone: true,
  imports: [CommonModule, RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './db-detail.html',
  styleUrl: './db-detail.css',
})
export class DbDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly dbService = inject(DatabaseService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);

  readonly dbId = signal<string | null>(null);
  readonly db = signal<DatabaseServiceInfo | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Two-tier tab nav: groups the 6 flat routes by purpose (URLs unchanged).
  readonly tabGroups: { id: string; label: string; default: string; tabs: { path: string; label: string }[] }[] = [
    { id: 'overview', label: 'Overview', default: 'overview', tabs: [{ path: 'overview', label: 'Overview' }] },
    { id: 'console', label: 'Console', default: 'console', tabs: [{ path: 'console', label: 'Query Console' }] },
    { id: 'observability', label: 'Observability', default: 'telemetry', tabs: [
      { path: 'telemetry', label: 'Metrics' },
      { path: 'logs', label: 'Logs' },
    ] },
    { id: 'backups', label: 'Backups', default: 'backups', tabs: [{ path: 'backups', label: 'Backups' }] },
    { id: 'settings', label: 'Settings', default: 'settings', tabs: [{ path: 'settings', label: 'Settings' }] },
  ];
  readonly currentTabPath = signal<string>('overview');
  readonly activeGroup = computed(
    () => this.tabGroups.find((g) => g.tabs.some((t) => t.path === this.currentTabPath())) ?? this.tabGroups[0],
  );
  private navSub?: Subscription;
  private leafTabPath(): string {
    const clean = this.router.url.split('?')[0].split('#')[0];
    return clean.split('/').filter(Boolean).pop() ?? 'overview';
  }

  // Credentials reveal state
  readonly connectionUrl = signal<string | null>(null);
  readonly databaseUser = signal<string | null>(null);
  readonly databasePassword = signal<string | null>(null);
  readonly credentialsRevealed = signal(false);
  readonly rotatingPassword = signal(false);

  private wsSubscription: Subscription | null = null;
  readonly timeTicker = signal<number>(Date.now());
  private tickerInterval: any = null;

  constructor() {
    effect(() => {
      const id = this.dbId();
      if (id) {
        this.loadDatabaseDetails(id);
        this.setupWsSubscription(id);
      }
    });
  }

  ngOnInit(): void {
    const id = this.route.snapshot.paramMap.get('dbId');
    if (id) {
      this.dbId.set(id);
    }
    this.currentTabPath.set(this.leafTabPath());
    this.navSub = this.router.events.subscribe((e) => {
      if (e instanceof NavigationEnd) this.currentTabPath.set(this.leafTabPath());
    });
  }

  ngOnDestroy(): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
    this.navSub?.unsubscribe();
    this.stopTicker();
  }

  setupWsSubscription(id: string): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
    this.wsSubscription = this.wsService.onEvent<any>('database_status_changed').subscribe(payload => {
      if (payload.database_id === id) {
        this.loadDatabaseDetails(id, true);
      }
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

  loadDatabaseDetails(id: string, silent = false): void {
    if (!silent) this.loading.set(true);
    this.dbService.getDatabase(id).subscribe({
      next: (res) => {
        this.db.set(res);
        this.loading.set(false);
        
        // Start ticker if database is still provisioning
        if (res.status === 'provisioning') {
          this.startTicker();
        } else {
          this.stopTicker();
        }
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Failed to load database details.');
        this.loading.set(false);
        this.stopTicker();
      }
    });
  }

  onRevealCredentials(): void {
    const id = this.dbId();
    if (!id) return;

    if (this.credentialsRevealed()) {
      this.credentialsRevealed.set(false);
      this.connectionUrl.set(null);
      this.databaseUser.set(null);
      this.databasePassword.set(null);
      return;
    }

    this.dbService.revealCredentials(id).subscribe({
      next: (res) => {
        this.connectionUrl.set(res.connectionUrl);
        this.databaseUser.set(res.databaseUser || null);
        this.databasePassword.set(res.databasePassword || null);
        this.credentialsRevealed.set(true);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'You do not have permission to decrypt the credentials.');
      }
    });
  }

  async rotatePassword(): Promise<void> {
    const id = this.dbId();
    if (!id) return;

    const confirmed = await this.confirm.ask({
      title: 'Rotate Database Password',
      message: 'A new password will be generated directly in the DB engine, and connected applications will be restarted automatically to reconnect (brief downtime). Continue?',
      confirmText: 'Rotate',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.rotatingPassword.set(true);
    this.dbService.rotatePassword(id).subscribe({
      next: () => {
        this.rotatingPassword.set(false);
        this.toast.success('Password rotated. Connected applications are restarting automatically.');
        // Any revealed credentials are now stale — hide them.
        this.credentialsRevealed.set(false);
        this.connectionUrl.set(null);
        this.databasePassword.set(null);
      },
      error: (err) => {
        this.rotatingPassword.set(false);
        this.toast.error(err.error?.message || 'Failed to rotate password.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copied to clipboard!');
    });
  }
}
