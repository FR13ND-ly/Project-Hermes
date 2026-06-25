import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo, DbBackup, BackupCron } from '../../../../../../core/services/database.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-db-detail',
  standalone: true,
  imports: [CommonModule, DatePipe, FormsModule, RouterLink],
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

  // Active sub-tab state
  readonly activeSubTab = signal<'overview' | 'console' | 'logs' | 'backups' | 'settings' | 'telemetry'>('overview');

  // Credentials reveal state
  readonly connectionUrl = signal<string | null>(null);
  readonly databaseUser = signal<string | null>(null);
  readonly databasePassword = signal<string | null>(null);
  readonly credentialsRevealed = signal(false);
  readonly rotatingPassword = signal(false);

  // Console query signals
  readonly queryInput = signal('');
  readonly queryLoading = signal(false);
  readonly queryHistory = signal<{ query: string; output: string; isError: boolean; timestamp: Date }[]>([]);

  // Logs stream signals
  readonly logs = signal<string[]>([]);
  readonly sseConnected = signal(false);
  readonly autoScroll = signal(true);
  private eventSource: EventSource | null = null;

  // Edit settings signals
  readonly cpuLimit = signal(0); // mCPU — 0 = unlimited
  readonly memLimit = signal(0); // MB — 0 = unlimited
  readonly backupEnabled = signal(false);
  readonly backupCount = signal(7);
  readonly savingSettings = signal(false);
  readonly saveSettingsSuccess = signal(false);

  // Auto-backup as a real cron, managed from settings.
  readonly backupCron = signal<BackupCron | null>(null);
  readonly showAddBackup = signal(false);
  readonly newBackupCount = signal(7);
  readonly savingBackup = signal(false);

  // Backups state
  readonly backups = signal<DbBackup[]>([]);
  readonly loadingBackups = signal(false);

  private wsSubscription: Subscription | null = null;

  constructor() {
    effect(() => {
      const id = this.dbId();
      if (id) {
        this.loadDatabaseDetails(id);
        this.setupWsSubscription(id);
      }
    });

    effect(() => {
      const tab = this.activeSubTab();
      const id = this.dbId();
      if (id && tab === 'logs') {
        this.connectLogs(id);
      } else {
        this.disconnectLogs();
      }
    });

    effect(() => {
      const tab = this.activeSubTab();
      const id = this.dbId();
      if (id && tab === 'backups') {
        this.loadBackups(id);
      }
    });

    effect(() => {
      const tab = this.activeSubTab();
      const id = this.dbId();
      // Re-load whenever the telemetry tab is active or the range changes.
      const range = this.dbRange();
      if (id && tab === 'telemetry') {
        this.loadDbMetrics(id, range);
      }
    });
  }

  // --- Telemetry (database metrics) ---
  readonly dbRange = signal('1h');
  readonly dbMetricsLoading = signal(false);
  readonly dbMetricsSimulated = signal(false);
  readonly dbCpuValues = signal<number[]>([]);
  readonly dbMemValues = signal<number[]>([]);
  readonly dbSizeValues = signal<number[]>([]);
  readonly dbConnValues = signal<number[]>([]);
  readonly dbCacheValues = signal<number[]>([]);

  loadDbMetrics(id: string, range: string): void {
    this.dbMetricsLoading.set(true);

    this.dbService.getMetrics(id, 'cpu', range).subscribe({
      next: (res) => {
        this.dbCpuValues.set((res.values || []).map(v => v * 1000)); // cores -> millicores
        this.dbMetricsSimulated.set(!!res.simulated);
        this.dbMetricsLoading.set(false);
      },
      error: () => { this.dbCpuValues.set([]); this.dbMetricsLoading.set(false); }
    });

    this.dbService.getMetrics(id, 'memory', range).subscribe({
      next: (res) => this.dbMemValues.set(res.values || []),
      error: () => this.dbMemValues.set([])
    });

    this.dbService.getMetrics(id, 'db_size', range).subscribe({
      next: (res) => this.dbSizeValues.set(res.values || []),
      error: () => this.dbSizeValues.set([])
    });

    this.dbService.getMetrics(id, 'db_connections', range).subscribe({
      next: (res) => this.dbConnValues.set(res.values || []),
      error: () => this.dbConnValues.set([])
    });

    // Cache-hit rate is meaningful for engines that expose hit/miss counters
    // (postgres via pg_stat, redis via keyspace_hits/misses).
    const t = this.db()?.type;
    if (t === 'postgres' || t === 'redis') {
      this.dbService.getMetrics(id, 'db_cache_hit_rate', range).subscribe({
        next: (res) => this.dbCacheValues.set(res.values || []),
        error: () => this.dbCacheValues.set([])
      });
    } else {
      this.dbCacheValues.set([]);
    }
  }

  onDbRangeChange(range: string): void {
    this.dbRange.set(range);
  }

  cpuUsedPct(): number {
    const v = this.dbCpuValues();
    const cur = v.length > 0 ? v[v.length - 1] : 0;
    const limit = this.cpuLimit();
    return limit > 0 ? Math.min(100, Math.round((cur / limit) * 100)) : 0;
  }

  memUsedPct(): number {
    const v = this.dbMemValues();
    const cur = v.length > 0 ? v[v.length - 1] : 0;
    const limit = this.memLimit();
    return limit > 0 ? Math.min(100, Math.round((cur / limit) * 100)) : 0;
  }

  lastVal(values: number[]): number {
    return values.length > 0 ? values[values.length - 1] : 0;
  }

  getSvgPath(values: number[]): string {
    if (values.length < 2) return '';
    const width = 500;
    const height = 150;
    const max = Math.max(...values, 0.1) * 1.1;
    const min = Math.min(...values, 0);
    const span = (max - min) || 1;

    return values.map((val, idx) => {
      const x = (idx / (values.length - 1)) * width;
      const y = height - ((val - min) / span) * height;
      return `${idx === 0 ? 'M' : 'L'} ${x.toFixed(1)} ${y.toFixed(1)}`;
    }).join(' ');
  }

  getSvgFillPath(values: number[]): string {
    const linePath = this.getSvgPath(values);
    if (!linePath) return '';
    return `${linePath} L 500 150 L 0 150 Z`;
  }

  ngOnInit(): void {
    const id = this.route.snapshot.paramMap.get('dbId');
    if (id) {
      this.dbId.set(id);
    }

    // Set initial subtab from query parameters if present
    this.route.queryParams.subscribe(params => {
      if (params['tab']) {
        const tab = params['tab'];
        if (tab === 'overview' || tab === 'console' || tab === 'logs' || tab === 'backups' || tab === 'settings' || tab === 'telemetry') {
          this.activeSubTab.set(tab as any);
        }
      }
    });
  }

  readonly timeTicker = signal<number>(Date.now());
  private tickerInterval: any = null;

  ngOnDestroy(): void {
    if (this.wsSubscription) {
      this.wsSubscription.unsubscribe();
    }
    this.stopTicker();
    this.disconnectLogs();
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
        // Preserve 0 (= unlimited); only fall back when the value is missing.
        this.cpuLimit.set(res.cpuLimit ?? 0);
        this.memLimit.set(res.memoryLimitMb ?? 0);
        this.backupEnabled.set(res.backupEnabled || false);
        this.backupCount.set(res.backupCount || 7);
        this.loadBackupCron(id);
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

  switchTab(tab: 'overview' | 'console' | 'logs' | 'backups' | 'settings' | 'telemetry'): void {
    this.activeSubTab.set(tab);
    this.router.navigate([], {
      relativeTo: this.route,
      queryParams: { tab },
      queryParamsHandling: 'merge'
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

  onRunQuery(): void {
    const id = this.dbId();
    const query = this.queryInput().trim();
    if (!id || !query) return;

    this.queryLoading.set(true);
    this.dbService.runQuery(id, query).subscribe({
      next: (res) => {
        this.queryHistory.update(history => [
          ...history,
          {
            query,
            output: res.output,
            isError: res.isError,
            timestamp: new Date()
          }
        ]);
        this.queryInput.set('');
        this.queryLoading.set(false);
        setTimeout(() => this.scrollConsoleToBottom(), 50);
      },
      error: (err) => {
        this.queryHistory.update(history => [
          ...history,
          {
            query,
            output: err.error?.message || 'Failed to communicate with the database.',
            isError: true,
            timestamp: new Date()
          }
        ]);
        this.queryLoading.set(false);
        setTimeout(() => this.scrollConsoleToBottom(), 50);
      }
    });
  }

  clearConsole(): void {
    this.queryHistory.set([]);
  }

  scrollConsoleToBottom(): void {
    const el = document.getElementById('query-terminal-window');
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }

  // Stdout container logs SSE
  connectLogs(id: string): void {
    this.disconnectLogs();
    this.logs.set(['[Console] Connecting to Kubernetes log stream...']);

    const streamUrl = this.dbService.getLogsStreamUrl(id);
    this.eventSource = new EventSource(streamUrl);

    this.eventSource.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] Connection established. Reading logs from pod:']);
    };

    this.eventSource.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => [...lines, event.data]);
        if (this.autoScroll()) {
          this.scrollLogsToBottom();
        }
      }
    };

    this.eventSource.onerror = () => {
      this.sseConnected.set(false);
      this.logs.update(lines => [...lines, '[Notice] Connection interrupted. Attempting to reconnect...']);
      this.disconnectLogs();
    };
  }

  disconnectLogs(): void {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
    this.sseConnected.set(false);
  }

  scrollLogsToBottom(): void {
    const el = document.getElementById('db-logs-window');
    if (el) {
      el.scrollTop = el.scrollHeight;
    }
  }

  toggleAutoScroll(): void {
    this.autoScroll.update(val => !val);
    if (this.autoScroll()) {
      this.scrollLogsToBottom();
    }
  }

  // Save Settings
  onSaveSettings(): void {
    const id = this.dbId();
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
        this.loadDatabaseDetails(id, true);
        setTimeout(() => this.saveSettingsSuccess.set(false), 3000);
      },
      error: (err) => {
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

  // Persist backup settings (cpu/mem kept as-is) and refresh the linked cron.
  private persistBackup(enabled: boolean, count: number, okMsg: string): void {
    const id = this.dbId();
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
      error: (err) => {
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
    this.router.navigate(['/projects', this.parent.projectId(), 'cron']);
  }

  // Backups operations
  loadBackups(dbId: string): void {
    this.loadingBackups.set(true);
    this.dbService.listBackups(dbId).subscribe({
      next: (res) => {
        this.backups.set(res);
        this.loadingBackups.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to load backups.');
        this.loadingBackups.set(false);
      }
    });
  }

  onCreateBackup(): void {
    const id = this.dbId();
    if (!id) return;

    this.toast.info('Initializing backup creation...');
    this.dbService.createBackup(id).subscribe({
      next: () => {
        this.toast.success('Backup created successfully.');
        this.loadBackups(id);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to create backup.');
      }
    });
  }

  async onRestoreBackup(backup: DbBackup): Promise<void> {
    const id = this.dbId();
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
        this.loadDatabaseDetails(id, true);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to restore database.');
      }
    });
  }

  async onDeleteBackup(backup: DbBackup): Promise<void> {
    const id = this.dbId();
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
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete backup.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copied to clipboard!');
    });
  }

  async onDeleteDatabase(): Promise<void> {
    const id = this.dbId();
    const dbData = this.db();
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
        this.router.navigate(['/projects', this.parent.projectId(), 'databases']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete database.');
      }
    });
  }
}
