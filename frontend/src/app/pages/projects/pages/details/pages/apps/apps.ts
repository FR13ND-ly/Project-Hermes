import { Component, inject, signal, OnInit } from '@angular/core';
import { DatePipe } from '@angular/common';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { ProjectService, AppDetail } from '../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../core/services/confirm.service';

@Component({
  selector: 'app-apps',
  imports: [DatePipe],
  templateUrl: './apps.html',
  styleUrl: './apps.css',
})
export class Apps implements OnInit {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly loading = signal(false);

  ngOnInit(): void {
    this.refreshApps();
  }

  refreshApps(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;
    this.loading.set(true);
    this.parent.loadDetails(projectId);
    setTimeout(() => this.loading.set(false), 600);
  }

  onViewDetails(app: AppDetail, tab: string = 'telemetry'): void {
    this.parent.selectedApp.set(app);
    this.parent.showAddAppForm.set(false);
    this.router.navigate([`/projects/${this.parent.projectId()}/apps/${app.id}`], { queryParams: { tab } });
  }

  async onDeleteApp(app: AppDetail): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Application Completely',
      message: `Are you sure you want to completely delete the application "${app.name}"? This action is irreversible and will destroy all active instances and associated data.`,
      confirmText: 'Delete Application',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;
    
    this.loading.set(true);
    this.projectService.deleteApp(app.id).subscribe({
      next: () => {
        this.toast.success(`Application "${app.name}" has been deleted successfully.`);
        if (this.parent.selectedApp()?.id === app.id) {
          this.parent.selectedApp.set(null);
        }
        this.refreshApps();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete application.');
        this.loading.set(false);
      }
    });
  }

  onAddApp(): void {
    this.parent.showAddAppForm.set(true);
    this.router.navigate([`/projects/${this.parent.projectId()}`]);
  }
}
