import { Component, inject, computed, signal, effect, HostListener } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { AuthService } from './core/services/auth';
import { WorkspaceService, Workspace, WorkspaceUsage } from './core/services/workspace.service';
import { ToastService } from './core/services/toast.service';
import { ConfirmService } from './core/services/confirm.service';
import { BuildIndicator } from './shared/components/build-indicator/build-indicator';

@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, RouterLinkActive, FormsModule, BuildIndicator],
  templateUrl: './app.html',
  styleUrl: './app.css'
})
export class App {
  readonly auth = inject(AuthService);
  private readonly workspaceService = inject(WorkspaceService);
  private readonly router = inject(Router);
  readonly toast = inject(ToastService);
  readonly confirm = inject(ConfirmService);

  readonly isAuthenticated = computed(() => this.auth.isAuthenticated());
  readonly currentUser = computed(() => this.auth.currentUser());
  readonly isSuperAdmin = computed(() => this.auth.currentUser()?.is_super_admin === true);

  isAdminRouteActive(): boolean {
    return this.router.url.startsWith('/admin/');
  }

  readonly workspaces = signal<Workspace[]>([]);
  readonly showWorkspaceDropdown = signal(false);
  readonly showCreateWorkspaceModal = signal(false);
  readonly newWorkspaceName = signal('');

  // Real resource usage for the current workspace (header gauges).
  readonly usage = signal<WorkspaceUsage | null>(null);

  /** RAM usage as a percentage, or null when the workspace has no memory cap. */
  readonly ramPercent = computed(() => {
    const u = this.usage();
    if (!u || u.maxMemoryMb <= 0) return null;
    return Math.min(100, Math.round((u.usedMemoryMb / u.maxMemoryMb) * 100));
  });

  /** Disk usage as a percentage, or null when the workspace has no storage cap. */
  readonly diskPercent = computed(() => {
    const u = this.usage();
    if (!u || u.maxStorageGb <= 0) return null;
    return Math.min(100, Math.round((u.usedStorageGb / u.maxStorageGb) * 100));
  });

  readonly currentWorkspaceName = computed(() => {
    const activeId = this.auth.currentWorkspaceId();
    const list = this.workspaces();
    if (list.length === 0) {
      return 'No Workspace';
    }
    const found = list.find(w => w.id === activeId);
    return found ? found.name : 'Personal Workspace';
  });

  constructor() {
    // Reactively load workspace list + usage when a session is active.
    // Route protection itself is handled by the route guards (auth.guard.ts).
    effect(() => {
      if (this.isAuthenticated()) {
        this.loadWorkspaces();
        this.loadUsage();
      } else {
        this.usage.set(null);
        this.workspaces.set([]);
      }
    });
  }

  @HostListener('document:keydown.escape')
  onEscape(): void {
    if (this.confirm.activeModal()) {
      this.confirm.cancel();
      return;
    }
    if (this.showCreateWorkspaceModal()) {
      this.showCreateWorkspaceModal.set(false);
      return;
    }
    if (this.showWorkspaceDropdown()) {
      this.showWorkspaceDropdown.set(false);
    }
  }

  loadWorkspaces(): void {
    this.workspaceService.listWorkspaces().subscribe({
      next: (res) => this.workspaces.set(res || []),
      error: (err) => console.error('Error loading workspaces', err)
    });
  }

  loadUsage(): void {
    this.workspaceService.getUsage().subscribe({
      next: (u) => this.usage.set(u),
      error: () => this.usage.set(null)
    });
  }

  onSwitchWorkspace(workspaceId: string): void {
    this.auth.switchWorkspace(workspaceId).subscribe({
      next: () => {
        // Redirect context to dashboard and force-reset to reload data
        this.router.navigate(['/dashboard']).then(() => {
          window.location.href = '/dashboard';
        });
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error switching workspace.');
      }
    });
  }

  onOpenCreateWorkspaceModal(): void {
    this.newWorkspaceName.set('');
    this.showCreateWorkspaceModal.set(true);
  }

  onCreateWorkspace(): void {
    const name = this.newWorkspaceName().trim();
    if (!name) return;

    this.workspaceService.createWorkspace(name).subscribe({
      next: (res) => {
        this.newWorkspaceName.set('');
        this.showCreateWorkspaceModal.set(false);
        this.onSwitchWorkspace(res.id);
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Error creating workspace.');
      }
    });
  }

  logout(): void {
    this.auth.logout();
  }
}
