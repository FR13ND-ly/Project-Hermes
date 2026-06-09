import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo, DbBackup } from '../../../../../../core/services/database.service';
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
  readonly activeSubTab = signal<'overview' | 'console' | 'logs' | 'backups' | 'settings'>('overview');

  // Credentials reveal state
  readonly connectionUrl = signal<string | null>(null);
  readonly databaseUser = signal<string | null>(null);
  readonly databasePassword = signal<string | null>(null);
  readonly credentialsRevealed = signal(false);

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
  readonly cpuLimit = signal(250); // mCPU
  readonly memLimit = signal(512); // MB
  readonly savingSettings = signal(false);
  readonly saveSettingsSuccess = signal(false);

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
        if (tab === 'overview' || tab === 'console' || tab === 'logs' || tab === 'backups' || tab === 'settings') {
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
        console.log('[DbDetail] Database status changed in WS, reloading:', payload);
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
        this.cpuLimit.set(res.cpuLimit || 250);
        this.memLimit.set(res.memoryLimitMb || 512);
        this.loading.set(false);
        
        // Start ticker if database is still provisioning
        if (res.status === 'provisioning') {
          this.startTicker();
        } else {
          this.stopTicker();
        }
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea detaliilor bazei de date.');
        this.loading.set(false);
        this.stopTicker();
      }
    });
  }

  switchTab(tab: 'overview' | 'console' | 'logs' | 'backups' | 'settings'): void {
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
        this.toast.error(err.error?.message || 'Nu aveți permisiunea de a decripta credențialele.');
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
          {
            query,
            output: res.output,
            isError: res.isError,
            timestamp: new Date()
          },
          ...history
        ]);
        this.queryInput.set('');
        this.queryLoading.set(false);
        setTimeout(() => this.scrollConsoleToBottom(), 50);
      },
      error: (err) => {
        this.queryHistory.update(history => [
          {
            query,
            output: err.error?.message || 'Eroare la comunicarea cu baza de date.',
            isError: true,
            timestamp: new Date()
          },
          ...history
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
    this.logs.set(['[Console] Se conectează la stream-ul de logs Kubernetes...']);

    const streamUrl = this.dbService.getLogsStreamUrl(id);
    this.eventSource = new EventSource(streamUrl);

    this.eventSource.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] Conexiune stabilă. Se citesc logs din pod:']);
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
      this.logs.update(lines => [...lines, '[Aviz] Conexiunea a fost întreruptă. Se încearcă reconectarea...']);
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
      memoryLimitMb: this.memLimit()
    }).subscribe({
      next: () => {
        this.savingSettings.set(false);
        this.saveSettingsSuccess.set(true);
        this.toast.success('Limitele resurselor au fost salvate cu succes. Pod-ul se va redeploya automat!');
        this.loadDatabaseDetails(id, true);
        setTimeout(() => this.saveSettingsSuccess.set(false), 3000);
      },
      error: (err) => {
        this.savingSettings.set(false);
        this.toast.error(err.error?.message || 'Eroare la salvarea setărilor.');
      }
    });
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
        this.toast.error(err.error?.message || 'Eroare la încărcarea copiilor de siguranță.');
        this.loadingBackups.set(false);
      }
    });
  }

  onCreateBackup(): void {
    const id = this.dbId();
    if (!id) return;

    this.toast.info('Se inițializează crearea copiei de siguranță...');
    this.dbService.createBackup(id).subscribe({
      next: () => {
        this.toast.success('Copia de siguranță a fost creată cu succes.');
        this.loadBackups(id);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea copiei de siguranță.');
      }
    });
  }

  async onRestoreBackup(backup: DbBackup): Promise<void> {
    const id = this.dbId();
    if (!id) return;

    const confirmed = await this.confirm.ask({
      title: 'Restaurare Bază de Date',
      message: `Sigur doriți să restaurați baza de date folosind copia din ${new Date(backup.createdAt).toLocaleString()}? Datele curente vor fi suprascrise!`,
      confirmText: 'Restaurează',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.toast.info('Se restaurează baza de date...');
    this.dbService.restoreBackup(id, backup.id).subscribe({
      next: () => {
        this.toast.success('Baza de date a fost restaurată cu succes.');
        this.loadDatabaseDetails(id, true);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la restaurarea bazei de date.');
      }
    });
  }

  async onDeleteBackup(backup: DbBackup): Promise<void> {
    const id = this.dbId();
    if (!id) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Copie de Siguranță',
      message: `Sigur doriți să ștergeți copia de siguranță "${backup.filename}"? Această acțiune este ireversibilă!`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.dbService.deleteBackup(id, backup.id).subscribe({
      next: () => {
        this.toast.success('Copia de siguranță a fost ștearsă.');
        this.loadBackups(id);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea copiei de siguranță.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }

  async onDeleteDatabase(): Promise<void> {
    const id = this.dbId();
    const dbData = this.db();
    if (!id || !dbData) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Bază de Date',
      message: `Sigur doriți să ștergeți această bază de date "${dbData.name}"? Toate datele stocate vor fi șterse definitiv!`,
      confirmText: 'Șterge definitiv',
      cancelText: 'Anulează',
      isDanger: true,
      matchText: dbData.name
    });
    if (!confirmed) return;

    this.dbService.deleteDatabase(id).subscribe({
      next: () => {
        this.toast.success('Baza de date a fost ștearsă.');
        this.router.navigate(['/projects', this.parent.projectId(), 'databases']);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea bazei de date.');
      }
    });
  }
}
