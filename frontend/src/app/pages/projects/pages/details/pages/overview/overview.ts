import { Component, inject, signal, OnInit } from '@angular/core';
import { CommonModule, NgClass, DatePipe, DecimalPipe } from '@angular/common';
import { Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { AppDetail, ProjectService } from '../../../../../../core/services/project.service';
import { WorkspaceService, WorkspaceUsage } from '../../../../../../core/services/workspace.service';
import { DomainService, Domain } from '../../../../../../core/services/domain.service';

@Component({
  selector: 'app-overview',
  standalone: true,
  imports: [CommonModule, NgClass, DatePipe, DecimalPipe, RouterLink],
  templateUrl: './overview.html',
  styleUrl: './overview.css',
})
export class Overview implements OnInit {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly projectService = inject(ProjectService);
  private readonly workspaceService = inject(WorkspaceService);
  private readonly domainService = inject(DomainService);
  private readonly router = inject(Router);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly loadingDbs = signal(false);
  readonly workspaceUsage = signal<WorkspaceUsage | null>(null);
  readonly loadingUsage = signal(false);
  readonly domains = signal<Domain[]>([]);
  readonly loadingDomains = signal(false);

  getAppStatus(app: AppDetail): 'running' | 'building' | 'stopped' {
    const insts = app.instances || [];
    if (insts.some(i => i.status === 'running')) return 'running';
    if (insts.some(i => i.status === 'building')) return 'building';
    return 'stopped';
  }

  getStatusLabel(app: AppDetail): string {
    switch (this.getAppStatus(app)) {
      case 'running': return 'Activ';
      case 'building': return 'Build';
      default: return 'Oprit';
    }
  }

  ngOnInit(): void {
    this.loadDatabases();
    this.loadWorkspaceUsage();
    this.loadDomains();
  }

  loadDatabases(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingDbs.set(true);
    this.dbService.listDatabases(projectId, 1, 1000).subscribe({
      next: (res) => {
        this.databases.set(res?.items || []);
        this.loadingDbs.set(false);
      },
      error: () => {
        this.databases.set([]);
        this.loadingDbs.set(false);
      }
    });
  }

  loadWorkspaceUsage(): void {
    this.loadingUsage.set(true);
    this.workspaceService.getUsage().subscribe({
      next: (res) => {
        this.workspaceUsage.set(res);
        this.loadingUsage.set(false);
      },
      error: () => {
        this.workspaceUsage.set(null);
        this.loadingUsage.set(false);
      }
    });
  }

  loadDomains(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingDomains.set(true);
    this.domainService.listDomains(1, 1000, projectId).subscribe({
      next: (res) => {
        this.domains.set(res?.items || []);
        this.loadingDomains.set(false);
      },
      error: () => {
        this.domains.set([]);
        this.loadingDomains.set(false);
      }
    });
  }

  get totalLinksCount(): number {
    let count = 0;
    for (const app of this.parent.apps()) {
      for (const inst of app.instances || []) {
        if (inst.assignedDomain) count++;
      }
    }
    for (const dom of this.domains()) {
      if (dom.targetType === 'custom') count++;
    }
    return count;
  }

  getAppScreenshot(app: AppDetail): string | null {
    const runningInst = (app.instances || []).find(i => i.status === 'running');
    if (runningInst && runningInst.screenshotCapturedAt) {
      return this.projectService.getScreenshotUrl(app.id, runningInst.id, runningInst.screenshotCapturedAt);
    }
    return null;
  }

  get totalRunningPods(): number {
    return this.parent.apps().reduce((acc, app) => {
      const active = (app.instances || []).filter(inst => inst.status === 'running').length;
      return acc + active;
    }, 0);
  }

  get totalBuildingApps(): number {
    return this.parent.apps().reduce((acc, app) => {
      const building = (app.instances || []).filter(inst => inst.status === 'building').length;
      return acc + building;
    }, 0);
  }

  onViewApp(app: AppDetail, tab: string = 'telemetry'): void {
    this.parent.selectedApp.set(app);
    this.router.navigate([`/projects/${this.parent.projectId()}/apps/${app.id}`], { queryParams: { tab } });
  }
}
