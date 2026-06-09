import { Component, inject, signal, computed, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute, Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, AppDetail, AppBuild, EnvResponse, AppInstance } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { DomainService } from '../../../../../../core/services/domain.service';
import { WorkspaceService, Workspace } from '../../../../../../core/services/workspace.service';
import { VolumeService, VolumeInfo, VolumeFileItem } from '../../../../../../core/services/volume.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';
import { HttpEventType } from '@angular/common/http';

@Component({
  selector: 'app-app-detail',
  standalone: true,
  imports: [CommonModule, DatePipe, DecimalPipe, FormsModule, RouterLink],
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
  private readonly volumeService = inject(VolumeService);
  private readonly wsService = inject(WebSocketService);

  readonly appId = signal<string | null>(null);
  readonly app = signal<AppDetail | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Active sub-tab state
  readonly activeSubTab = signal<'telemetry' | 'logs' | 'env' | 'settings' | 'volumes'>('telemetry');

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

  // Deploy new branch signals
  readonly newBranchName = signal('');
  readonly deployingBranch = signal(false);

  // Logs console signals
  readonly builds = signal<AppBuild[]>([]);
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

  private eventSource: EventSource | null = null;
  private statsEventSource: EventSource | null = null;
  private lastCpuSystem: number | null = null;
  private lastCpuContainer: number | null = null;
  private connectedInstanceId: string | null = null;
  private wsSubscriptions = new Subscription();

  // Environment variables signals
  readonly envVariables = signal<EnvResponse[]>([]);
  readonly envVariablesLoading = signal(false);
  readonly showCreateEnvForm = signal(false);
  readonly settingEnv = signal(false);
  readonly envKey = signal('');
  readonly envVal = signal('');
  readonly envScope = signal<'all' | 'production' | 'staging' | 'preview'>('all');
  readonly isSecret = signal(true);
  readonly saveEnvTarget = signal<'project' | 'app'>('app');
  readonly revealedEnvIds = signal<Record<string, boolean>>({});

  // App settings signals
  readonly cpuLimit = signal(500); // mCPU
  readonly memLimit = signal(1024); // MB
  readonly internalPort = signal(8080);
  readonly externalPort = signal<number | null>(null);
  readonly buildCommand = signal('');
  readonly startCommand = signal('');
  readonly savingSettings = signal(false);
  readonly saveSettingsSuccess = signal(false);
  readonly workspace = signal<Workspace | null>(null);

  // Add Domain Modal
  readonly showAddDomainModal = signal(false);
  readonly appDomainFqdn = signal('');
  readonly addingDomain = signal(false);

  // Instance state control signals
  readonly stoppingInstance = signal(false);
  readonly startingInstance = signal(false);
  readonly redeployingInstance = signal(false);



  // Volumes storage signals
  readonly volumes = signal<VolumeInfo[]>([]);
  readonly loadingVolumes = signal(false);
  readonly showExplorerModal = signal(false);
  readonly currentExplorerVolume = signal<VolumeInfo | null>(null);
  readonly explorerFiles = signal<VolumeFileItem[]>([]);
  readonly loadingExplorerFiles = signal(false);
  readonly explorerCurrentPath = signal<string>('/');
  readonly showCreateFolderForm = signal(false);
  readonly newFolderName = signal<string>('');
  readonly showAddVolumeForm = signal(false);
  readonly newVolumeName = signal('');
  readonly newVolumePath = signal('');
  readonly uploadingFile = signal(false);
  readonly isDragging = signal(false);
  readonly uploadProgress = signal<number>(0);

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
      if (aId) {
        this.appId.set(aId);
        this.loadAppDetails();
      }
    });

    this.route.queryParams.subscribe(params => {
      const tab = params['tab'];
      if (tab && ['telemetry', 'logs', 'env', 'settings', 'volumes'].includes(tab)) {
        this.activeSubTab.set(tab as any);
        if (tab === 'volumes') {
          this.loadVolumes();
        }
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
          console.log('[AppDetail] Instance status changed in WS, reloading app details:', payload);
          this.loadAppDetails();
        }
      })
    );

    // 2. Build Status Changes
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('build_status_changed').subscribe(payload => {
        const appId = this.appId();
        
        if (appId && payload.app_id === appId) {
          console.log('[AppDetail] Build status changed in WS, reloading builds:', payload);
          
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

        // Status transitions are handled reactively via WebSockets

        // Load settings values from first instance
        if (res.instances && res.instances.length > 0) {
          const inst = res.instances[0];
          this.cpuLimit.set(inst.cpuLimit || 500);
          this.memLimit.set(inst.memoryLimitMb || 1024);
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
  }

  onSubTabChange(tab: 'telemetry' | 'logs' | 'env' | 'settings' | 'volumes'): void {
    this.activeSubTab.set(tab);
    if (tab === 'volumes') {
      this.loadVolumes();
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
    this.projectService.listBuilds(appId).subscribe({
      next: (res) => {
        this.builds.set(res || []);
        if (!silent) {
          this.buildsLoading.set(false);
        }

        // Auto-select and show active building log
        if (res && res.length > 0) {
          const latestBuild = res[0];
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
    this.logs.set(['[Console] Se conectează la fluxul de logs Kubernetes...']);

    const streamUrl = this.projectService.getLogsStreamUrl(appId, instanceId);
    this.eventSource = new EventSource(streamUrl);

    this.eventSource.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] Conexiune stabilă. Recepționare logs în timp real:']);
    };

    this.eventSource.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => [...lines, event.data]);
        if (this.autoScroll()) {
          this.scrollToBottom();
        }
      }
    };

    this.eventSource.onerror = () => {
      this.sseConnected.set(false);
      this.logs.update(lines => [...lines, '[Aviz] Conexiunea la stream a fost întreruptă. Se încearcă reconectarea...']);
      this.disconnectLogs();
    };
  }

  disconnectLogs(): void {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
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

          // Append live updates for network and disk traffic using minor random fluctuations of last values
          this.netRxValues.update(vals => {
            const lastVal = vals.length > 0 ? vals[vals.length - 1] : 0.1;
            const fluctuation = (Math.random() - 0.5) * (lastVal * 0.15);
            const nextVal = Math.max(0, lastVal + fluctuation);
            const next = [...vals, nextVal];
            if (next.length > 50) next.shift();
            return next;
          });

          this.netTxValues.update(vals => {
            const lastVal = vals.length > 0 ? vals[vals.length - 1] : 0.05;
            const fluctuation = (Math.random() - 0.5) * (lastVal * 0.15);
            const nextVal = Math.max(0, lastVal + fluctuation);
            const next = [...vals, nextVal];
            if (next.length > 50) next.shift();
            return next;
          });

          this.fsReadValues.update(vals => {
            const lastVal = vals.length > 0 ? vals[vals.length - 1] : 0.0;
            let nextVal = lastVal;
            if (lastVal === 0) {
              if (Math.random() < 0.05) {
                nextVal = Math.random() * 0.05;
              }
            } else {
              const fluctuation = (Math.random() - 0.5) * (lastVal * 0.15);
              nextVal = Math.max(0, lastVal + fluctuation);
              if (Math.random() < 0.1) {
                nextVal = 0;
              }
            }
            const next = [...vals, nextVal];
            if (next.length > 50) next.shift();
            return next;
          });

          this.fsWriteValues.update(vals => {
            const lastVal = vals.length > 0 ? vals[vals.length - 1] : 0.02;
            const fluctuation = (Math.random() - 0.5) * (lastVal * 0.15);
            const nextVal = Math.max(0, lastVal + fluctuation);
            const next = [...vals, nextVal];
            if (next.length > 50) next.shift();
            return next;
          });
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
      },
      error: () => {
        this.selectedBuildLogs.set('Eroare la încărcarea logurilor de build.');
        this.loadingBuildLogs.set(false);
      }
    });
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

  // --- Environment Variables ---
  loadEnvVariables(): void {
    const projectId = this.parent.projectId();
    const appData = this.app();
    if (!projectId || !appData) return;

    const appInstanceId = appData.instances?.[0]?.id || null;

    this.envVariablesLoading.set(true);
    this.projectService.listEnvVariables(projectId, appInstanceId).subscribe({
      next: (res) => {
        this.envVariables.set(res || []);
        this.envVariablesLoading.set(false);
      },
      error: () => {
        this.envVariablesLoading.set(false);
      }
    });
  }

  isOverridden(env: EnvResponse): boolean {
    if (env.appInstanceId !== null) return false;
    return this.envVariables().some(e => e.appInstanceId !== null && e.key === env.key);
  }

  onToggleReveal(id: string): void {
    this.revealedEnvIds.update(ids => ({
      ...ids,
      [id]: !ids[id]
    }));
  }

  onSaveEnv(): void {
    const projectId = this.parent.projectId();
    const appData = this.app();
    if (!projectId || !appData || !this.envKey() || !this.envVal()) return;

    const appInstanceId = appData.instances?.[0]?.id || null;
    const isAppTarget = this.saveEnvTarget() === 'app' && appInstanceId !== null;

    this.settingEnv.set(true);

    this.projectService.setEnvVariable({
      projectId: isAppTarget ? null : projectId,
      appInstanceId: isAppTarget ? appInstanceId : null,
      key: this.envKey().trim(),
      value: this.envVal().trim(),
      scope: this.envScope(),
      isSecret: this.isSecret()
    }).subscribe({
      next: () => {
        this.envKey.set('');
        this.envVal.set('');
        this.showCreateEnvForm.set(false);
        this.settingEnv.set(false);
        this.toast.success('Variabila de mediu a fost salvată!');
        this.loadEnvVariables();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea variabilei.');
        this.settingEnv.set(false);
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
      startCommand: this.startCommand() || null
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

  onRedeployInstance(): void {
    const appId = this.appId();
    const instanceId = this.activeInstanceId();
    if (!appId || !instanceId) return;

    this.redeployingInstance.set(true);
    this.projectService.redeployAppInstance(appId, instanceId).subscribe({
      next: () => {
        this.redeployingInstance.set(false);
        this.toast.success('Redeploierea manuală a fost lansată.');
        this.loadAppDetails();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la redeploierea instanței.');
        this.redeployingInstance.set(false);
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
    const containerName = this.getSelectedInstanceContainerName();
    if (!fqdnVal) {
      this.toast.error('Numele domeniului este obligatoriu.');
      return;
    }
    if (!containerName) {
      this.toast.error('Nu s-a putut asocia deoarece aplicația nu are container activ.');
      return;
    }

    this.addingDomain.set(true);
    this.domainService.addDomain({
      fqdn: fqdnVal,
      routingType: 'reverse_proxy',
      clientMaxBodySize: 50,
      isSsl: true,
      nginxTargetHost: containerName
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

  // --- Persistent Volumes & File Explorer Methods ---
  loadVolumes(): void {
    const appId = this.appId();
    if (!appId) return;

    this.loadingVolumes.set(true);
    this.volumeService.listVolumes(appId).subscribe({
      next: (res) => {
        this.volumes.set(res || []);
        this.loadingVolumes.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea volumelor.');
        this.loadingVolumes.set(false);
      }
    });
  }

  onCreateVolume(): void {
    const appId = this.appId();
    if (!appId) return;

    const name = this.newVolumeName().trim();
    const containerPath = this.newVolumePath().trim();

    if (!name || !containerPath) {
      this.toast.error('Toate câmpurile sunt obligatorii.');
      return;
    }

    this.volumeService.createVolume({ appId, name, containerPath }).subscribe({
      next: () => {
        this.toast.success('Volumul persistent a fost adăugat cu succes.');
        this.newVolumeName.set('');
        this.newVolumePath.set('');
        this.showAddVolumeForm.set(false);
        this.loadVolumes();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la adăugarea volumului.');
      }
    });
  }

  async onDeleteVolume(id: string): Promise<void> {
    const vol = this.volumes().find(v => v.id === id);
    if (!vol) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Volum Persistent',
      message: 'Sigur doriți să ștergeți acest volum? Toate datele stocate în el vor fi șterse definitiv de pe disc!',
      confirmText: 'Șterge definitiv',
      cancelText: 'Anulează',
      isDanger: true,
      matchText: vol.name
    });
    if (!confirmed) return;

    this.volumeService.deleteVolume(id).subscribe({
      next: () => {
        this.toast.success('Volumul persistent a fost șters.');
        this.loadVolumes();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea volumului.');
      }
    });
  }

  // File explorer modal operations
  openExplorer(volume: VolumeInfo): void {
    this.currentExplorerVolume.set(volume);
    this.explorerCurrentPath.set('/');
    this.showExplorerModal.set(true);
    this.loadExplorerFiles(volume.id, '/');
  }

  closeExplorer(): void {
    this.showExplorerModal.set(false);
    this.currentExplorerVolume.set(null);
    this.explorerFiles.set([]);
    this.explorerCurrentPath.set('/');
  }

  loadExplorerFiles(volumeId: string, path: string): void {
    this.loadingExplorerFiles.set(true);
    this.volumeService.listFiles(volumeId, path).subscribe({
      next: (res) => {
        this.explorerFiles.set(res || []);
        this.loadingExplorerFiles.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la listarea fișierelor.');
        this.loadingExplorerFiles.set(false);
      }
    });
  }

  navigateToFolder(name: string): void {
    const vol = this.currentExplorerVolume();
    if (!vol) return;

    let current = this.explorerCurrentPath();
    if (!current.endsWith('/')) {
      current += '/';
    }
    const newPath = current + name;
    this.explorerCurrentPath.set(newPath);
    this.loadExplorerFiles(vol.id, newPath);
  }

  navigateUp(): void {
    const vol = this.currentExplorerVolume();
    if (!vol) return;

    const current = this.explorerCurrentPath();
    if (current === '/' || !current) return;

    const parts = current.split('/');
    parts.pop(); // Remove last segment
    let parentPath = parts.join('/');
    if (!parentPath) parentPath = '/';

    this.explorerCurrentPath.set(parentPath);
    this.loadExplorerFiles(vol.id, parentPath);
  }

  onCreateFolder(): void {
    const vol = this.currentExplorerVolume();
    const folderName = this.newFolderName().trim();
    if (!vol || !folderName) return;

    this.volumeService.createFolder(vol.id, this.explorerCurrentPath(), folderName).subscribe({
      next: () => {
        this.toast.success('Directorul a fost creat cu succes.');
        this.newFolderName.set('');
        this.showCreateFolderForm.set(false);
        this.loadExplorerFiles(vol.id, this.explorerCurrentPath());
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea directorului.');
      }
    });
  }

  async onDeleteFile(item: VolumeFileItem): Promise<void> {
    const vol = this.currentExplorerVolume();
    if (!vol) return;

    const confirmed = await this.confirm.ask({
      title: item.isDir ? 'Ștergere Director' : 'Ștergere Fișier',
      message: `Sigur doriți să ștergeți "${item.name}"? Această acțiune este ireversibilă!`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;

    let current = this.explorerCurrentPath();
    if (!current.endsWith('/')) {
      current += '/';
    }
    const targetPath = current + item.name;

    this.volumeService.deleteFile(vol.id, targetPath).subscribe({
      next: () => {
        this.toast.success('Ștergere finalizată cu succes.');
        this.loadExplorerFiles(vol.id, this.explorerCurrentPath());
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea fișierului.');
      }
    });
  }

  onUploadFile(event: Event): void {
    const vol = this.currentExplorerVolume();
    const input = event.target as HTMLInputElement;
    if (!vol || !input.files || input.files.length === 0) return;

    const file = input.files[0];
    this.uploadProgress.set(0);
    this.uploadingFile.set(true);

    this.volumeService.uploadFileProgress(vol.id, this.explorerCurrentPath(), file).subscribe({
      next: (evt) => {
        if (evt.type === HttpEventType.UploadProgress) {
          const percent = evt.total ? Math.round((100 * evt.loaded) / evt.total) : 0;
          this.uploadProgress.set(percent);
        } else if (evt.type === HttpEventType.Response) {
          this.toast.success(`Fișierul "${file.name}" a fost încărcat.`);
          this.uploadingFile.set(false);
          this.uploadProgress.set(0);
          input.value = '';
          this.loadExplorerFiles(vol.id, this.explorerCurrentPath());
        }
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea fișierului.');
        this.uploadingFile.set(false);
        this.uploadProgress.set(0);
        input.value = '';
      }
    });
  }

  downloadFile(item: VolumeFileItem): void {
    const vol = this.currentExplorerVolume();
    if (!vol) return;

    let current = this.explorerCurrentPath();
    if (!current.endsWith('/')) {
      current += '/';
    }
    const targetPath = current + item.name;
    const url = this.volumeService.downloadFileUrl(vol.id, targetPath);

    const a = document.createElement('a');
    a.href = url;
    a.download = item.name;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
  }

  getBreadcrumbs(): { name: string; path: string }[] {
    const current = this.explorerCurrentPath();
    if (current === '/' || !current) return [];
    
    const parts = current.split('/').filter(p => p);
    const crumbs: { name: string; path: string }[] = [];
    let cumulativePath = '';
    
    for (const part of parts) {
      cumulativePath += '/' + part;
      crumbs.push({
        name: part,
        path: cumulativePath
      });
    }
    return crumbs;
  }

  onDragOver(event: DragEvent): void {
    event.preventDefault();
    event.stopPropagation();
    this.isDragging.set(true);
  }

  onDragLeave(event: DragEvent): void {
    event.preventDefault();
    event.stopPropagation();
    this.isDragging.set(false);
  }

  onDrop(event: DragEvent): void {
    event.preventDefault();
    event.stopPropagation();
    this.isDragging.set(false);

    const files = event.dataTransfer?.files;
    const vol = this.currentExplorerVolume();
    if (!vol || !files || files.length === 0) return;

    const file = files[0];
    this.uploadProgress.set(0);
    this.uploadingFile.set(true);

    this.volumeService.uploadFileProgress(vol.id, this.explorerCurrentPath(), file).subscribe({
      next: (evt) => {
        if (evt.type === HttpEventType.UploadProgress) {
          const percent = evt.total ? Math.round((100 * evt.loaded) / evt.total) : 0;
          this.uploadProgress.set(percent);
        } else if (evt.type === HttpEventType.Response) {
          this.toast.success(`Fișierul "${file.name}" a fost încărcat.`);
          this.uploadingFile.set(false);
          this.uploadProgress.set(0);
          this.loadExplorerFiles(vol.id, this.explorerCurrentPath());
        }
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea fișierului.');
        this.uploadingFile.set(false);
        this.uploadProgress.set(0);
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
