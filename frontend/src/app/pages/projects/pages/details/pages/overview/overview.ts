import { Component, inject, signal, OnInit } from '@angular/core';
import { CommonModule, NgClass, DatePipe } from '@angular/common';
import { Router, RouterLink } from '@angular/router';
import { Details } from '../../details';
import { DatabaseService, DatabaseServiceInfo } from '../../../../../../core/services/database.service';
import { AppDetail } from '../../../../../../core/services/project.service';

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
  private readonly router = inject(Router);

  readonly databases = signal<DatabaseServiceInfo[]>([]);
  readonly loadingDbs = signal(false);
  readonly uptimeBars = Array(30).fill(0);

  ngOnInit(): void {
    this.loadDatabases();
  }

  loadDatabases(): void {
    const projectId = this.parent.projectId();
    if (!projectId) return;

    this.loadingDbs.set(true);
    this.dbService.listDatabases(projectId).subscribe({
      next: (res) => {
        this.databases.set(res || []);
        this.loadingDbs.set(false);
      },
      error: () => {
        this.databases.set([]);
        this.loadingDbs.set(false);
      }
    });
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
