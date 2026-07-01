import { Component, inject, signal, computed, OnInit, OnDestroy } from '@angular/core';
import { DecimalPipe, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink, Router } from '@angular/router';
import { WorkspaceService, Workspace, WorkspaceUsage } from '../../core/services/workspace.service';
import { ProjectService, Project } from '../../core/services/project.service';
import { AuthService } from '../../core/services/auth';
import { WebSocketService } from '../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-dashboard',
  imports: [DecimalPipe, DatePipe, FormsModule, RouterLink],
  templateUrl: './dashboard.html',
  styleUrl: './dashboard.css',
})
export class Dashboard implements OnInit, OnDestroy {
  private readonly workspaceService = inject(WorkspaceService);
  private readonly projectService = inject(ProjectService);
  readonly auth = inject(AuthService);
  private readonly router = inject(Router);
  private readonly wsService = inject(WebSocketService);

  readonly projects = signal<Project[]>([]);
  readonly workspaces = signal<Workspace[]>([]);
  readonly usage = signal<WorkspaceUsage | null>(null);

  // Quota gauge fills (0 when unlimited — the template shows "Unlimited" separately).
  readonly memPercent = computed(() => {
    const u = this.usage();
    if (!u || u.maxMemoryMb <= 0) return 0;
    return Math.min(100, (u.usedMemoryMb / u.maxMemoryMb) * 100);
  });
  readonly storagePercent = computed(() => {
    const u = this.usage();
    if (!u || u.maxStorageGb <= 0) return 0;
    return Math.min(100, (u.usedStorageGb / u.maxStorageGb) * 100);
  });

  readonly newWorkspaceName = signal('');
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  private wsSubscriptions = new Subscription();

  ngOnInit(): void {
    this.loadData();
    this.setupWebSocketSubscriptions();
  }

  ngOnDestroy(): void {
    this.wsSubscriptions.unsubscribe();
  }

  private setupWebSocketSubscriptions(): void {
    // Reload dashboard silently on any relevant status/resource changes
    const events = ['instance_status_changed', 'database_status_changed', 'build_status_changed', 'incident_created'];
    for (const event of events) {
      this.wsSubscriptions.add(
        this.wsService.onEvent(event).subscribe(() => {
          this.loadData(true);
        })
      );
    }
  }

  loadData(silent = false): void {
    if (!silent) {
      this.loading.set(true);
    }
    this.error.set(null);

    // Load workspaces
    this.workspaceService.listWorkspaces().subscribe({
      next: (res) => this.workspaces.set(res),
      error: () => {
        if (!silent) this.error.set('Error loading workspaces.');
      }
    });

    // Load projects
    this.projectService.listProjects().subscribe({
      next: (res) => {
        this.projects.set(res);
        this.loading.set(false);
      },
      error: () => {
        if (!silent) this.error.set('Error loading projects.');
        this.loading.set(false);
      }
    });

    // Load usage
    this.workspaceService.getUsage().subscribe({
      next: (res) => this.usage.set(res),
      error: () => console.error('Error loading resource quotas.')
    });
  }

  onSwitchWorkspace(workspaceId: string): void {
    this.loading.set(true);
    this.auth.switchWorkspace(workspaceId).subscribe({
      next: () => {
        this.loadData();
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Error switching workspace.');
        this.loading.set(false);
      }
    });
  }

  onCreateWorkspace(): void {
    if (!this.newWorkspaceName()) return;

    this.loading.set(true);
    this.workspaceService.createWorkspace(this.newWorkspaceName()).subscribe({
      next: (res) => {
        this.newWorkspaceName.set('');
        this.onSwitchWorkspace(res.id);
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Error creating workspace.');
        this.loading.set(false);
      }
    });
  }
}
