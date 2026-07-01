import { Component, inject, computed, signal, effect, HostListener } from '@angular/core';
import { RouterOutlet, RouterLink, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { AuthService } from './core/services/auth';
import { WorkspaceService, Workspace, WorkspaceUsage } from './core/services/workspace.service';
import { ToastService } from './core/services/toast.service';
import { ConfirmService } from './core/services/confirm.service';
import { ThemeService } from './core/services/theme.service';
import { BuildIndicator } from './shared/components/build-indicator/build-indicator';

@Component({
  selector: 'app-root',
  imports: [RouterOutlet, RouterLink, FormsModule, BuildIndicator],
  templateUrl: './app.html',
  styleUrl: './app.css'
})
export class App {
  readonly auth = inject(AuthService);
  private readonly workspaceService = inject(WorkspaceService);
  private readonly router = inject(Router);
  readonly toast = inject(ToastService);
  readonly confirm = inject(ConfirmService);
  readonly theme = inject(ThemeService);

  readonly isAuthenticated = computed(() => this.auth.isAuthenticated());
  readonly currentUser = computed(() => this.auth.currentUser());
  readonly isSuperAdmin = computed(() => this.auth.currentUser()?.is_super_admin === true);

  isAdminRouteActive(): boolean {
    return this.router.url.startsWith('/admin/');
  }

  readonly workspaces = signal<Workspace[]>([]);
  readonly showWorkspaceDropdown = signal(false);
  readonly showUserMenu = signal(false);
  readonly showThemeMenu = signal(false);
  readonly showCreateWorkspaceModal = signal(false);
  readonly newWorkspaceName = signal('');

  // Real resource usage for the current workspace (header gauges).
  readonly usage = signal<WorkspaceUsage | null>(null);

  // Whole-server resources (super-admin header gauges): CPU / RAM / disk, used + total.
  readonly serverResources = signal<any | null>(null);
  private serverResPoll: any = null;

  readonly serverCpuPercent = computed(() => {
    const s = this.serverResources();
    return s && s.cpuCoresTotal > 0 ? Math.min(100, Math.round((s.cpuCoresUsed / s.cpuCoresTotal) * 100)) : 0;
  });
  readonly serverRamPercent = computed(() => {
    const s = this.serverResources();
    return s && s.memoryBytesTotal > 0 ? Math.min(100, Math.round((s.memoryBytesUsed / s.memoryBytesTotal) * 100)) : 0;
  });
  readonly serverDiskPercent = computed(() => {
    const s = this.serverResources();
    return s && s.diskBytesTotal > 0 ? Math.min(100, Math.round((s.diskBytesUsed / s.diskBytesTotal) * 100)) : 0;
  });

  /** CPU cores, compact (e.g. "1.2" / "8"). */
  fmtCores(c: number): string {
    if (c === null || c === undefined) return '–';
    return c >= 10 ? c.toFixed(0) : c.toFixed(1);
  }
  /** Bytes → GiB string (e.g. "3.1"). */
  fmtGiB(bytes: number): string {
    if (!bytes) return '0';
    return (bytes / (1024 ** 3)).toFixed(1);
  }

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
        // Server gauges are super-admin only; the rest see their workspace usage.
        if (this.isSuperAdmin()) {
          this.loadServerResources();
        } else {
          this.serverResources.set(null);
        }
      } else {
        this.usage.set(null);
        this.workspaces.set([]);
        this.serverResources.set(null);
      }
    });

    // Refresh server gauges periodically while a super admin is signed in.
    this.serverResPoll = setInterval(() => {
      if (this.isAuthenticated() && this.isSuperAdmin()) {
        this.loadServerResources();
      }
    }, 15000);
  }

  loadServerResources(): void {
    this.auth.getServerResources().subscribe({
      next: (s) => this.serverResources.set(s),
      error: () => this.serverResources.set(null),
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
      return;
    }
    if (this.showUserMenu()) {
      this.showUserMenu.set(false);
      return;
    }
    if (this.showThemeMenu()) {
      this.showThemeMenu.set(false);
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
