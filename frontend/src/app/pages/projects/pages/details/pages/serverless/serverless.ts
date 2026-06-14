import { Component, inject, signal, computed, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { ProjectService, ServerlessFunction, ProjectEnvResponse, ServerlessBuild } from '../../../../../../core/services/project.service';
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

  readonly loading = signal(false);
  readonly functions = signal<ServerlessFunction[]>([]);
  readonly page = signal(1);
  readonly pageSize = signal(DEFAULT_PAGE_SIZE);
  readonly total = signal(0);
  readonly selectedFunction = signal<ServerlessFunction | null>(null);
  readonly activeTab = signal<'details' | 'code' | 'settings' | 'env' | 'builds' | 'logs'>('details');
  readonly domains = signal<Domain[]>([]);
  readonly logs = signal<string[]>([]);

  // Build history + live build (Kaniko) logs
  readonly builds = signal<ServerlessBuild[]>([]);
  readonly selectedBuildId = signal<string | null>(null);
  readonly buildLogs = signal<string[]>([]);

  // Creation Modal Signals
  readonly showCreateModal = signal(false);
  readonly creating = signal(false);
  readonly newFnName = signal('');
  readonly newFnMethod = signal('GET');
  readonly newFnRoutePath = signal('');
  readonly newFnMemory = signal(0);
  readonly newFnRuntime = signal('nodejs-cjs');

  // Register Domain Modal Signals
  readonly showAddDomainModal = signal(false);
  readonly newDomainFqdn = signal('');
  readonly addingDomain = signal(false);

  // Edit / Settings Signals
  readonly editName = signal('');
  readonly editCode = signal('');
  readonly editMethod = signal('GET');
  readonly editRoutePath = signal('');
  readonly editMemory = signal(0);
  readonly editAssignedDomain = signal<string | null>(null);
  readonly editEnvVariables = signal<{ key: string, value: string }[]>([]);
  // Project-pool vars available to this function, each flagged linked/not (parity with apps).
  readonly availableProjectEnv = signal<ProjectEnvResponse[]>([]);
  readonly togglingLinkId = signal<string | null>(null);
  readonly linkedProjectEnv = computed(() => this.availableProjectEnv().filter(v => v.linked));
  // "Adaugă Variabilă" panel (mirror app-detail): a brand-new var or one from the pool.
  readonly showAddEnvPanel = signal(false);
  readonly addEnvMode = signal<'new' | 'project'>('new');
  readonly newEnvKey = signal('');
  readonly newEnvValue = signal('');
  readonly reloadingEnv = signal(false);
  readonly editRuntime = signal('nodejs-cjs');
  readonly editInheritProjectEnvs = signal(false);
  readonly savingSettings = signal(false);
  readonly deploying = signal(false);

  editorInstance: any = null;
  private logSource: EventSource | null = null;
  private buildLogSource: EventSource | null = null;
  private wsSubscriptions = new Subscription();

  constructor() {
    effect(() => {
      const projId = this.parent.projectId();
      if (projId) {
        this.loadFunctions();
        this.loadDomains();
      }
    });
  }

  ngOnInit(): void {
    this.loadFunctions();
    this.loadDomains();
    this.setupWsSubscriptions();
  }

  ngOnDestroy(): void {
    this.stopLogsStream();
    this.stopBuildLogsStream();
    if (this.editorInstance) {
      this.editorInstance.dispose();
      this.editorInstance = null;
    }
    this.wsSubscriptions.unsubscribe();
  }

  loadFunctions(): void {
    const projId = this.parent.projectId();
    if (!projId) return;

    this.loading.set(true);
    this.projectService.listProjectFunctions(projId, this.page(), this.pageSize()).subscribe({
      next: (res) => {
        this.functions.set(res?.items || []);
        this.total.set(res?.total || 0);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea funcțiilor serverless.');
        this.loading.set(false);
      }
    });
  }

  onPageChange(page: number): void {
    this.page.set(page);
    this.loadFunctions();
  }

  loadDomains(): void {
    // Secondary list (attach targets) — fetch a large page to keep all options.
    this.domainService.listDomains(1, 1000).subscribe({
      next: (res) => {
        this.domains.set(res?.items || []);
      },
      error: (err) => {
        console.error('Failed to load domains:', err);
      }
    });
  }

  onOpenCreateModal(): void {
    this.newFnName.set('');
    this.newFnMethod.set('GET');
    this.newFnRoutePath.set('');
    this.newFnMemory.set(128);
    this.newFnRuntime.set('nodejs-cjs');
    this.showCreateModal.set(true);
  }

  onOpenAddDomainModal(): void {
    this.newDomainFqdn.set('');
    this.showAddDomainModal.set(true);
  }

  onCloseAddDomainModal(): void {
    this.showAddDomainModal.set(false);
  }

  onRegisterDomain(): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    const fqdn = this.newDomainFqdn().trim().toLowerCase();
    if (!fqdn) {
      this.toast.error('Numele de domeniu este obligatoriu.');
      return;
    }

    this.addingDomain.set(true);
    this.domainService.addDomain({
      fqdn,
      targetType: 'serverless',
      targetId: fn.id,
      routingType: 'reverse_proxy',
      isSsl: true,
    }).subscribe({
      next: (domain) => {
        this.addingDomain.set(false);
        this.showAddDomainModal.set(false);
        this.toast.success(`Domeniul ${fqdn} a fost înregistrat cu succes!`);
        this.loadDomains();
        this.editAssignedDomain.set(domain.fqdn);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la înregistrarea domeniului.');
        this.addingDomain.set(false);
      }
    });
  }

  onCreateFunction(): void {
    const projId = this.parent.projectId();
    if (!projId) return;

    const name = this.newFnName().trim();
    const method = this.newFnMethod();
    let routePath = this.newFnRoutePath().trim();
    const memory = this.newFnMemory();

    if (!name || !routePath) {
      this.toast.error('Numele și calea de rută sunt obligatorii.');
      return;
    }

    if (!routePath.startsWith('/')) {
      routePath = '/' + routePath;
    }

    this.creating.set(true);
    this.projectService.createFunction(projId, {
      name,
      method,
      routePath,
      memoryLimitMb: memory,
      runtime: this.newFnRuntime()
    }).subscribe({
      next: (res) => {
        this.creating.set(false);
        this.showCreateModal.set(false);
        this.toast.success('Funcția serverless a fost creată cu succes!');
        this.loadFunctions();
        this.selectFunction(res);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la crearea funcției.');
        this.creating.set(false);
      }
    });
  }

  selectFunction(fn: ServerlessFunction): void {
    this.stopLogsStream();
    this.selectedFunction.set(fn);
    this.activeTab.set('details');
    this.logs.set([]);

    // Populate edit fields
    this.editName.set(fn.name);
    this.editCode.set(fn.code);
    this.editMethod.set(fn.method);
    this.editRoutePath.set(fn.routePath);
    this.editMemory.set(fn.memoryLimitMb);
    this.editAssignedDomain.set(fn.assignedDomain || null);
    this.editRuntime.set(fn.runtime || 'nodejs-cjs');
    this.editInheritProjectEnvs.set(fn.inheritProjectEnvs || false);

    // Update editor value if initialized
    if (this.editorInstance) {
      this.editorInstance.setValue(fn.code);
      const model = this.editorInstance.getModel();
      if (model && typeof monaco !== 'undefined') {
        const lang = fn.runtime?.startsWith('python') ? 'python' : 'javascript';
        monaco.editor.setModelLanguage(model, lang);
      }
    }

    // Parse environment variables
    const envs = fn.envVariables || [];
    const formattedEnvs = Array.isArray(envs) ? envs.map((e: any) => ({
      key: e.key || '',
      value: e.value || ''
    })) : [];
    this.editEnvVariables.set(formattedEnvs);

    // Load the project pool (with linked flags) for the "associate from pool" panel.
    this.availableProjectEnv.set([]);
    this.loadFunctionProjectEnv();

    // Build history for the "Build-uri" tab.
    this.stopBuildLogsStream();
    this.selectedBuildId.set(null);
    this.buildLogs.set([]);
    this.loadBuilds();
  }

  deselectFunction(): void {
    this.stopLogsStream();
    this.stopBuildLogsStream();
    if (this.editorInstance) {
      this.editorInstance.dispose();
      this.editorInstance = null;
    }
    this.selectedFunction.set(null);
  }

  onTabChange(tab: 'details' | 'code' | 'settings' | 'env' | 'builds' | 'logs'): void {
    this.activeTab.set(tab);
    if (tab === 'code') {
      setTimeout(() => {
        this.initMonaco();
      }, 50);
    } else {
      this.stopLogsStream();
    }
    if (tab === 'logs') {
      this.startLogsStream();
    }
    if (tab === 'builds') {
      this.loadBuilds();
    } else {
      this.stopBuildLogsStream();
    }
  }

  onSaveFunction(): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    const name = this.editName().trim();
    const code = this.editCode();
    const method = this.editMethod();
    let routePath = this.editRoutePath().trim();
    const memory = this.editMemory();
    const assignedDomain = this.editAssignedDomain();
    
    // Filter and clean env variables
    const envVariables = this.editEnvVariables()
      .map(v => ({ key: v.key.trim().toUpperCase(), value: v.value.trim() }))
      .filter(v => v.key !== '');

    if (!name || !routePath) {
      this.toast.error('Numele și calea de rută sunt obligatorii.');
      return;
    }

    if (!routePath.startsWith('/')) {
      routePath = '/' + routePath;
    }

    this.savingSettings.set(true);
    this.projectService.updateFunction(projId, fn.id, {
      name,
      code,
      method,
      routePath,
      memoryLimitMb: memory,
      assignedDomain: assignedDomain || null,
      envVariables,
      runtime: this.editRuntime(),
      inheritProjectEnvs: false
    }).subscribe({
      next: (updated) => {
        this.toast.success('Configurațiile funcției au fost salvate cu succes!');
        this.selectedFunction.set(updated);
        this.savingSettings.set(false);
        this.loadFunctions();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea setărilor.');
        this.savingSettings.set(false);
      }
    });
  }

  onDeployFunction(): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    // First save the current code and settings to draft
    const name = this.editName().trim();
    const code = this.editCode();
    const method = this.editMethod();
    let routePath = this.editRoutePath().trim();
    const memory = this.editMemory();
    const assignedDomain = this.editAssignedDomain();
    const envVariables = this.editEnvVariables()
      .map(v => ({ key: v.key.trim().toUpperCase(), value: v.value.trim() }))
      .filter(v => v.key !== '');

    if (!name || !routePath) {
      this.toast.error('Numele și calea de rută sunt obligatorii.');
      return;
    }

    if (!routePath.startsWith('/')) {
      routePath = '/' + routePath;
    }

    this.deploying.set(true);
    this.projectService.updateFunction(projId, fn.id, {
      name,
      code,
      method,
      routePath,
      memoryLimitMb: memory,
      assignedDomain: assignedDomain || null,
      envVariables,
      runtime: this.editRuntime(),
      inheritProjectEnvs: false
    }).subscribe({
      next: (updated) => {
        this.selectedFunction.set(updated);
        
        // Trigger deploy
        this.projectService.deployFunction(projId, fn.id).subscribe({
          next: (res) => {
            this.toast.success('Compilarea și deployment-ul funcției au fost inițiate!');
            this.deploying.set(false);

            // Set local status to building
            const current = this.selectedFunction();
            if (current) {
              this.selectedFunction.set({
                ...current,
                status: 'building'
              });
            }
            this.loadFunctions();

            // Jump to the Build-uri tab and stream the new build's Kaniko logs live.
            if (res?.buildId) {
              this.activeTab.set('builds');
              this.loadBuilds();
              this.selectBuild({ id: res.buildId, status: 'building' });
            }
          },
          error: (err) => {
            this.toast.error(err.error?.message || 'Eroare la inițierea deployment-ului.');
            this.deploying.set(false);
          }
        });
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la salvarea codului înainte de deploy.');
        this.deploying.set(false);
      }
    });
  }

  async onDeleteFunction(): Promise<void> {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Funcție Serverless',
      message: `Sigur doriți să ștergeți funcția "${fn.name}"? Serviciul asociat din Kubernetes va fi eliminat complet. Această acțiune este ireversibilă!`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true,
      matchText: fn.name
    });

    if (!confirmed) return;

    this.projectService.deleteFunction(projId, fn.id).subscribe({
      next: () => {
        this.toast.success('Funcția serverless a fost ștearsă.');
        this.deselectFunction();
        this.loadFunctions();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea funcției.');
      }
    });
  }

  copyBuildLogs(): void {
    const logs = this.selectedFunction()?.buildLogs;
    if (!logs) return;
    navigator.clipboard.writeText(logs).then(() => {
      this.toast.success('Logurile de compilare au fost copiate în clipboard!');
    });
  }

  // Env vars helper actions
  addEnvVar(): void {
    this.editEnvVariables.update(vars => [...vars, { key: '', value: '' }]);
  }

  removeEnvVar(index: number): void {
    this.editEnvVariables.update(vars => vars.filter((_, i) => i !== index));
  }

  openAddEnvPanel(mode: 'new' | 'project'): void {
    this.addEnvMode.set(mode);
    this.showAddEnvPanel.set(true);
    this.newEnvKey.set('');
    this.newEnvValue.set('');
  }

  onAddManualEnv(): void {
    const key = this.newEnvKey().trim().toUpperCase();
    if (!key) {
      this.toast.error('Cheia este obligatorie.');
      return;
    }
    this.editEnvVariables.update(vars => [...vars, { key, value: this.newEnvValue().trim() }]);
    this.newEnvKey.set('');
    this.newEnvValue.set('');
    this.showAddEnvPanel.set(false);
    this.toast.success('Variabilă adăugată. Apasă „Salvează Variabilele" pentru a persista.');
  }

  onReloadFunctionEnv(): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;
    this.reloadingEnv.set(true);
    this.projectService.reloadFunctionEnv(projId, fn.id).subscribe({
      next: (updated) => {
        this.reloadingEnv.set(false);
        this.selectedFunction.set(updated);
        this.toast.success('Variabilele au fost reaplicate fără recompilare.');
      },
      error: (err) => {
        this.reloadingEnv.set(false);
        this.toast.error(err.error?.message || 'Eroare la reîncărcarea variabilelor.');
      }
    });
  }

  // --- Project-pool linking (parity with apps) ---
  loadFunctionProjectEnv(): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;
    this.projectService.listFunctionProjectEnv(projId, fn.id).subscribe({
      next: (res) => this.availableProjectEnv.set(res || []),
      error: () => this.availableProjectEnv.set([])
    });
  }

  onToggleProjectEnvLink(penv: ProjectEnvResponse): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;
    this.togglingLinkId.set(penv.id);
    const op = penv.linked
      ? this.projectService.unlinkFunctionProjectEnv(projId, fn.id, penv.id)
      : this.projectService.linkFunctionProjectEnv(projId, fn.id, penv.id);
    op.subscribe({
      next: () => {
        this.togglingLinkId.set(null);
        this.loadFunctionProjectEnv();
        this.toast.success(penv.linked ? 'Variabilă deconectată din pool.' : 'Variabilă conectată din pool. Lansează (Deploy) pentru a aplica.');
      },
      error: (err) => {
        this.togglingLinkId.set(null);
        this.toast.error(err.error?.message || 'Eroare la actualizarea legăturii.');
      }
    });
  }

  // --- Build history + live build logs ---
  loadBuilds(): void {
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;
    this.projectService.listFunctionBuilds(projId, fn.id).subscribe({
      next: (res) => this.builds.set(res || []),
      error: () => this.builds.set([])
    });
  }

  selectBuild(build: ServerlessBuild | { id: string; status: string }): void {
    this.stopBuildLogsStream();
    this.selectedBuildId.set(build.id);
    this.buildLogs.set(['[Console] Conectare la fluxul de loguri de compilare...']);

    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    const url = this.projectService.getFunctionBuildLogsStreamUrl(projId, fn.id, build.id);
    this.buildLogSource = new EventSource(url);
    this.buildLogSource.onmessage = (event) => {
      this.buildLogs.update(lines => {
        const next = [...lines, event.data];
        if (next.length > 1000) next.shift();
        return next;
      });
    };
    this.buildLogSource.onerror = () => {
      // Stream closed (build finished or pod gone) — refresh the list so status/duration update.
      this.stopBuildLogsStream();
      this.loadBuilds();
    };
  }

  private stopBuildLogsStream(): void {
    if (this.buildLogSource) {
      this.buildLogSource.close();
      this.buildLogSource = null;
    }
  }

  buildStatusClass(status: string | undefined): string {
    switch (status) {
      case 'success': return 'bg-emerald-950/30 border border-emerald-800/40 text-emerald-400';
      case 'building': return 'bg-amber-950/30 border border-amber-800/40 text-amber-400 animate-pulse';
      case 'failed': return 'bg-red-950/30 border border-red-800/40 text-red-400';
      default: return 'bg-zinc-800/50 border border-zinc-700/60 text-zinc-400';
    }
  }

  buildStatusLabel(status: string | undefined): string {
    switch (status) {
      case 'success': return 'Succes';
      case 'building': return 'Compilare';
      case 'failed': return 'Eșuat';
      default: return status || '—';
    }
  }

  private startLogsStream(): void {
    this.stopLogsStream();
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    this.logs.set(['[Console] Conectare la fluxul de loguri live...']);
    const url = `${environment.apiBaseUrl}/projects/${projId}/functions/${fn.id}/logs/stream`;
    
    this.logSource = new EventSource(url);
    this.logSource.onmessage = (event) => {
      const line = event.data;
      this.logs.update(lines => {
        const nextLines = [...lines, line];
        if (nextLines.length > 500) {
          nextLines.shift();
        }
        return nextLines;
      });
    };

    this.logSource.onerror = (err) => {
      console.error('SSE connection error:', err);
    };
  }

  private stopLogsStream(): void {
    if (this.logSource) {
      this.logSource.close();
      this.logSource = null;
    }
  }

  private setupWsSubscriptions(): void {
    this.wsSubscriptions.unsubscribe();
    this.wsSubscriptions = new Subscription();

    // 1. WebSocket updated
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('serverless_function_updated').subscribe(payload => {
        const activeProj = this.parent.projectId();
        if (payload && payload.function && payload.function.projectId === activeProj) {
          // Update inside list
          this.functions.update(list => {
            const idx = list.findIndex(f => f.id === payload.function.id);
            if (idx !== -1) {
              const updated = [...list];
              updated[idx] = payload.function;
              return updated;
            } else {
              return [...list, payload.function];
            }
          });

          // Update selected if active
          const current = this.selectedFunction();
          if (current && current.id === payload.function.id) {
            this.selectedFunction.set(payload.function);
            // If function is now active or failed, we can notify user
            if (current.status === 'building' && payload.function.status === 'active') {
              this.toast.success(`Funcția "${payload.function.name}" a fost compilată și este online!`);
            } else if (current.status === 'building' && payload.function.status === 'failed') {
              this.toast.error(`Compilarea funcției "${payload.function.name}" a eșuat.`);
            }
          }
        }
      })
    );

    // 2. WebSocket deleted
    this.wsSubscriptions.add(
      this.wsService.onEvent<any>('serverless_function_deleted').subscribe(payload => {
        const activeProj = this.parent.projectId();
        if (payload && payload.function_id) {
          this.functions.update(list => list.filter(f => f.id !== payload.function_id));
          const current = this.selectedFunction();
          if (current && current.id === payload.function_id) {
            this.toast.info('Funcția pe care o vizualizați a fost ștearsă.');
            this.deselectFunction();
          }
        }
      })
    );
  }

  getStatusClass(status: string | undefined): string {
    switch (status) {
      case 'active':
        return 'bg-emerald-950/30 border border-emerald-800/40 text-emerald-400';
      case 'building':
        return 'bg-amber-950/30 border border-amber-800/40 text-amber-400 animate-pulse';
      case 'failed':
        return 'bg-red-950/30 border border-red-800/40 text-red-400';
      case 'draft':
      default:
        return 'bg-zinc-800/50 border border-zinc-700/60 text-zinc-400';
    }
  }

  getStatusText(status: string | undefined): string {
    switch (status) {
      case 'active': return 'Activ';
      case 'building': return 'Compilare';
      case 'failed': return 'Eșuat';
      case 'draft': return 'Draft';
      default: return 'Draft';
    }
  }

  initMonaco(): void {
    if (typeof monaco !== 'undefined') {
      this.createEditor();
      return;
    }

    const loaderUrl = 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.48.0/min/vs/loader.min.js';
    const existingScript = document.querySelector(`script[src="${loaderUrl}"]`);
    if (!existingScript) {
      const script = document.createElement('script');
      script.src = loaderUrl;
      script.async = true;
      script.onload = () => {
        const req = (window as any).require;
        req.config({ paths: { vs: 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.48.0/min/vs' } });
        req(['vs/editor/editor.main'], () => {
          this.createEditor();
        });
      };
      document.body.appendChild(script);
    } else {
      const interval = setInterval(() => {
        if (typeof monaco !== 'undefined') {
          clearInterval(interval);
          this.createEditor();
        }
      }, 100);
    }
  }

  createEditor(): void {
    const container = document.getElementById('monaco-editor-container');
    if (!container) return;

    if (this.editorInstance) {
      this.editorInstance.dispose();
      this.editorInstance = null;
    }

    const runtime = this.editRuntime();
    const language = runtime.startsWith('python') ? 'python' : 'javascript';

    this.editorInstance = monaco.editor.create(container, {
      value: this.editCode(),
      language: language,
      theme: 'vs-dark',
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 12,
      fontFamily: 'Fira Code, JetBrains Mono, Courier New, monospace',
      lineHeight: 20,
      tabSize: 2,
    });

    this.editorInstance.onDidChangeModelContent(() => {
      this.editCode.set(this.editorInstance.getValue());
    });
  }

  onRuntimeChange(runtime: string): void {
    this.editRuntime.set(runtime);
    if (this.editorInstance) {
      const model = this.editorInstance.getModel();
      if (model && typeof monaco !== 'undefined') {
        const lang = runtime.startsWith('python') ? 'python' : 'javascript';
        monaco.editor.setModelLanguage(model, lang);
      }
    }
  }
}
