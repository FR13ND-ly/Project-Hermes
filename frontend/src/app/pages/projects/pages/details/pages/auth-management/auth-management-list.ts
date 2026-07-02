import { Component, inject, signal, OnInit } from '@angular/core';
import { Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { AuthManagementService, BaasService } from '../../../../../../core/services/auth-management.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-auth-management-list',
  imports: [RouterLink],
  templateUrl: './auth-management-list.html',
})
export class AuthManagementList implements OnInit {
  readonly parent = inject(Details);
  private readonly authMgmtService = inject(AuthManagementService);
  private readonly router = inject(Router);
  readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly services = signal<BaasService[]>([]);
  readonly loadingServices = signal(false);

  ngOnInit(): void {
    const projectId = this.parent.projectId();
    if (projectId) {
      this.loadServices(projectId);
    }
  }

  loadServices(projectId: string): void {
    this.loadingServices.set(true);
    this.authMgmtService.listServices(projectId).subscribe({
      next: (list) => {
        this.services.set(list || []);
        this.loadingServices.set(false);
      },
      error: () => {
        this.services.set([]);
        this.loadingServices.set(false);
      }
    });
  }

  selectService(svc: BaasService): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'auth-management', svc.id]);
  }

  async onDeleteService(svc: BaasService): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Authentication Service',
      message: `Are you sure you want to delete "${svc.name}"? All associated users, roles, and API keys will be permanently deleted.`,
      confirmText: 'Delete',
      isDanger: true,
    });

    if (!confirmed) return;

    this.authMgmtService.deleteService(svc.id).subscribe({
      next: () => {
        this.toast.success(`Service "${svc.name}" deleted.`);
        this.services.update(list => list.filter(s => s.id !== svc.id));
      },
      error: (err) => this.toast.error(err.error?.message || 'Failed to delete service.')
    });
  }
}
