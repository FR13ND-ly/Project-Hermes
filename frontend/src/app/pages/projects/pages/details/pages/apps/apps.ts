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
      title: 'Ștergere Completă Aplicație',
      message: `Sigur doriți să ștergeți complet aplicația "${app.name}"? Această acțiune este ireversibilă și va distruge toate instanțele active și datele asociate.`,
      confirmText: 'Șterge Aplicația',
      cancelText: 'Anulează',
      isDanger: true
    });
    if (!confirmed) return;
    
    this.loading.set(true);
    this.projectService.deleteApp(app.id).subscribe({
      next: () => {
        this.toast.success(`Aplicația "${app.name}" a fost ștearsă cu succes.`);
        if (this.parent.selectedApp()?.id === app.id) {
          this.parent.selectedApp.set(null);
        }
        this.refreshApps();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea aplicației.');
        this.loading.set(false);
      }
    });
  }

  onAddApp(): void {
    this.parent.showAddAppForm.set(true);
    this.router.navigate([`/projects/${this.parent.projectId()}`]);
  }
}
