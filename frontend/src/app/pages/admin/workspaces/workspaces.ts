import { Component, inject, signal, OnInit } from '@angular/core';
import { DatePipe, DecimalPipe } from '@angular/common';
import { Router } from '@angular/router';
import { AuthService } from '../../../core/services/auth';
import { WorkspaceService, AdminWorkspaceStats } from '../../../core/services/workspace.service';
import { ToastService } from '../../../core/services/toast.service';
import { ConfirmService } from '../../../core/services/confirm.service';

@Component({
  selector: 'app-admin-workspaces',
  imports: [DatePipe, DecimalPipe],
  templateUrl: './workspaces.html',
  styleUrl: './workspaces.css',
})
export class AdminWorkspaces implements OnInit {
  private readonly auth = inject(AuthService);
  private readonly workspaceService = inject(WorkspaceService);
  private readonly toast = inject(ToastService);
  private readonly router = inject(Router);
  private readonly confirm = inject(ConfirmService);

  readonly workspaces = signal<AdminWorkspaceStats[]>([]);
  readonly loading = signal(false);
  readonly deletingId = signal<string | null>(null);

  constructor() {
    // Security: only super admins are allowed here
    const user = this.auth.currentUser();
    if (!user || !user.is_super_admin) {
      this.router.navigate(['/dashboard']);
    }
  }

  ngOnInit(): void {
    this.load();
  }

  load(): void {
    this.loading.set(true);
    this.workspaceService.adminListWorkspaces().subscribe({
      next: (res) => {
        this.workspaces.set(res || []);
        this.loading.set(false);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error loading workspaces.');
        this.loading.set(false);
      }
    });
  }

  memoryPct(ws: AdminWorkspaceStats): number {
    if (!ws.maxMemoryMb) return 0;
    return Math.min(100, Math.round((ws.allocatedMemoryMb / ws.maxMemoryMb) * 100));
  }

  storagePct(ws: AdminWorkspaceStats): number {
    if (!ws.maxStorageGb) return 0;
    return Math.min(100, Math.round((ws.allocatedStorageGb / ws.maxStorageGb) * 100));
  }

  async onDelete(ws: AdminWorkspaceStats): Promise<void> {
    const confirmed = await this.confirm.ask({
      title: 'Delete Workspace',
      message: `Are you sure you want to delete the workspace "${ws.name}"? This irreversibly destroys the K8s namespace, all applications, databases, storage and associated data.`,
      confirmText: 'Delete Permanently',
      cancelText: 'Cancel',
      isDanger: true
    });
    if (!confirmed) return;

    this.deletingId.set(ws.id);
    this.workspaceService.adminDeleteWorkspace(ws.id).subscribe({
      next: () => {
        this.deletingId.set(null);
        this.toast.success(`Workspace "${ws.name}" has been deleted.`);
        this.workspaces.update(list => list.filter(w => w.id !== ws.id));
      },
      error: (err) => {
        this.deletingId.set(null);
        this.toast.error(err.error?.message || 'Error deleting the workspace.');
      }
    });
  }
}
