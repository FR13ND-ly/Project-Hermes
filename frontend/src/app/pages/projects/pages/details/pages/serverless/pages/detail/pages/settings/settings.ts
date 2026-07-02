import { Component, inject, signal, OnInit, effect } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { ServerlessDetailComponent } from '../../detail';
import { ProjectService } from '../../../../../../../../../../core/services/project.service';
import { DomainService, Domain } from '../../../../../../../../../../core/services/domain.service';
import { ToastService } from '../../../../../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-serverless-settings',
  imports: [FormsModule],
  templateUrl: './settings.html',
  styles: ``,
})
export class ServerlessSettingsComponent implements OnInit {
  readonly detailParent = inject(ServerlessDetailComponent);
  private readonly projectService = inject(ProjectService);
  private readonly domainService = inject(DomainService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);
  private readonly router = inject(Router);

  // Settings state
  readonly editName = signal('');
  readonly editRuntime = signal('nodejs-cjs');
  readonly editMemory = signal(0);
  readonly editAssignedDomain = signal<string | null>(null);
  readonly savingSettings = signal(false);

  readonly domains = signal<Domain[]>([]);

  // Register domain modal
  readonly showAddDomainModal = signal(false);
  readonly newDomainFqdn = signal('');
  readonly addingDomain = signal(false);

  private initialized = false;

  constructor() {
    effect(() => {
      // Reset initialization when functionId changes
      const id = this.detailParent.functionId();
      if (id) {
        this.initialized = false;
      }
    });

    effect(() => {
      const inst = this.detailParent.selectedInstance();
      if (inst && !this.initialized) {
        this.editName.set(inst.name);
        this.editRuntime.set(inst.runtime || 'nodejs-cjs');
        this.editMemory.set(inst.memoryLimitMb);
        this.editAssignedDomain.set(inst.assignedDomain || null);
        this.initialized = true;
      }
    });
  }

  ngOnInit(): void {
    this.loadDomains();
  }

  loadDomains(): void {
    const projectId = this.detailParent.parent.projectId();
    this.domainService.listDomains(1, 1000, projectId || undefined).subscribe({
      next: (res: any) => this.domains.set(res?.items || []),
      error: () => {}
    });
  }

  onSaveSettings(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    const name = this.editName().trim();
    if (!name) {
      this.toast.error('Name is required.');
      return;
    }
    this.savingSettings.set(true);
    this.projectService.updateInstance(projId, inst.id, {
      name,
      runtime: this.editRuntime(),
      memoryLimitMb: this.editMemory(),
      assignedDomain: this.editAssignedDomain() || null,
    }).subscribe({
      next: (updated: any) => {
        this.savingSettings.set(false);
        this.detailParent.selectedInstance.set(updated);
        this.toast.success('Instance settings saved.');
        this.detailParent.loadFunctionDetails(inst.id, true);
      },
      error: (err: any) => {
        this.savingSettings.set(false);
        this.toast.error(err.error?.message || 'Error saving settings.');
      }
    });
  }

  async onDeleteInstance(): Promise<void> {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    const confirmed = await this.confirm.ask({
      title: 'Delete serverless instance',
      message: `Are you sure you want to delete instance "${inst.name}"? All associated routes and K8s resources will be removed.`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true,
    });
    if (!confirmed) return;
    this.projectService.deleteFunction(projId, inst.id).subscribe({
      next: () => {
        this.toast.success('Instance deleted.');
        this.router.navigate(['/projects', projId, 'serverless']);
      },
      error: (err: any) => this.toast.error(err.error?.message || 'Failed to delete.')
    });
  }

  onOpenAddDomainModal(): void {
    this.newDomainFqdn.set('');
    this.showAddDomainModal.set(true);
  }

  onRegisterDomain(): void {
    const inst = this.detailParent.selectedInstance();
    if (!inst) return;
    const fqdn = this.newDomainFqdn().trim().toLowerCase();
    if (!fqdn) {
      this.toast.error('Domain name is required.');
      return;
    }
    this.addingDomain.set(true);
    this.domainService.addDomain({
      fqdn,
      targetType: 'serverless',
      targetId: inst.id,
      routingType: 'reverse_proxy',
      isSsl: true
    }).subscribe({
      next: (domain: any) => {
        this.addingDomain.set(false);
        this.showAddDomainModal.set(false);
        this.toast.success('Domain registered successfully!');
        this.loadDomains();
        this.editAssignedDomain.set(domain.fqdn);
      },
      error: (err: any) => {
        this.addingDomain.set(false);
        this.toast.error(err.error?.message || 'Failed to register domain.');
      }
    });
  }
}
