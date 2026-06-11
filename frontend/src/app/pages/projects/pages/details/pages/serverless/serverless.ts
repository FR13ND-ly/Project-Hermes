import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Details } from '../../details';
import { ProjectService, ServerlessFunction } from '../../../../../../core/services/project.service';
import { DomainService, Domain } from '../../../../../../core/services/domain.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';
import { WebSocketService } from '../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-project-serverless',
  standalone: true,
  imports: [CommonModule, FormsModule],
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
  readonly selectedFunction = signal<ServerlessFunction | null>(null);
  readonly activeTab = signal<'details' | 'code' | 'logs'>('details');
  readonly domains = signal<Domain[]>([]);
  readonly logs = signal<string[]>([]);

  // Creation Modal Signals
  readonly showCreateModal = signal(false);
  readonly creating = signal(false);
  readonly newFnName = signal('');
  readonly newFnMethod = signal('GET');
  readonly newFnRoutePath = signal('');
  readonly newFnMemory = signal(0);

  // Edit / Settings Signals
  readonly editName = signal('');
  readonly editCode = signal('');
  readonly editMethod = signal('GET');
  readonly editRoutePath = signal('');
  readonly editMemory = signal(0);
  readonly editAssignedDomain = signal<string | null>(null);
  readonly editEnvVariables = signal<{ key: string, value: string }[]>([]);
  readonly savingSettings = signal(false);
  readonly deploying = signal(false);

  private logSource: EventSource | null = null;
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
    this.wsSubscriptions.unsubscribe();
  }

  loadFunctions(): void {
    const projId = this.parent.projectId();
    if (!projId) return;

    this.loading.set(true);
    this.projectService.listProjectFunctions(projId).subscribe({
      next: (res) => {
        this.functions.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la încărcarea funcțiilor serverless.');
        this.loading.set(false);
      }
    });
  }

  loadDomains(): void {
    this.domainService.listDomains().subscribe({
      next: (res) => {
        this.domains.set(res || []);
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
    this.showCreateModal.set(true);
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
      memoryLimitMb: memory
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

    // Parse environment variables
    const envs = fn.envVariables || [];
    const formattedEnvs = Array.isArray(envs) ? envs.map((e: any) => ({
      key: e.key || '',
      value: e.value || ''
    })) : [];
    this.editEnvVariables.set(formattedEnvs);
  }

  deselectFunction(): void {
    this.stopLogsStream();
    this.selectedFunction.set(null);
  }

  onTabChange(tab: 'details' | 'code' | 'logs'): void {
    this.activeTab.set(tab);
    if (tab === 'logs') {
      this.startLogsStream();
    } else {
      this.stopLogsStream();
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
      envVariables
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
      envVariables
    }).subscribe({
      next: (updated) => {
        this.selectedFunction.set(updated);
        
        // Trigger deploy
        this.projectService.deployFunction(projId, fn.id).subscribe({
          next: () => {
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

  private startLogsStream(): void {
    this.stopLogsStream();
    const projId = this.parent.projectId();
    const fn = this.selectedFunction();
    if (!projId || !fn) return;

    this.logs.set(['[Console] Conectare la fluxul de loguri live...']);
    const url = `http://localhost:8000/api/v1/projects/${projId}/functions/${fn.id}/logs/stream`;
    
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
}
