import { Component, inject, computed, signal, effect } from '@angular/core';
import { RouterOutlet, RouterLink, RouterLinkActive, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { AuthService } from './core/services/auth';
import { WorkspaceService, Workspace } from './core/services/workspace.service';
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

  readonly workspaces = signal<Workspace[]>([]);
  readonly showWorkspaceDropdown = signal(false);
  readonly showCreateWorkspaceModal = signal(false);
  readonly newWorkspaceName = signal('');

  readonly currentWorkspaceName = computed(() => {
    const activeId = this.auth.currentWorkspaceId();
    const list = this.workspaces();
    if (list.length === 0) {
      return 'Fără Workspace';
    }
    const found = list.find(w => w.id === activeId);
    return found ? found.name : 'Personal Workspace';
  });

  constructor() {
    this.checkRoute();

    // Reactively load workspaces list when user session is active
    effect(() => {
      if (this.isAuthenticated()) {
        this.loadWorkspaces();
      }
    });
  }

  loadWorkspaces(): void {
    this.workspaceService.listWorkspaces().subscribe({
      next: (res) => this.workspaces.set(res || []),
      error: (err) => console.error('Eroare la încărcarea spațiilor de lucru', err)
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
        this.toast.error(err.error?.message || 'Eroare la schimbarea spațiului de lucru.');
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
        this.toast.error(err.error?.message || 'Eroare la crearea spațiului de lucru.');
      }
    });
  }

  logout(): void {
    this.auth.logout();
  }

  private checkRoute(): void {
    setTimeout(() => {
      if (!this.auth.isAuthenticated()) {
        this.router.navigate(['/auth']);
      } else if (window.location.pathname === '/' || window.location.pathname === '/auth') {
        this.router.navigate(['/dashboard']);
      }
    }, 50);
  }
}
