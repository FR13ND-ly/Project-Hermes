import { Component, inject, signal, OnInit, OnDestroy } from '@angular/core';
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
        if (!silent) this.error.set('Eroare la încărcarea spațiilor de lucru.');
      }
    });

    // Load projects
    this.projectService.listProjects().subscribe({
      next: (res) => {
        this.projects.set(res);
        this.loading.set(false);
      },
      error: () => {
        if (!silent) this.error.set('Eroare la încărcarea proiectelor.');
        this.loading.set(false);
      }
    });

    // Load usage
    this.workspaceService.getUsage().subscribe({
      next: (res) => this.usage.set(res),
      error: () => console.error('Eroare la încărcarea cotelor de resurse.')
    });
  }

  onSwitchWorkspace(workspaceId: string): void {
    this.loading.set(true);
    this.auth.switchWorkspace(workspaceId).subscribe({
      next: () => {
        this.loadData();
      },
      error: (err) => {
        this.error.set(err.error?.message || 'Eroare la schimbarea spațiului de lucru.');
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
        this.error.set(err.error?.message || 'Eroare la crearea spațiului de lucru.');
        this.loading.set(false);
      }
    });
  }
}
