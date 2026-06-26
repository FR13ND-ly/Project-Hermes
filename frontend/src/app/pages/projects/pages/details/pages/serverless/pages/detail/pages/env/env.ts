import { Component, inject, signal, computed, OnInit, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { ServerlessDetailComponent } from '../../detail';
import { ProjectService, ProjectEnvResponse, FunctionEnvResponse } from '../../../../../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-serverless-env',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './env.html',
  styles: ``,
})
export class ServerlessEnvComponent implements OnInit {
  readonly detailParent = inject(ServerlessDetailComponent);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly functionEnv = signal<FunctionEnvResponse[]>([]);
  readonly revealedEnvIds = signal<Record<string, boolean>>({});
  readonly availableProjectEnv = signal<ProjectEnvResponse[]>([]);
  readonly togglingLinkId = signal<string | null>(null);
  readonly linkedProjectEnv = computed(() => this.availableProjectEnv().filter(v => v.linked));
  readonly showAddEnvPanel = signal(false);
  readonly addEnvMode = signal<'new' | 'project'>('new');
  readonly newEnvKey = signal('');
  readonly newEnvValue = signal('');
  readonly newEnvIsSecret = signal(false);
  readonly editingEnvId = signal<string | null>(null);
  readonly savingEnv = signal(false);
  readonly reloadingEnv = signal(false);

  constructor() {
    effect(() => {
      const id = this.detailParent.functionId();
      if (id) {
        this.loadFunctionEnv();
        this.loadFunctionProjectEnv();
      }
    });
  }

  ngOnInit(): void {
    // Handled by effect
  }

  loadFunctionEnv(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
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
    this.newEnvIsSecret.set(false);
  }

  closeAddEnvPanel(): void {
    this.showAddEnvPanel.set(false);
    this.editingEnvId.set(null);
    this.newEnvKey.set('');
    this.newEnvValue.set('');
    this.newEnvIsSecret.set(false);
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
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    const key = this.newEnvKey().trim().toUpperCase();
    if (!key) {
      this.toast.error('Key is required.');
      return;
    }
    this.savingEnv.set(true);
    this.projectService.setFunctionEnv(projId, inst.id, {
      key,
      value: this.newEnvValue(),
      isSecret: this.newEnvIsSecret()
    }).subscribe({
      next: () => {
        this.savingEnv.set(false);
        this.closeAddEnvPanel();
        this.toast.success('Variable saved. Deploy or "Reload variables" to apply.');
        this.loadFunctionEnv();
      },
      error: (err: any) => {
        this.savingEnv.set(false);
        this.toast.error(err.error?.message || 'Error saving variable.');
      }
    });
  }

  async onDeleteFunctionEnv(env: FunctionEnvResponse): Promise<void> {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    const confirmed = await this.confirm.ask({
      title: 'Delete variable',
      message: `Are you sure you want to delete "${env.key}"?`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true,
    });
    if (!confirmed) return;
    this.projectService.deleteFunctionEnv(projId, inst.id, env.id).subscribe({
      next: () => {
        this.toast.success('Variable deleted.');
        this.loadFunctionEnv();
      },
      error: (err: any) => this.toast.error(err.error?.message || 'Failed to delete.')
    });
  }

  toggleRevealEnv(id: string): void {
    this.revealedEnvIds.update(m => ({ ...m, [id]: !m[id] }));
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => this.toast.success('Copied!'));
  }

  onReloadFunctionEnv(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    this.reloadingEnv.set(true);
    this.projectService.reloadFunctionEnv(projId, inst.id).subscribe({
      next: (updated) => {
        this.reloadingEnv.set(false);
        this.detailParent.selectedInstance.set(updated);
        this.toast.success('Variables reapplied without recompiling.');
      },
      error: (err: any) => {
        this.reloadingEnv.set(false);
        this.toast.error(err.error?.message || 'Error reloading variables.');
      }
    });
  }

  loadFunctionProjectEnv(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listFunctionProjectEnv(projId, inst.id).subscribe({
      next: (res) => this.availableProjectEnv.set(res || []),
      error: () => this.availableProjectEnv.set([])
    });
  }

  onToggleProjectEnvLink(penv: ProjectEnvResponse): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    this.togglingLinkId.set(penv.id);
    const op = penv.linked
      ? this.projectService.unlinkFunctionProjectEnv(projId, inst.id, penv.id)
      : this.projectService.linkFunctionProjectEnv(projId, inst.id, penv.id);
    op.subscribe({
      next: () => {
        this.togglingLinkId.set(null);
        this.loadFunctionProjectEnv();
        this.toast.success(penv.linked ? 'Disconnected from pool.' : 'Connected from pool. Deploy/Reload to apply.');
      },
      error: (err: any) => {
        this.togglingLinkId.set(null);
        this.toast.error(err.error?.message || 'Failed to update link.');
      }
    });
  }
}
