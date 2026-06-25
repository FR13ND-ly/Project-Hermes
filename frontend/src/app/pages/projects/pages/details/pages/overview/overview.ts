import { Component, inject, signal, OnInit } from '@angular/core';
import { CommonModule, NgClass, DatePipe } from '@angular/common';
import { Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { AppDetail, ProjectService } from '../../../../../../core/services/project.service';

@Component({
  selector: 'app-overview',
  standalone: true,
  imports: [CommonModule, NgClass, DatePipe, RouterLink],
  templateUrl: './overview.html',
  styleUrl: './overview.css',
})
export class Overview implements OnInit {
  readonly parent = inject(Details);
  private readonly dbService = inject(DatabaseService);
  private readonly projectService = inject(ProjectService);
  private readonly router = inject(Router);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly loadingDbs = signal(false);

  /** Days the app has actually existed (capped at 30), so we never render
   *  status bars for days before the resource was created. */
  getDaysActive(app: AppDetail): number {
    if (!app.created_at) return 1;
    const created = new Date(app.created_at).getTime();
    if (isNaN(created)) return 1;
    const days = Math.floor((Date.now() - created) / (1000 * 60 * 60 * 24)) + 1;
    return Math.max(1, Math.min(30, days));
  }

  /** One entry per real day of existence, oldest first. */
  getUptimeBars(app: AppDetail): { daysAgo: number; isToday: boolean }[] {
    const n = this.getDaysActive(app);
    const bars: { daysAgo: number; isToday: boolean }[] = [];
    for (let i = n - 1; i >= 0; i--) {
      bars.push({ daysAgo: i, isToday: i === 0 });
    }
    return bars;
  }

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

