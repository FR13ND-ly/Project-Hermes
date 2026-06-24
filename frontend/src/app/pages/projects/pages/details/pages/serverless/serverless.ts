import { Component, inject, signal, computed, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ActivatedRoute } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, ServerlessInstance, ServerlessRoute, ProjectEnvResponse, ServerlessBuild, FunctionEnvResponse } from '../../../../../../core/services/project.service';
import { DomainService, Domain } from '../../../../../../core/services/domain.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';
import { environment } from '../../../../../../../environments/environment';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

declare const monaco: any;

@Component({
  selector: 'app-project-serverless',
  standalone: true,
  imports: [CommonModule, FormsModule, Pagination],
  templateUrl: './serverless.html',
})
export class ServerlessComponent implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly domainService = inject(DomainService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly wsService = inject(WebSocketService);
  private readonly route = inject(ActivatedRoute);

  readonly loading = signal(false);
  readonly instances = signal<ServerlessInstance[]>([]);
  readonly page = signal(1);
  readonly pageSize = signal(DEFAULT_PAGE_SIZE);
  readonly total = signal(0);
  readonly selectedInstance = signal<ServerlessInstance | null>(null);

  readonly activeTab = signal<'details' | 'routes' | 'settings' | 'env' | 'metrics' | 'builds' | 'logs'>('details');

  // Telemetry (historical, Prometheus)
  readonly metricsRange = signal('1h');
  readonly metricsLoading = signal(false);
  readonly metricsSimulated = signal(false);
  readonly cpuValues = signal<number[]>([]);
  readonly memValues = signal<number[]>([]);
  readonly domains = signal<Domain[]>([]);
  readonly logs = signal<string[]>([]);
  readonly builds = signal<ServerlessBuild[]>([]);
  readonly selectedBuildId = signal<string | null>(null);
  readonly buildLogs = signal<string[]>([]);

  // Create instance modal
  readonly showCreateModal = signal(false);
  readonly creating = signal(false);
  readonly newName = signal('');
  readonly newRuntime = signal('nodejs-cjs');
  readonly newMemory = signal(0); // 0 = unlimited (no forced initial limit)

  // Register domain modal
  readonly showAddDomainModal = signal(false);
  readonly newDomainFqdn = signal('');
  readonly addingDomain = signal(false);

  // Settings (instance-level)
  readonly editName = signal('');
  readonly editRuntime = signal('nodejs-cjs');
  readonly editMemory = signal(0);
  readonly editAssignedDomain = signal<string | null>(null);
  readonly savingSettings = signal(false);
  readonly deploying = signal(false);

  // Routes
  readonly routes = signal<ServerlessRoute[]>([]);
  readonly selectedRoute = signal<ServerlessRoute | null>(null);
  readonly routeMethod = signal('GET');
  readonly routePath = signal('');
  readonly routeCode = signal('');
  readonly savingRoute = signal(false);

  // Env (instance-level)
  readonly functionEnv = signal<FunctionEnvResponse[]>([]);
  readonly revealedEnvIds = signal<Record<string, boolean>>({});
  readonly availableProjectEnv = signal<ProjectEnvResponse[]>([]);
  readonly togglingLinkId = signal<string | null>(null);
  readonly linkedProjectEnv = computed(() => this.availableProjectEnv().filter(v => v.linked));
  readonly showAddEnvPanel = signal(false);
  readonly addEnvMode = signal<'new' | 'project'>('new');
  readonly newEnvKey = signal('');
  readonly newEnvValue = signal('');
  readonly newEnvIsSecret = signal(true);
  readonly editingEnvId = signal<string | null>(null);
  readonly savingEnv = signal(false);
  readonly reloadingEnv = signal(false);

  editorInstance: any = null;
  private logSource: EventSource | null = null;
  private buildLogSource: EventSource | null = null;
  private wsSubscriptions = new Subscription();

  readonly httpMethods = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'ANY'];

  constructor() {
    effect(() => {
      const projId = this.parent.projectId();
      if (projId) {
        this.loadInstances();
        this.loadDomains();
      }
    });
  }

  ngOnInit(): void {
    this.loadInstances();
    this.loadDomains();
    for (const evt of ['serverless_function_updated', 'serverless_function_deleted']) {
      this.wsSubscriptions.add(this.wsService.onEvent<any>(evt).subscribe(() => {
        this.loadInstances();
        const sel = this.selectedInstance();
        if (sel) {
          this.projectService.getFunctionDetails(this.parent.projectId()!, sel.id).subscribe({
            next: (fresh) => this.selectedInstance.set(fresh),
            error: () => {}
          });
        }
      }));
    }
  }

  ngOnDestroy(): void {
    this.stopLogsStream();
    this.stopBuildLogsStream();
    this.wsSubscriptions.unsubscribe();
    if (this.editorInstance) { this.editorInstance.dispose(); this.editorInstance = null; }
  }

  // ---------- Instances ----------
  loadInstances(): void {
    const projId = this.parent.projectId();
    if (!projId) return;
    this.loading.set(true);
    this.projectService.listProjectFunctions(projId, this.page(), this.pageSize()).subscribe({
      next: (res) => {
        this.instances.set(res?.items || []);
        this.total.set(res?.total || 0);
        this.loading.set(false);
      },
      error: () => { this.instances.set([]); this.loading.set(false); }
    });
  }

  onPageChange(p: number): void { this.page.set(p); this.loadInstances(); }

  loadDomains(): void {
    // Scope to this project so other projects' domains don't leak in here.
    const projectId = this.parent.projectId();
    this.domainService.listDomains(1, 1000, projectId || undefined).subscribe({
      next: (res) => this.domains.set(res?.items || []),
      error: () => {}
    });
  }

  selectInstance(inst: ServerlessInstance): void {
    this.stopLogsStream();
    this.stopBuildLogsStream();
    this.selectedInstance.set(inst);
    this.activeTab.set('details');

    this.editName.set(inst.name);
    this.editRuntime.set(inst.runtime || 'nodejs-cjs');
    this.editMemory.set(inst.memoryLimitMb);
    this.editAssignedDomain.set(inst.assignedDomain || null);

    this.routes.set(inst.routes || []);
    this.selectedRoute.set(null);
    this.loadRoutes();

    this.functionEnv.set([]);
    this.revealedEnvIds.set({});
    this.loadFunctionEnv();
    this.availableProjectEnv.set([]);
    this.loadFunctionProjectEnv();

    this.selectedBuildId.set(null);
    this.buildLogs.set([]);
    this.loadBuilds();
  }

  deselectInstance(): void {
    this.stopLogsStream();
    this.stopBuildLogsStream();
    if (this.editorInstance) { this.editorInstance.dispose(); this.editorInstance = null; }
    this.selectedInstance.set(null);
  }

  openCreateModal(): void {
    this.newName.set('');
    this.newRuntime.set('nodejs-cjs');
    this.newMemory.set(0);
    this.showCreateModal.set(true);
  }

  onCreateInstance(): void {
    const projId = this.parent.projectId();
    if (!projId) return;
    const name = this.newName().trim();
    if (!name) { this.toast.error('Numele instanței este obligatoriu.'); return; }
    this.creating.set(true);
    this.projectService.createInstance(projId, { name, runtime: this.newRuntime(), memoryLimitMb: this.newMemory() }).subscribe({
      next: (res) => {
        this.creating.set(false);
        this.showCreateModal.set(false);
        this.toast.success('Instanța serverless a fost creată.');
        this.loadInstances();
        this.selectInstance(res);
      },
      error: (err) => { this.creating.set(false); this.toast.error(err.error?.message || 'Eroare la creare.'); }
    });
  }

  onSaveSettings(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    const name = this.editName().trim();
    if (!name) { this.toast.error('Numele este obligatoriu.'); return; }
    this.savingSettings.set(true);
    this.projectService.updateInstance(projId, inst.id, {
      name,
      runtime: this.editRuntime(),
      memoryLimitMb: this.editMemory(),
      assignedDomain: this.editAssignedDomain() || null,
    }).subscribe({
      next: (updated) => {
        this.savingSettings.set(false);
        this.selectedInstance.set(updated);
        this.routes.set(updated.routes || []);
        this.toast.success('Setările instanței au fost salvate.');
        this.loadInstances();
      },
      error: (err) => { this.savingSettings.set(false); this.toast.error(err.error?.message || 'Eroare la salvare.'); }
    });
  }

  onDeployInstance(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    if ((this.routes() || []).length === 0) {
      this.toast.error('Adaugă cel puțin o rută înainte de Deploy.');
      return;
    }
    this.deploying.set(true);
    this.projectService.deployFunction(projId, inst.id).subscribe({
      next: (res) => {
        this.deploying.set(false);
        this.toast.success('Compilarea și deployment-ul au fost inițiate!');
        const cur = this.selectedInstance();
        if (cur) this.selectedInstance.set({ ...cur, status: 'building' });
        this.loadInstances();
        if (res?.buildId) {
          this.activeTab.set('builds');
          this.loadBuilds();
          this.selectBuild({ id: res.buildId, status: 'building' } as ServerlessBuild);
        }
      },
      error: (err) => { this.deploying.set(false); this.toast.error(err.error?.message || 'Eroare la deploy.'); }
    });
  }

  async onDeleteInstance(): Promise<void> {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    const confirmed = await this.confirm.ask({
      title: 'Ștergere instanță serverless',
      message: `Sigur ștergi instanța „${inst.name}"? Toate rutele și resursele K8s asociate vor fi eliminate.`,
      confirmText: 'Șterge', cancelText: 'Anulează', isDanger: true,
    });
    if (!confirmed) return;
    this.projectService.deleteFunction(projId, inst.id).subscribe({
      next: () => { this.toast.success('Instanță ștearsă.'); this.deselectInstance(); this.loadInstances(); },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la ștergere.')
    });
  }

  // ---------- Tabs ----------
  onTabChange(tab: 'details' | 'routes' | 'settings' | 'env' | 'metrics' | 'builds' | 'logs'): void {
    this.activeTab.set(tab);
    if (tab === 'logs') this.startLogsStream();
    else this.stopLogsStream();
    if (tab === 'metrics') this.loadMetrics();
    if (tab === 'routes' && this.selectedRoute()) {
      setTimeout(() => this.mountEditor(), 50);
    }
  }

  // ---------- Telemetry ----------
  loadMetrics(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    const range = this.metricsRange();
    this.metricsLoading.set(true);
    this.projectService.getInstanceMetrics(projId, inst.id, 'cpu', range).subscribe({
      next: (res) => {
        this.cpuValues.set((res.values || []).map(v => v * 1000)); // cores -> millicores
        this.metricsSimulated.set(!!res.simulated);
        this.metricsLoading.set(false);
      },
      error: () => { this.cpuValues.set([]); this.metricsLoading.set(false); }
    });
    this.projectService.getInstanceMetrics(projId, inst.id, 'memory', range).subscribe({
      next: (res) => this.memValues.set(res.values || []),
      error: () => this.memValues.set([])
    });
  }

  onMetricsRangeChange(range: string): void { this.metricsRange.set(range); this.loadMetrics(); }

  lastVal(values: number[]): number { return values.length > 0 ? values[values.length - 1] : 0; }

  getSvgPath(values: number[]): string {
    if (values.length < 2) return '';
    const width = 500, height = 150;
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

  // ---------- Routes ----------
  loadRoutes(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listRoutes(projId, inst.id).subscribe({
      next: (res) => this.routes.set(res || []),
      error: () => {}
    });
  }

  onAddRoute(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    // Create a blank route, then open it in the editor.
    this.projectService.createRoute(projId, inst.id, { method: 'GET', routePath: '/noua-ruta-' + (this.routes().length + 1) }).subscribe({
      next: (r) => { this.toast.success('Rută adăugată. Editeaz-o și apoi Deploy.'); this.loadRoutes(); this.selectRoute(r); },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la adăugarea rutei.')
    });
  }

  selectRoute(r: ServerlessRoute): void {
    this.selectedRoute.set(r);
    this.routeMethod.set(r.method);
    this.routePath.set(r.routePath);
    this.routeCode.set(r.code);
    setTimeout(() => this.mountEditor(), 50);
  }

  closeRouteEditor(): void {
    this.selectedRoute.set(null);
    if (this.editorInstance) { this.editorInstance.dispose(); this.editorInstance = null; }
  }

  onSaveRoute(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    const r = this.selectedRoute();
    if (!projId || !inst || !r) return;
    const routePath = this.routePath().trim();
    if (!routePath) { this.toast.error('Calea rutei este obligatorie.'); return; }
    this.savingRoute.set(true);
    this.projectService.updateRoute(projId, inst.id, r.id, {
      method: this.routeMethod(),
      routePath,
      code: this.editorInstance ? this.editorInstance.getValue() : this.routeCode(),
    }).subscribe({
      next: (updated) => {
        this.savingRoute.set(false);
        this.selectedRoute.set(updated);
        this.toast.success('Rută salvată. Lansează (Deploy) pentru a aplica.');
        this.loadRoutes();
      },
      error: (err) => { this.savingRoute.set(false); this.toast.error(err.error?.message || 'Eroare la salvarea rutei.'); }
    });
  }

  async onDeleteRoute(r: ServerlessRoute): Promise<void> {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    const confirmed = await this.confirm.ask({
      title: 'Ștergere rută',
      message: `Sigur ștergi ruta ${r.method} ${r.routePath}?`,
      confirmText: 'Șterge', cancelText: 'Anulează', isDanger: true,
    });
    if (!confirmed) return;
    this.projectService.deleteRoute(projId, inst.id, r.id).subscribe({
      next: () => {
        this.toast.success('Rută ștearsă.');
        if (this.selectedRoute()?.id === r.id) this.closeRouteEditor();
        this.loadRoutes();
      },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la ștergere.')
    });
  }

  invokeUrl(r: ServerlessRoute): string {
    const inst = this.selectedInstance();
    if (!inst) return '';
    const base = inst.assignedDomain ? `https://${inst.assignedDomain}` : (inst.externalPort ? `http://localhost:${inst.externalPort}` : '');
    return base + r.routePath;
  }

  // ---------- Domain ----------
  onOpenAddDomainModal(): void { this.newDomainFqdn.set(''); this.showAddDomainModal.set(true); }

  onRegisterDomain(): void {
    const inst = this.selectedInstance();
    if (!inst) return;
    const fqdn = this.newDomainFqdn().trim().toLowerCase();
    if (!fqdn) { this.toast.error('Numele de domeniu este obligatoriu.'); return; }
    this.addingDomain.set(true);
    this.domainService.addDomain({ fqdn, targetType: 'serverless', targetId: inst.id, routingType: 'reverse_proxy', isSsl: true }).subscribe({
      next: (domain) => {
        this.addingDomain.set(false);
        this.showAddDomainModal.set(false);
        this.toast.success(`Domeniul ${fqdn} a fost înregistrat!`);
        this.loadDomains();
        this.editAssignedDomain.set(domain.fqdn);
      },
      error: (err) => { this.addingDomain.set(false); this.toast.error(err.error?.message || 'Eroare la înregistrarea domeniului.'); }
    });
  }

  // ---------- Env (instance-level) ----------
  loadFunctionEnv(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listFunctionEnv(projId, inst.id).subscribe({
      next: (res) => this.functionEnv.set(res || []),
      error: () => this.functionEnv.set([])
    });
  }

  openAddEnvPanel(mode: 'new' | 'project'): void {
    this.addEnvMode.set(mode);
    this.showAddEnvPanel.set(true);
    this.editingEnvId.set(null);
    this.newEnvKey.set('');
    this.newEnvValue.set('');
    this.newEnvIsSecret.set(true);
  }

  closeAddEnvPanel(): void {
    this.showAddEnvPanel.set(false);
    this.editingEnvId.set(null);
    this.newEnvKey.set('');
    this.newEnvValue.set('');
    this.newEnvIsSecret.set(true);
  }

  startEditEnv(env: FunctionEnvResponse): void {
    this.addEnvMode.set('new');
    this.showAddEnvPanel.set(true);
    this.editingEnvId.set(env.id);
    this.newEnvKey.set(env.key);
    this.newEnvValue.set(env.isSecret ? '' : (env.value ?? ''));
    this.newEnvIsSecret.set(env.isSecret);
    setTimeout(() => document.getElementById('fn-env-form-panel')?.scrollIntoView({ behavior: 'smooth', block: 'center' }), 0);
  }

  onAddFunctionEnv(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    const key = this.newEnvKey().trim().toUpperCase();
    if (!key) { this.toast.error('Cheia este obligatorie.'); return; }
    this.savingEnv.set(true);
    this.projectService.setFunctionEnv(projId, inst.id, { key, value: this.newEnvValue(), isSecret: this.newEnvIsSecret() }).subscribe({
      next: () => {
        this.savingEnv.set(false);
        this.closeAddEnvPanel();
        this.toast.success('Variabilă salvată. Lansează (Deploy) sau „Reload variabile".');
        this.loadFunctionEnv();
      },
      error: (err) => { this.savingEnv.set(false); this.toast.error(err.error?.message || 'Eroare la salvare.'); }
    });
  }

  async onDeleteFunctionEnv(env: FunctionEnvResponse): Promise<void> {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    const confirmed = await this.confirm.ask({
      title: 'Ștergere variabilă',
      message: `Sigur ștergi „${env.key}"?`, confirmText: 'Șterge', cancelText: 'Anulează', isDanger: true,
    });
    if (!confirmed) return;
    this.projectService.deleteFunctionEnv(projId, inst.id, env.id).subscribe({
      next: () => { this.toast.success('Variabilă ștearsă.'); this.loadFunctionEnv(); },
      error: (err) => this.toast.error(err.error?.message || 'Eroare la ștergere.')
    });
  }

  toggleRevealEnv(id: string): void { this.revealedEnvIds.update(m => ({ ...m, [id]: !m[id] })); }
  copyToClipboard(text: string): void { navigator.clipboard.writeText(text).then(() => this.toast.success('Copiat!')); }

  onReloadFunctionEnv(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.reloadingEnv.set(true);
    this.projectService.reloadFunctionEnv(projId, inst.id).subscribe({
      next: (updated) => { this.reloadingEnv.set(false); this.selectedInstance.set(updated); this.toast.success('Variabile reaplicate fără recompilare.'); },
      error: (err) => { this.reloadingEnv.set(false); this.toast.error(err.error?.message || 'Eroare la reload.'); }
    });
  }

  loadFunctionProjectEnv(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listFunctionProjectEnv(projId, inst.id).subscribe({
      next: (res) => this.availableProjectEnv.set(res || []),
      error: () => this.availableProjectEnv.set([])
    });
  }

  onToggleProjectEnvLink(penv: ProjectEnvResponse): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.togglingLinkId.set(penv.id);
    const op = penv.linked
      ? this.projectService.unlinkFunctionProjectEnv(projId, inst.id, penv.id)
      : this.projectService.linkFunctionProjectEnv(projId, inst.id, penv.id);
    op.subscribe({
      next: () => { this.togglingLinkId.set(null); this.loadFunctionProjectEnv(); this.toast.success(penv.linked ? 'Deconectată din pool.' : 'Conectată din pool. Deploy/Reload pentru a aplica.'); },
      error: (err) => { this.togglingLinkId.set(null); this.toast.error(err.error?.message || 'Eroare la actualizarea legăturii.'); }
    });
  }

  // ---------- Builds + logs ----------
  loadBuilds(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listFunctionBuilds(projId, inst.id).subscribe({
      next: (res) => this.builds.set(res || []),
      error: () => this.builds.set([])
    });
  }

  selectBuild(build: ServerlessBuild): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.stopBuildLogsStream();
    this.selectedBuildId.set(build.id);
    this.buildLogs.set(['[Console] Conectare la fluxul de loguri de compilare...']);
    const url = this.projectService.getFunctionBuildLogsStreamUrl(projId, inst.id, build.id);
    this.buildLogSource = new EventSource(url);
    this.buildLogSource.onmessage = (event) => {
      this.buildLogs.update(lines => { const next = [...lines, event.data]; if (next.length > 1000) next.shift(); return next; });
    };
    this.buildLogSource.onerror = () => { this.stopBuildLogsStream(); this.loadBuilds(); };
  }

  private stopBuildLogsStream(): void { if (this.buildLogSource) { this.buildLogSource.close(); this.buildLogSource = null; } }

  private startLogsStream(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    this.stopLogsStream();
    this.logs.set(['[Console] Conectare la fluxul de loguri...']);
    const url = `${environment.apiBaseUrl}/projects/${projId}/serverless/${inst.id}/logs/stream?token=${encodeURIComponent(localStorage.getItem('hermes_token') || '')}`;
    this.logSource = new EventSource(url);
    this.logSource.onmessage = (event) => {
      this.logs.update(lines => { const next = [...lines, event.data]; if (next.length > 500) next.shift(); return next; });
    };
    this.logSource.onerror = () => { /* keep open; Knative scale-to-zero is normal */ };
  }

  private stopLogsStream(): void { if (this.logSource) { this.logSource.close(); this.logSource = null; } }

  // ---------- Monaco (route code editor) ----------
  private mountEditor(): void {
    if (typeof monaco !== 'undefined') { this.createEditor(); return; }
    const loaderUrl = 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.48.0/min/vs/loader.min.js';
    if (!document.querySelector(`script[src="${loaderUrl}"]`)) {
      const script = document.createElement('script');
      script.src = loaderUrl;
      script.onload = () => {
        const req = (window as any).require;
        req.config({ paths: { vs: 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.48.0/min/vs' } });
        req(['vs/editor/editor.main'], () => this.createEditor());
      };
      document.body.appendChild(script);
    } else {
      const i = setInterval(() => { if (typeof monaco !== 'undefined') { clearInterval(i); this.createEditor(); } }, 100);
    }
  }

  private createEditor(): void {
    const container = document.getElementById('route-editor-container');
    if (!container) return;
    if (this.editorInstance) { this.editorInstance.dispose(); this.editorInstance = null; }
    const language = this.editRuntime().startsWith('python') ? 'python' : 'javascript';
    this.editorInstance = monaco.editor.create(container, {
      value: this.routeCode(),
      language,
      theme: 'vs-dark',
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 12,
      fontFamily: 'Fira Code, JetBrains Mono, monospace',
      lineHeight: 20,
      tabSize: 2,
    });
    this.editorInstance.onDidChangeModelContent(() => this.routeCode.set(this.editorInstance.getValue()));
  }
}
