import { Component, inject, signal, computed, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, AppDetail, AppBuild, EnvResponse, AppInstance, ProjectEnvResponse } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { DomainService } from '../../../../../../core/services/domain.service';
import { WorkspaceService, Workspace } from '../../../../../../core/services/workspace.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription, interval } from 'rxjs';
import { HttpEventType } from '@angular/common/http';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

@Component({
  selector: 'app-app-detail',
  standalone: true,
  imports: [CommonModule, DatePipe, DecimalPipe, FormsModule, RouterLink, Pagination],
  templateUrl: './app-detail.html',
  styleUrl: './app-detail.css',
})
export class AppDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly domainService = inject(DomainService);
  private readonly workspaceService = inject(WorkspaceService);
  private readonly wsService = inject(WebSocketService);

  readonly appId = signal<string | null>(null);
  readonly app = signal<AppDetail | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Active sub-tab state
  readonly activeSubTab = signal<'telemetry' | 'logs' | 'general' | 'env' | 'advanced'>('telemetry');

  // Telemetry signals
  readonly activeInstanceId = signal<string | null>(null);
  readonly selectedRange = signal('1h');
  readonly cpuValues = signal<number[]>([]);
  readonly memValues = signal<number[]>([]);
  readonly netRxValues = signal<number[]>([]);
  readonly netTxValues = signal<number[]>([]);
  readonly fsReadValues = signal<number[]>([]);
  readonly fsWriteValues = signal<number[]>([]);
  readonly metricsLoading = signal(false);
  readonly metricsSimulated = signal(false);

  // Current usage vs allocated quota (consumed / allocated)
  readonly cpuCurrent = computed(() => {
    const v = this.cpuValues();
    return v.length > 0 ? v[v.length - 1] : 0;
  });
  readonly memCurrent = computed(() => {
    const v = this.memValues();
    return v.length > 0 ? v[v.length - 1] : 0;
  });
  readonly cpuUsedPct = computed(() => {
    const limit = this.cpuLimit();
    return limit > 0 ? Math.min(100, Math.round((this.cpuCurrent() / limit) * 100)) : 0;
  });
  readonly memUsedPct = computed(() => {
    const limit = this.memLimit();
    return limit > 0 ? Math.min(100, Math.round((this.memCurrent() / limit) * 100)) : 0;
  });

  // Deploy new branch signals
  readonly newBranchName = signal('');
  readonly deployingBranch = signal(false);

  // Logs console signals
  readonly builds = signal<AppBuild[]>([]);
  readonly buildsPage = signal(1);
  readonly buildsPageSize = signal(DEFAULT_PAGE_SIZE);
  readonly buildsTotal = signal(0);
  readonly buildsLoading = signal(false);
  readonly selectedBuildId = signal<string | null>(null);
  readonly selectedBuildLogs = signal<string>('');
  readonly loadingBuildLogs = signal(false);
  readonly logs = signal<string[]>([]);
  readonly sseConnected = signal(false);
  readonly autoScroll = signal(true);

  // Live timer & polling signals
  readonly timeTicker = signal<number>(Date.now());
  private tickerInterval: any = null;

  readonly selectedBuild = computed(() => {
    const id = this.selectedBuildId();
    if (!id) return null;
    return this.builds().find(b => b.id === id) || null;
  });

  // Build lifecycle stepper
  readonly buildPhaseSteps = [
    { key: 'queued', label: 'În coadă' },
    { key: 'cloning', label: 'Clonare cod' },
    { key: 'building', label: 'Construire imagine' },
    { key: 'deploying', label: 'Deploy cluster' },
    { key: 'running', label: 'Live' },
  ];

  // The build whose lifecycle the stepper reflects: the selected one, else the latest.
  readonly displayBuild = computed(() => this.selectedBuild() || this.builds()[0] || null);

  phaseIndex(phase: string | undefined | null): number {
    if (!phase) return -1;
    return this.buildPhaseSteps.findIndex(s => s.key === phase);
  }

  isBuildFailed(build: AppBuild | null): boolean {
    if (!build) return false;
    const s = build.status;
    const p = build.phase;
    return s === 'failed' || s === 'cancelled' || s === 'timed_out' || s === 'superseded' || s === 'crashed'
        || p === 'failed' || p === 'cancelled' || p === 'timed_out' || p === 'superseded' || p === 'crashed';
  }

  isBuildSucceeded(build: AppBuild | null): boolean {
    return !!build && build.status === 'succeeded';
  }

  isBuildInProgress(build: AppBuild | null): boolean {
    return !!build && build.status === 'building';
  }

  buildOutcomeLabel(build: AppBuild | null): string {
    if (!build) return '';
    // A successfully-built image can still fail/crash at the deploy or runtime
    // phase — label by the phase in that case (the build itself is fine).
    let v: string | undefined | null;
    if (build.status === 'building') {
      v = build.phase || 'building';
    } else if (build.status === 'succeeded' && (build.phase === 'failed' || build.phase === 'crashed')) {
      v = build.phase;
    } else {
      v = build.status;
    }
    switch (v) {
      case 'cancelled': return 'Build anulat';
      case 'timed_out': return 'Build expirat (timeout)';
      case 'superseded': return 'Înlocuit de un build mai nou';
      case 'crashed': return 'Aplicația a crăpat la pornire';
      case 'failed': return build.status === 'succeeded' ? 'Deploy eșuat (build OK)' : 'Build eșuat';
      case 'queued': return 'În coadă';
      default: return 'Build eșuat';
    }
  }

  readonly retryingBuild = signal(false);
  readonly cancellingBuild = signal(false);
  readonly rollingBackId = signal<string | null>(null);

  onRollbackBuild(build: AppBuild): void {
    const appId = this.appId();
    if (!appId || this.rollingBackId()) return;

    this.rollingBackId.set(build.id);
    this.projectService.rollbackBuild(appId, build.id).subscribe({
      next: () => {
        this.rollingBackId.set(null);
        this.toast.success('Rollback pornit — acest build devine cel activ (LIVE).');
        this.pushSystemLog(`⏪ Rollback la build ${build.id.substring(0, 8)} — se re-deployează imaginea acestui build...`);
        // Select the rolled-back build so it's highlighted; the LIVE badge follows
        // once the builds list refreshes against the instance's new image.
        this.onViewBuildLogs(build);
        this.loadAppDetails();
        this.loadBuilds(true);
      },
      error: (err) => {
        this.rollingBackId.set(null);
        this.toast.error(err.error?.message || err.error?.error?.message || 'Eroare la rollback.');
      }
    });
  }

  onCancelBuild(build: AppBuild): void {
    const appId = this.appId();
    if (!appId || this.cancellingBuild()) return;

    this.cancellingBuild.set(true);
    this.projectService.cancelBuild(appId, build.id).subscribe({
      next: () => {
        this.cancellingBuild.set(false);
        this.toast.success('Build-ul a fost anulat.');
        this.loadAppDetails();
      },
      error: (err) => {
        this.cancellingBuild.set(false);
        this.toast.error(err.error?.message || err.error?.error?.message || 'Eroare la anularea build-ului.');
      }
    });
  }

  onRetryBuild(build: AppBuild): void {
    const appId = this.appId();
    if (!appId || this.retryingBuild()) return;

    this.retryingBuild.set(true);
    this.projectService.retryBuild(appId, build.id).subscribe({
      next: () => {
        this.retryingBuild.set(false);
        this.toast.success('Build-ul a fost repornit cu aceeași configurație.');
        this.selectedBuildId.set(null);
        this.loadAppDetails();
      },
      error: (err) => {
        this.retryingBuild.set(false);
        this.toast.error(err.error?.message || err.error?.error?.message || 'Eroare la repornirea build-ului.');
      }
    });
  }

  stepState(build: AppBuild | null, stepIndex: number): 'done' | 'active' | 'pending' {
    if (!build) return 'pending';
    // A succeeded build with no phase recorded counts as fully live.
    const cur = build.status === 'succeeded' && !build.phase
      ? this.buildPhaseSteps.length - 1
      : this.phaseIndex(build.phase);
    if (cur < 0) return 'pending';
    if (stepIndex < cur) return 'done';
    if (stepIndex === cur) return 'active';
    return 'pending';
  }

  private logsSocket: WebSocket | null = null;
  private logsReconnectTimer: any = null;
  private statsEventSource: EventSource | null = null;
  private lastCpuSystem: number | null = null;
  private lastCpuContainer: number | null = null;
  private connectedInstanceId: string | null = null;
  private wsSubscriptions = new Subscription();
  private buildLogsEventSource: EventSource | null = null;

  // Environment variables signals
  readonly envVariables = signal<EnvResponse[]>([]);
  readonly envVariablesLoading = signal(false);

  // Project-pool env available to this instance (with linked flag)
  readonly availableProjectEnv = signal<ProjectEnvResponse[]>([]);
  readonly togglingLinkId = signal<string | null>(null);
  readonly showCreateEnvForm = signal(false);
  // Non-null while editing an existing var (locks the key, value blank for secrets).
  readonly editingEnvId = signal<string | null>(null);
  // Add-variable panel mode: a brand-new var, or link one from the project pool.
  readonly addEnvMode = signal<'new' | 'project'>('new');
  // The project-pool vars this instance currently links (shown inline in the table).
  readonly linkedProjectEnv = computed(() => this.availableProjectEnv().filter(v => v.linked));
  readonly settingEnv = signal(false);
  readonly envKey = signal('');
  readonly envVal = signal('');
  readonly isSecret = signal(true);
  readonly revealedEnvIds = signal<Record<string, boolean>>({});

  // JSON editor signals
  readonly jsonEditMode = signal(false);
  readonly jsonText = signal('');
  readonly savingJson = signal(false);

  // App settings signals
  readonly cpuLimit = signal(0); // mCPU
  readonly memLimit = signal(0); // MB
  readonly internalPort = signal(8080);
  readonly externalPort = signal<number | null>(null);
  readonly buildCommand = signal('');
  readonly startCommand = signal('');
  readonly savingSettings = signal(false);
  readonly saveSettingsSuccess = signal(false);
  readonly enableBaas = signal(false);
  readonly workspace = signal<Workspace | null>(null);

  // Add Domain Modal
  readonly showAddDomainModal = signal(false);
  readonly appDomainFqdn = signal('');
  readonly addingDomain = signal(false);

  // Instance state control signals
  readonly stoppingInstance = signal(false);
  readonly startingInstance = signal(false);
  readonly redeployingInstance = signal(false);
  readonly reloadingInstance = signal(false);

  /** Append a Hermes system line into the live stdout view (visual feedback). */
  private pushSystemLog(msg: string): void {
    const ts = new Date().toLocaleTimeString();
    this.logs.update(lines => [...lines, `[Hermes ${ts}] ${msg}`]);
  }




  // Log filtering signals
  readonly logSearchQuery = signal('');
  readonly filteredLogs = computed(() => {
    const query = this.logSearchQuery().trim().toLowerCase();
    const rawLogs = this.logs();
    if (!query) return rawLogs;
    return rawLogs.filter(line => line.toLowerCase().includes(query));
  });
  readonly filteredBuildLogs = computed(() => {
    const query = this.logSearchQuery().trim().toLowerCase();
    const rawBuildLogs = this.selectedBuildLogs();
    if (!query) return rawBuildLogs;
    return rawBuildLogs.split('\n').filter(line => line.toLowerCase().includes(query)).join('\n');
  });

  readonly appSlug = computed(() => {
    const appData = this.app();
    if (!appData) return '';
    return appData.name.trim().toLowerCase().replace(/\s+/g, '-');
  });

  constructor() {
    // Re-connect telemetry and logs if activeInstance changes
    effect(() => {
      const appIdVal = this.appId();
      const instId = this.activeInstanceId();
      if (appIdVal && instId) {
        this.loadMetrics();
        if (this.activeSubTab() === 'logs' && !this.selectedBuildId()) {
          this.connectLogs(instId);
        }
      }
    });

    // Re-connect live logs automatically when entering the logs tab
    effect(() => {
      const tab = this.activeSubTab();
      const instId = this.activeInstanceId();
      if (tab === 'logs' && instId && !this.selectedBuildId()) {
        this.connectLogs(instId);
      } else if (tab !== 'logs') {
        this.disconnectLogs();
      }
    });

    // Re-connect live telemetry automatically when entering the telemetry tab with 1h range
    effect(() => {
      const tab = this.activeSubTab();
      const instId = this.activeInstanceId();
      const range = this.selectedRange();
      if (tab === 'telemetry' && instId && range === '1h') {
        this.connectTelemetry(instId);
      } else {
        this.disconnectTelemetry();
      }
    });
  }

  ngOnInit(): void {
    this.loadWorkspace();
    this.route.paramMap.subscribe(params => {
      const aId = params.get('appId');
      if (!aId || aId === this.appId()) return;

      // Switching to a different app while the component is reused (Angular keeps
      // the same instance across :appId changes, so ngOnDestroy does NOT fire).
      // Tear down the previous app's live streams and clear per-app view state so
      // nothing from the old app leaks into the newly opened one.
      this.disconnectLogs();
      this.disconnectTelemetry();
      this.disconnectBuildLogs();
      this.activeInstanceId.set(null);
      this.selectedBuildId.set(null);
      this.builds.set([]);
      this.app.set(null);

      this.appId.set(aId);
      this.loadAppDetails();
    });

    this.route.queryParams.subscribe(params => {
      const tab = params['tab'];
      if (tab && ['telemetry', 'logs', 'general', 'env', 'advanced'].includes(tab)) {
        this.activeSubTab.set(tab as any);
      }
    });

    // Start ticker for live duration calculations
    this.tickerInterval = setInterval(() => {
      this.timeTicker.set(Date.now());
    }, 1000);

    this.setupWsSubscriptions();
  }

  ngOnDestroy(): void {
    this.disconnectLogs();
    this.disconnectTelemetry();
    this.disconnectBuildLogs();
    this.wsSubscriptions.unsubscribe();
    if (this.tickerInterval) {
      clearInterval(this.tickerInterval);
      this.tickerInterval = null;
    }
  }

  private setupWsSubscriptions(): void {
    this.wsSubscriptions.unsubscribe();
    this.wsSubscriptions = new Subscription();

    // 1. Instance Status Changes
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('instance_status_changed').subscribe(payload => {
        const appId = this.appId();
        const currentApp = this.app();
        
        const isCurrentInstance = payload.instance_id === this.activeInstanceId();
        const belongsToApp = currentApp?.instances?.some(inst => inst.id === payload.instance_id);
        
        if (appId && (isCurrentInstance || belongsToApp)) {
          this.loadAppDetails();
        }
      })
    );

    // 2. Build Status Changes
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('build_status_changed').subscribe(payload => {
        const appId = this.appId();
        
        if (appId && payload.app_id === appId) {
          
          this.loadBuilds(true);
          
          if (payload.build_id === this.selectedBuildId()) {
            this.fetchBuildLogs(payload.build_id);
          }
          
          this.loadAppDetails();
        }
      })
    );
  }

  loadAppDetails(): void {
    const appId = this.appId();
    if (!appId) return;

    this.loading.set(true);
    this.error.set(null);

    this.projectService.getAppDetails(appId).subscribe({
      next: (res) => {
        this.app.set(res);
        this.loading.set(false);
        this.parent.selectedApp.set(res);

        // Load envs and builds
        this.loadBuilds();
        this.loadEnvVariables();

        this.buildCommand.set(res.buildCommand || '');
        this.startCommand.set(res.startCommand || '');
        this.enableBaas.set(res.enableBaas || false);

        // Status transitions are handled reactively via WebSockets

        // Load settings values from first instance
        if (res.instances && res.instances.length > 0) {
          const inst = res.instances[0];
          this.cpuLimit.set(inst.cpuLimit !== undefined && inst.cpuLimit !== null ? inst.cpuLimit : 0);
          this.memLimit.set(inst.memoryLimitMb !== undefined && inst.memoryLimitMb !== null ? inst.memoryLimitMb : 0);
          this.internalPort.set(inst.internalPort || 8080);
          this.externalPort.set(inst.externalPort || null);



          if (!this.activeInstanceId()) {
            this.activeInstanceId.set(inst.id);
          }
        }
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la încărcarea detaliilor aplicației.');
        this.loading.set(false);
      }
    });
  }

  // --- Telemetry & Branch deployment ---
  loadMetrics(): void {
    const appId = this.appId();
    const instanceId = this.activeInstanceId();
    if (!appId || !instanceId) return;

    this.metricsLoading.set(true);
    
    // CPU
    this.projectService.getMetrics(appId, instanceId, 'cpu', this.selectedRange()).subscribe({
      next: (res) => {
        const millicoresValues = (res.values || []).map(val => val * 1000);
        this.cpuValues.set(millicoresValues);
        this.metricsSimulated.set(!!res.simulated);
        this.metricsLoading.set(false);
      },
      error: () => {
        this.cpuValues.set([]);
        this.metricsLoading.set(false);
      }
    });

    // Memory
    this.projectService.getMetrics(appId, instanceId, 'memory', this.selectedRange()).subscribe({
      next: (res) => {
        this.memValues.set(res.values || []);
      },
      error: () => {
        this.memValues.set([]);
      }
    });

    // Network Inbound (network_rx)
    this.projectService.getMetrics(appId, instanceId, 'network_rx', this.selectedRange()).subscribe({
      next: (res) => {
        this.netRxValues.set(res.values || []);
      },
      error: () => {
        this.netRxValues.set([]);
      }
    });

    // Network Outbound (network_tx)
    this.projectService.getMetrics(appId, instanceId, 'network_tx', this.selectedRange()).subscribe({
      next: (res) => {
        this.netTxValues.set(res.values || []);
      },
      error: () => {
        this.netTxValues.set([]);
      }
    });

    // File System Read (fs_read)
    this.projectService.getMetrics(appId, instanceId, 'fs_read', this.selectedRange()).subscribe({
      next: (res) => {
        this.fsReadValues.set(res.values || []);
      },
      error: () => {
        this.fsReadValues.set([]);
      }
    });

    // File System Write (fs_write)
    this.projectService.getMetrics(appId, instanceId, 'fs_write', this.selectedRange()).subscribe({
      next: (res) => {
        this.fsWriteValues.set(res.values || []);
      },
      error: () => {
        this.fsWriteValues.set([]);
      }
    });
  }

  onRangeChange(range: string): void {
    this.selectedRange.set(range);
    this.loadMetrics();
  }

  onInstanceChange(id: string): void {
    this.activeInstanceId.set(id);
    if (this.activeSubTab() === 'env') {
      this.loadEnvVariables();
    }
  }

  onSubTabChange(tab: 'telemetry' | 'logs' | 'general' | 'env' | 'advanced'): void {
    this.activeSubTab.set(tab);
    if (tab !== 'logs') {
      this.disconnectBuildLogs();
    }
    this.router.navigate([], {
      relativeTo: this.route,
      queryParams: { tab },
      queryParamsHandling: 'merge'
    });
  }

  deployBranch(): void {
    const appId = this.appId();
    if (!appId || !this.newBranchName()) return;

    this.deployingBranch.set(true);
    this.projectService.createBranchInstance(appId, this.newBranchName()).subscribe({
      next: () => {
        this.newBranchName.set('');
        this.deployingBranch.set(false);
        this.toast.success('Deployment-ul de branch a fost lansat cu succes!');
        this.loadAppDetails();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea deploymentului.');
        this.deployingBranch.set(false);
      }
    });
  }

  async deleteInstance(instanceId: string): Promise<void> {
    const appId = this.appId();
    if (!appId) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Instanță (Pod)',
      message: 'Sigur doriți să ștergeți această instanță? Kubernetes o va reporni automat dacă este asociată unui deployment activ, altfel va fi oprită.',
      confirmText: 'Șterge Pod',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteAppInstance(appId, instanceId).subscribe({
      next: () => {
        if (this.activeInstanceId() === instanceId) {
          this.activeInstanceId.set(null);
        }
        this.toast.success('Instanța a fost oprită.');
        this.loadAppDetails();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la opritul instanței.');
      }
    });
  }

  // --- Logs & Builds History ---
  loadBuilds(silent: boolean = false): void {
    const appId = this.appId();
    if (!appId) return;

    if (!silent) {
      this.buildsLoading.set(true);
    }
    this.projectService.listBuilds(appId, this.buildsPage(), this.buildsPageSize()).subscribe({
      next: (res) => {
        const items = res?.items || [];
        this.builds.set(items);
        this.buildsTotal.set(res?.total || 0);
        if (!silent) {
          this.buildsLoading.set(false);
        }

        // Auto-select and show active building log
        if (items.length > 0) {
          const latestBuild = items[0];
          if (latestBuild.status === 'building' && !this.selectedBuildId()) {
            this.onViewBuildLogs(latestBuild);
            this.activeSubTab.set('logs');
          }
        }
      },
      error: () => {
        this.builds.set([]);
        if (!silent) {
          this.buildsLoading.set(false);
        }
      }
    });
  }

  connectLogs(instanceId: string): void {
    const appId = this.appId();
    if (!appId) return;

    this.disconnectLogs();
    this.connectedInstanceId = instanceId;
    this.logs.set(['[Console] Se conectează la fluxul de logs (WebSocket)...']);

    const wsUrl = this.projectService.getLogsWsUrl(appId, instanceId);
    const socket = new WebSocket(wsUrl);
    this.logsSocket = socket;

    socket.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] Conexiune WebSocket stabilă. Recepționare logs în timp real:']);
    };

    socket.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => [...lines, event.data as string]);
        if (this.autoScroll()) {
          this.scrollToBottom();
        }
      }
    };

    socket.onclose = () => {
      // Ignore if this socket was already replaced or intentionally closed.
      if (this.logsSocket !== socket) return;
      this.sseConnected.set(false);
      this.logs.update(lines => [...lines, '[Aviz] Conexiunea la stream a fost întreruptă. Se reconectează...']);
      this.logsReconnectTimer = setTimeout(() => {
        if (this.logsSocket === socket && this.connectedInstanceId === instanceId) {
          this.connectLogs(instanceId);
        }
      }, 2500);
    };

    socket.onerror = () => {
      // The onclose handler that follows performs the reconnect.
      this.sseConnected.set(false);
    };
  }

  disconnectLogs(): void {
    if (this.logsReconnectTimer) {
      clearTimeout(this.logsReconnectTimer);
      this.logsReconnectTimer = null;
    }
    if (this.logsSocket) {
      const sock = this.logsSocket;
      this.logsSocket = null; // null first so onclose treats it as intentional
      try { sock.close(); } catch { /* ignore */ }
    }
    this.connectedInstanceId = null;
    this.sseConnected.set(false);
  }

  connectTelemetry(instanceId: string): void {
    const appId = this.appId();
    if (!appId) return;

    this.disconnectTelemetry();
    this.lastCpuSystem = null;
    this.lastCpuContainer = null;

    const streamUrl = this.projectService.getStatsStreamUrl(appId, instanceId);
    this.statsEventSource = new EventSource(streamUrl);

    this.statsEventSource.onmessage = (event) => {
      if (event.data) {
        try {
          const data = JSON.parse(event.data);

          // Backend reports honest availability — when metrics can't be read it
          // sends { available: false } rather than fabricating values. Skip the
          // sample and reset the CPU delta baseline so resumption doesn't spike.
          if (data.available === false) {
            this.lastCpuSystem = null;
            this.lastCpuContainer = null;
            return;
          }

          const memoryMb = data.memoryBytes / (1024 * 1024);

          let cpuMillicores: number | null = null;
          if (this.lastCpuSystem !== null && this.lastCpuContainer !== null) {
            const deltaSys = data.cpuSystem - this.lastCpuSystem;
            const deltaCont = data.cpuContainer - this.lastCpuContainer;
            if (deltaSys > 0) {
              cpuMillicores = (deltaCont / deltaSys) * 1000;
            }
          }
          this.lastCpuSystem = data.cpuSystem;
          this.lastCpuContainer = data.cpuContainer;

          // Append and limit rolling window to last 50 points
          this.memValues.update(vals => {
            const next = [...vals, memoryMb];
            if (next.length > 50) next.shift();
            return next;
          });

          if (cpuMillicores !== null) {
            const currentCpu = cpuMillicores;
            this.cpuValues.update(vals => {
              const next = [...vals, currentCpu];
              if (next.length > 50) next.shift();
              return next;
            });
          }

          // Network and disk are not part of the live stats stream; they are only
          // available from the historical Prometheus query, so we leave them as-is
          // here rather than fabricating values.
        } catch (e) {
          console.error('[Telemetry] Error parsing stats SSE stream:', e);
        }
      }
    };

    this.statsEventSource.onerror = () => {
      this.disconnectTelemetry();
    };
  }

  disconnectTelemetry(): void {
    if (this.statsEventSource) {
      this.statsEventSource.close();
      this.statsEventSource = null;
    }
    this.lastCpuSystem = null;
    this.lastCpuContainer = null;
  }

  toggleAutoScroll(): void {
    this.autoScroll.update(val => !val);
    if (this.autoScroll()) {
      this.scrollToBottom();
    }
  }

  onViewBuildLogs(build: AppBuild): void {
    this.selectedBuildId.set(build.id);
    this.loadingBuildLogs.set(true);
    this.disconnectLogs(); // Pause live container logs
    
    this.fetchBuildLogs(build.id);
  }

  fetchBuildLogs(buildId: string): void {
    const appId = this.appId();
    if (!appId) return;

    this.projectService.getBuildDetails(appId, buildId).subscribe({
      next: (res) => {
        this.selectedBuildLogs.set(res.logs || 'Nu există loguri înregistrate pentru acest build.');
        this.loadingBuildLogs.set(false);
        if (this.autoScroll()) {
          this.scrollToBottom();
        }

        if (res.status === 'building') {
          this.connectBuildLogs(buildId);
        } else {
          this.disconnectBuildLogs();
        }
      },
      error: () => {
        this.selectedBuildLogs.set('Eroare la încărcarea logurilor de build.');
        this.loadingBuildLogs.set(false);
        this.disconnectBuildLogs();
      }
    });
  }

  connectBuildLogs(buildId: string): void {
    const appId = this.appId();
    if (!appId) return;

    this.disconnectBuildLogs();
    this.selectedBuildLogs.set('[Console] Se conectează la fluxul de build live...');

    const streamUrl = this.projectService.getBuildLogsStreamUrl(appId, buildId);
    this.buildLogsEventSource = new EventSource(streamUrl);

    this.buildLogsEventSource.onmessage = (event) => {
      if (event.data) {
        this.selectedBuildLogs.update(logs => {
          if (logs === '[Console] Se conectează la fluxul de build live...') {
            return event.data + '\n';
          }
          return logs + event.data + '\n';
        });

        if (this.autoScroll()) {
          this.scrollToBottom();
        }
      }
    };

    this.buildLogsEventSource.onerror = () => {
      // Stream completed or disconnected. Stop stream and fetch final state.
      this.disconnectBuildLogs();
      this.loadBuilds(true);
      this.loadAppDetails();

      // Retrieve full logs from database
      this.projectService.getBuildDetails(appId, buildId).subscribe({
        next: (res) => {
          this.selectedBuildLogs.set(res.logs || 'Nu există loguri înregistrate pentru acest build.');
          if (this.autoScroll()) {
            this.scrollToBottom();
          }
        }
      });
    };
  }

  disconnectBuildLogs(): void {
    if (this.buildLogsEventSource) {
      this.buildLogsEventSource.close();
      this.buildLogsEventSource = null;
    }
  }

  formatDuration(seconds: number | undefined): string {
    if (seconds === undefined || seconds < 0) return '0s';
    if (seconds < 60) return `${seconds}s`;
    const mins = Math.floor(seconds / 60);
    const secs = seconds % 60;
    return `${mins}m ${secs}s`;
  }

  getLiveDuration(createdAt: string): string {
    const now = this.timeTicker();
    const elapsed = Math.floor((now - new Date(createdAt).getTime()) / 1000);
    return this.formatDuration(elapsed);
  }

  onBackToLiveLogs(): void {
    this.selectedBuildId.set(null);
    this.selectedBuildLogs.set('');
    this.disconnectBuildLogs();
    
    if (this.activeInstanceId()) {
      this.connectLogs(this.activeInstanceId()!);
    }
  }

  private scrollToBottom(): void {
    setTimeout(() => {
      const el = document.getElementById('terminal-log-box');
      if (el) {
        el.scrollTop = el.scrollHeight;
      }
    }, 50);
  }

  // --- Environment Variables (scoped to the active instance) ---
  loadEnvVariables(): void {
    const appInstanceId = this.activeInstanceId() || this.app()?.instances?.[0]?.id || null;
    if (!appInstanceId) return;

    this.envVariablesLoading.set(true);
    // The env tab has a JSON bulk editor that replaces ALL of an instance's vars,
    // so we must load the full set here (not a page) to avoid silently dropping the
    // rest on save. The endpoint is still paginated for external/API consumers.
    this.projectService.listEnvVariables(appInstanceId, 1, 1000).subscribe({
      next: (res) => {
        this.envVariables.set(res?.items || []);
        this.envVariablesLoading.set(false);
      },
      error: () => {
        this.envVariablesLoading.set(false);
      }
    });
    this.loadAvailableProjectEnv();
  }

  onBuildsPageChange(page: number): void {
    this.buildsPage.set(page);
    this.loadBuilds();
  }

  // --- Project-pool linking ---
  loadAvailableProjectEnv(): void {
    const appInstanceId = this.activeInstanceId() || this.app()?.instances?.[0]?.id || null;
    if (!appInstanceId) return;
    this.projectService.listInstanceProjectEnv(appInstanceId).subscribe({
      next: (res) => this.availableProjectEnv.set(res || []),
      error: () => this.availableProjectEnv.set([])
    });
  }

  onToggleProjectEnvLink(env: ProjectEnvResponse): void {
    const appInstanceId = this.activeInstanceId() || this.app()?.instances?.[0]?.id || null;
    if (!appInstanceId || this.togglingLinkId()) return;

    this.togglingLinkId.set(env.id);
    const req = env.linked
      ? this.projectService.unlinkProjectEnv(appInstanceId, env.id)
      : this.projectService.linkProjectEnv(appInstanceId, env.id);

    req.subscribe({
      next: () => {
        this.togglingLinkId.set(null);
        this.toast.success(env.linked ? 'Variabilă deconectată.' : 'Variabilă conectată.');
        this.loadAvailableProjectEnv();
      },
      error: (err) => {
        this.togglingLinkId.set(null);
        this.toast.error(err.error?.message || 'Eroare la actualizarea legăturii.');
      }
    });
  }

  onToggleReveal(id: string): void {
    this.revealedEnvIds.update(ids => ({
      ...ids,
      [id]: !ids[id]
    }));
  }

  // Open/close the add panel (resetting any edit state).
  onToggleCreateEnvForm(): void {
    const next = !this.showCreateEnvForm();
    this.showCreateEnvForm.set(next);
    this.editingEnvId.set(null);
    this.addEnvMode.set('new');
    this.envKey.set('');
    this.envVal.set('');
    this.isSecret.set(true);
  }

  // Open the add panel prefilled to edit an existing var (key locked; value blank for secrets).
  startEditEnv(env: EnvResponse): void {
    this.showCreateEnvForm.set(true);
    this.addEnvMode.set('new');
    this.editingEnvId.set(env.id);
    this.envKey.set(env.key);
    this.envVal.set(env.isSecret ? '' : (env.value ?? ''));
    this.isSecret.set(env.isSecret);
    setTimeout(() => document.getElementById('env-form-panel')?.scrollIntoView({ behavior: 'smooth', block: 'center' }), 0);
  }

  onSaveEnv(): void {
    const appInstanceId = this.activeInstanceId() || this.app()?.instances?.[0]?.id || null;
    if (!appInstanceId || !this.envKey().trim()) return;

    const wasEditing = this.editingEnvId() !== null;
    this.settingEnv.set(true);

    this.projectService.setEnvVariable({
      appInstanceId,
      key: this.envKey().trim(),
      value: this.envVal(),
      isSecret: this.isSecret()
    }).subscribe({
      next: () => {
        this.envKey.set('');
        this.envVal.set('');
        this.editingEnvId.set(null);
        this.showCreateEnvForm.set(false);
        this.settingEnv.set(false);
        this.toast.success(wasEditing ? 'Variabila de mediu a fost actualizată!' : 'Variabila de mediu a fost salvată!');
        this.loadEnvVariables();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea variabilei.');
        this.settingEnv.set(false);
      }
    });
  }

  // --- JSON bulk editor ---
  openJsonEditor(): void {
    const obj: Record<string, string> = {};
    for (const env of this.envVariables()) {
      obj[env.key] = env.isSecret ? '' : (env.value ?? '');
    }
    this.jsonText.set(JSON.stringify(obj, null, 2));
    this.jsonEditMode.set(true);
  }

  closeJsonEditor(): void {
    this.jsonEditMode.set(false);
  }

  saveJsonEnvs(): void {
    const appInstanceId = this.activeInstanceId() || this.app()?.instances?.[0]?.id || null;
    if (!appInstanceId) return;

    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(this.jsonText());
    } catch {
      this.toast.error('JSON invalid. Verificați sintaxa.');
      return;
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      this.toast.error('JSON-ul trebuie să fie un obiect { "CHEIE": "valoare" }.');
      return;
    }

    const variables = Object.entries(parsed).map(([key, value]) => ({
      key,
      value: value === null || value === undefined ? '' : String(value),
      isSecret: false
    }));

    this.savingJson.set(true);
    this.projectService.setEnvsBulk(appInstanceId, variables).subscribe({
      next: () => {
        this.savingJson.set(false);
        this.jsonEditMode.set(false);
        this.toast.success('Variabilele de mediu au fost actualizate.');
        this.loadEnvVariables();
      },
      error: (err) => {
        this.savingJson.set(false);
        this.toast.error(err.error?.message || 'Eroare la salvarea variabilelor.');
      }
    });
  }

  async onDeleteEnv(envId: string): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Ștergere Variabilă de Mediu',
      message: 'Sigur doriți să ștergeți această variabilă? Modificarea va fi aplicată containerelor la următorul redeploy.',
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    this.projectService.deleteEnvVariable(envId).subscribe({
      next: () => {
        this.toast.success('Variabila de mediu a fost ștearsă.');
        this.loadEnvVariables();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergere.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat!');
    });
  }

  // --- App Settings ---
  onSaveSettings(): void {
    const appData = this.app();
    if (!appData || !appData.instances || appData.instances.length === 0) return;

    const inst = appData.instances[0];
    this.savingSettings.set(true);
    this.saveSettingsSuccess.set(false);

    this.projectService.updateInstanceSettings(appData.id, inst.id, {
      cpuLimit: this.cpuLimit(),
      memoryLimitMb: this.memLimit(),
      internalPort: this.internalPort(),
      externalPort: this.externalPort() || null,
      buildCommand: this.buildCommand() || null,
      startCommand: this.startCommand() || null,
      enableBaas: this.enableBaas()
    }).subscribe({
      next: () => {
        this.savingSettings.set(false);
        this.saveSettingsSuccess.set(true);
        this.toast.success('Setările aplicației au fost salvate și redeploierea a început.');
        this.loadAppDetails();
        setTimeout(() => this.saveSettingsSuccess.set(false), 4000);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la actualizarea setărilor.');
        this.savingSettings.set(false);
      }
    });
  }

  async onDeleteApp(): Promise<void> {
    const appData = this.app();
    if (!appData) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Completă Aplicație',
      message: `Sigur doriți să ștergeți complet aplicația "${appData.name}"? Această acțiune este ireversibilă și va distruge toate instanțele active, build-urile și configurațiile de rutare din Kubernetes.`,
      confirmText: 'Șterge Aplicația',
      cancelText: 'Anulează',
      isDanger: true,
      matchText: appData.name
    });
    if (!confirmed) return;

    this.loading.set(true);
    this.projectService.deleteApp(appData.id).subscribe({
      next: () => {
        this.toast.success(`Aplicația "${appData.name}" a fost ștearsă.`);
        if (this.parent.selectedApp()?.id === appData.id) {
          this.parent.selectedApp.set(null);
        }
        this.router.navigate([`/projects/${this.parent.projectId()}/apps`]);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea aplicației.');
        this.loading.set(false);
      }
    });
  }

  // --- Charts SVG Helpers ---
  getSvgPath(values: number[]): string {
    if (values.length < 2) return '';
    const width = 500;
    const height = 150;
    const max = Math.max(...values, 0.1) * 1.1;
    const min = Math.min(...values, 0);

    return values.map((val, idx) => {
      const x = (idx / (values.length - 1)) * width;
      const y = height - ((val - min) / (max - min)) * height;
      return `${idx === 0 ? 'M' : 'L'} ${x.toFixed(1)} ${y.toFixed(1)}`;
    }).join(' ');
  }

  getSvgFillPath(values: number[]): string {
    const linePath = this.getSvgPath(values);
    if (!linePath) return '';
    return `${linePath} L 500 150 L 0 150 Z`;
  }

  getSelectedInstance(): AppInstance | null {
    const appData = this.app();
    if (!appData || !appData.instances || appData.instances.length === 0) return null;
    const activeId = this.activeInstanceId();
    if (activeId) {
      const found = appData.instances.find(inst => inst.id === activeId);
      if (found) return found;
    }
    return appData.instances[0];
  }

  getSelectedInstanceContainerName(): string | null {
    return this.getSelectedInstance()?.containerName || null;
  }

  getSelectedInstanceInternalPort(): number {
    return this.getSelectedInstance()?.internalPort || 80;
  }

  // Derive the stable in-cluster alias from the container name when networkAlias is
  // absent (old/auto apps): strip the trailing -<8 hex> instance hash.
  stripHash(name: string | null | undefined): string {
    return name ? name.replace(/-[0-9a-f]{8}$/, '') : (name || '');
  }

  onStopInstance(): void {
    const appId = this.appId();
    const instanceId = this.activeInstanceId();
    if (!appId || !instanceId) return;

    this.stoppingInstance.set(true);
    this.projectService.stopAppInstance(appId, instanceId).subscribe({
      next: () => {
        this.stoppingInstance.set(false);
        this.toast.success('Instanța a fost oprită (scale 0).');
        this.loadAppDetails();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la oprirea instanței.');
        this.stoppingInstance.set(false);
      }
    });
  }

  onStartInstance(): void {
    const appId = this.appId();
    const instanceId = this.activeInstanceId();
    if (!appId || !instanceId) return;

    this.startingInstance.set(true);
    this.projectService.startAppInstance(appId, instanceId).subscribe({
      next: () => {
        this.startingInstance.set(false);
        this.toast.success('Instanța a fost pornită (scale 1).');
        this.loadAppDetails();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la pornirea instanței.');
        this.startingInstance.set(false);
      }
    });
  }

  // Redeploy = full rebuild from Git.
  onRedeployInstance(): void {
    const appId = this.appId();
    const instanceId = this.activeInstanceId();
    if (!appId || !instanceId) return;

    this.redeployingInstance.set(true);
    this.projectService.redeployAppInstance(appId, instanceId).subscribe({
      next: () => {
        this.redeployingInstance.set(false);
        this.toast.success('Rebuild pornit — se reconstruiește imaginea din Git.');
        this.pushSystemLog('🔨 Redeploy (rebuild) declanșat — se reconstruiește imaginea din Git...');
        this.loadAppDetails();
        this.loadBuilds(true);
      },
      error: (err) => {
        this.toast.error(err.error?.message || err.error?.error?.message || 'Eroare la pornirea rebuild-ului.');
        this.redeployingInstance.set(false);
      }
    });
  }

  // Reload = re-apply the current image with fresh config/env (no rebuild).
  onReloadInstance(): void {
    const appId = this.appId();
    const instanceId = this.activeInstanceId();
    if (!appId || !instanceId || this.reloadingInstance()) return;

    this.reloadingInstance.set(true);
    this.projectService.reloadAppInstance(appId, instanceId).subscribe({
      next: () => {
        this.reloadingInstance.set(false);
        this.toast.success('Reload lansat — se re-aplică imaginea curentă cu env-ul actualizat.');
        this.pushSystemLog('🔄 Reload declanșat — se re-aplică imaginea curentă cu configurația și env-ul actualizate...');
        this.loadAppDetails();
      },
      error: (err) => {
        this.toast.error(err.error?.message || err.error?.error?.message || 'Eroare la reload.');
        this.reloadingInstance.set(false);
      }
    });
  }

  onOpenAddDomainModal(): void {
    const container = this.getSelectedInstanceContainerName();
    if (!container) {
      this.toast.error('Această aplicație nu are nicio instanță activă (pod) lansată. Lansați un build mai întâi.');
      return;
    }
    this.appDomainFqdn.set('');
    this.showAddDomainModal.set(true);
  }

  onAddDomainSubmit(): void {
    const fqdnVal = this.appDomainFqdn().trim();
    const instanceId = this.activeInstanceId() || this.app()?.instances?.[0]?.id || null;
    if (!fqdnVal) {
      this.toast.error('Numele domeniului este obligatoriu.');
      return;
    }
    if (!instanceId) {
      this.toast.error('Nu s-a putut asocia deoarece aplicația nu are o instanță activă.');
      return;
    }

    this.addingDomain.set(true);
    this.domainService.addDomain({
      fqdn: fqdnVal,
      targetType: 'app',
      targetId: instanceId,
      clientMaxBodySize: 50,
      isSsl: true
    }).subscribe({
      next: () => {
        this.addingDomain.set(false);
        this.showAddDomainModal.set(false);
        this.toast.success(`Domeniul "${fqdnVal}" a fost asociat cu succes!`);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la asocierea domeniului.');
        this.addingDomain.set(false);
      }
    });
  }


  onDownloadLogs(): void {
    let logsText = '';
    let filename = 'app-logs.log';

    if (this.selectedBuildId()) {
      logsText = this.selectedBuildLogs();
      filename = `build-${this.selectedBuildId()?.substring(0, 8)}.log`;
    } else {
      logsText = this.logs().join('\n');
      const activeInst = this.getSelectedInstance();
      if (activeInst) {
        filename = `instance-${activeInst.containerName}.log`;
      }
    }

    const blob = new Blob([logsText], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    this.toast.success('Logurile au fost descărcate cu succes!');
  }



  loadWorkspace(): void {
    this.workspaceService.getCurrentWorkspace().subscribe({
      next: (res) => this.workspace.set(res),
      error: (err) => console.error('Eroare la încărcarea workspace-ului', err)
    });
  }
}
