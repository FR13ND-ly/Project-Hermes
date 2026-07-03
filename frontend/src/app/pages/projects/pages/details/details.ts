import { Component, inject, signal, OnInit, OnDestroy, computed } from '@angular/core';
import { ActivatedRoute, RouterLink, RouterOutlet, Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { forkJoin, Subscription } from 'rxjs';
import { ProjectService, AppDetail, Project } from '../../../../core/services/project.service';
import { ToastService } from '../../../../core/services/toast.service';
import { AuthService } from '../../../../core/services/auth';
import { WebSocketService } from '../../../../core/services/websocket.service';
import { BreadcrumbService } from '../../../../core/services/breadcrumb.service';

interface Crumb {
  label: string;
  link?: string;
}

@Component({
  selector: 'app-details',
  imports: [RouterLink, RouterOutlet, FormsModule],
  templateUrl: './details.html',
  styleUrl: './details.css',
})
export class Details implements OnInit, OnDestroy {
  private readonly route = inject(ActivatedRoute);
  private readonly projectService = inject(ProjectService);
  readonly router = inject(Router);
  readonly toast = inject(ToastService);
  readonly authService = inject(AuthService);
  private readonly wsService = inject(WebSocketService);
  private readonly breadcrumbSvc = inject(BreadcrumbService);

  private refreshInterval: any = null;
  private sub = new Subscription();

  readonly projectId = signal<string | null>(null);
  readonly project = signal<Project | null>(null);
  readonly apps = signal<AppDetail[]>([]);
  readonly selectedApp = signal<AppDetail | null>(null);
  readonly loading = signal(false);
  readonly error = signal<string | null>(null);

  // Computed wrapper so child components continue to function on parent.appDetail()
  readonly appDetail = computed(() => this.selectedApp());

  ngOnInit(): void {
    this.route.paramMap.subscribe(params => {
      const id = params.get('id');
      if (id) {
        this.projectId.set(id);
        this.loadDetails(id);
        this.startPolling(id);
      }
    });

    // Real-time WebSocket subscriptions
    const events = ['instance_status_changed', 'build_status_changed', 'database_status_changed', 'serverless_function_updated'];
    for (const evt of events) {
      this.sub.add(
        this.wsService.onEvent<any>(evt).subscribe(() => {
          const pid = this.projectId();
          if (pid) {
            this.loadDetails(pid, true);
          }
        })
      );
    }
  }

  private startPolling(id: string): void {
    if (this.refreshInterval) {
      clearInterval(this.refreshInterval);
    }
    this.refreshInterval = setInterval(() => {
      if (this.projectId() && !this.loading()) {
        this.loadDetails(this.projectId()!, true);
      }
    }, 30000);
  }

  ngOnDestroy(): void {
    this.sub.unsubscribe();
    if (this.refreshInterval) {
      clearInterval(this.refreshInterval);
    }
  }

  loadDetails(id: string, silent = false): void {
    if (!silent) {
      this.loading.set(true);
    }
    this.error.set(null);

    forkJoin({
      project: this.projectService.getProject(id),
      apps: this.projectService.listProjectApps(id, 1, 1000)
    }).subscribe({
      next: (res) => {
        this.project.set(res.project);
        const appsList = res.apps?.items || [];
        this.apps.set(appsList);

        const currentSelected = this.selectedApp();
        if (currentSelected && appsList.some(a => a.id === currentSelected.id)) {
          const updated = appsList.find(a => a.id === currentSelected.id);
          this.selectedApp.set(updated || null);
        } else if (appsList.length > 0) {
          if (!currentSelected) {
            this.selectedApp.set(appsList[0]);
          }
        } else {
          this.selectedApp.set(null);
        }
        
        if (!silent) {
          this.loading.set(false);
        }
      },
      error: (err) => {
        if (!silent) {
          this.error.set(err.error?.message || 'Failed to load project details.');
          this.loading.set(false);
        }
      }
    });
  }

  onSelectApp(appId: string): void {
    const found = this.apps().find(a => a.id === appId);
    if (found) {
      this.selectedApp.set(found);
      this.router.navigate([`/projects/${this.projectId()}/apps/${appId}`]);
    }
  }

  getAppStatus(app: AppDetail | null): 'RUNNING' | 'INACTIVE' | 'BUILDING' | 'FAILED' | 'CRASHED' | 'STOPPED' {
    if (!app || !app.instances || app.instances.length === 0) return 'INACTIVE';
    const status = app.instances[0].status;
    if (status === 'running') return 'RUNNING';
    if (status === 'building') return 'BUILDING';
    if (status === 'failed') return 'FAILED';
    if (status === 'crashed') return 'CRASHED';
    if (status === 'stopped') return 'STOPPED';
    return 'INACTIVE';
  }

  getAppStatusClass(app: AppDetail | null): string {
    const status = this.getAppStatus(app);
    switch (status) {
      case 'RUNNING':
        return 'bg-emerald-950/20 border-emerald-900/30 text-emerald-400';
      case 'BUILDING':
        return 'bg-amber-950/20 border-amber-900/30 text-amber-400 animate-pulse';
      case 'FAILED':
      case 'CRASHED':
        return 'bg-red-950/20 border-red-900/30 text-red-400';
      case 'STOPPED':
      case 'INACTIVE':
      default:
        return 'bg-zinc-955 border-zinc-900 text-zinc-500';
    }
  }

  getAppIndicatorClass(app: AppDetail | null): string {
    const status = this.getAppStatus(app);
    switch (status) {
      case 'RUNNING':
        return 'bg-emerald-500';
      case 'BUILDING':
        return 'bg-amber-500 animate-pulse';
      case 'FAILED':
      case 'CRASHED':
        return 'bg-red-500';
      case 'STOPPED':
      case 'INACTIVE':
      default:
        return 'bg-zinc-500';
    }
  }

  get isInAppDetailContext(): boolean {
    const urlParts = this.router.url.split('?')[0].split('/');
    return urlParts.length >= 5 && urlParts[3] === 'apps' && !!urlParts[4] && urlParts[4] !== 'create';
  }

  /** Section slug (URL) → breadcrumb label. */
  private static readonly SECTION_LABELS: Record<string, string> = {
    apps: 'Applications',
    databases: 'Databases',
    storages: 'Storage',
    serverless: 'Serverless',
    cron: 'Cron Jobs',
    networking: 'Networking',
    environments: 'Environments',
    settings: 'Settings',
    'auth-management': 'Authentication',
    incidents: 'Incidents',
  };

  /**
   * Full breadcrumb trail derived from the current URL:
   * Projects / {project} [/ {section} [/ {resource-name} | Create]].
   * Resource names come from the loaded apps list (apps) or from the
   * BreadcrumbService, which the resource-detail page fills once its data
   * loads; until then an ellipsis placeholder shows. Sub-tabs (overview,
   * telemetry, …) are intentionally not part of the trail — they're visible
   * as tabs on the page itself.
   */
  get breadcrumbs(): Crumb[] {
    const id = this.projectId();
    const base = `/projects/${id}`;
    const crumbs: Crumb[] = [
      { label: 'Projects', link: '/projects' },
      { label: this.project()?.name || '…', link: base },
    ];

    const parts = this.router.url.split('?')[0].split('#')[0].split('/').filter(Boolean);
    // parts: ['projects', id, section?, resource?, ...tabs]
    const section = parts[2];
    if (!section) return crumbs;

    const sectionLabel = Details.SECTION_LABELS[section];
    if (!sectionLabel) return crumbs;
    crumbs.push({ label: sectionLabel, link: `${base}/${section}` });

    const resource = parts[3];
    if (!resource) return crumbs;

    if (resource === 'create') {
      crumbs.push({ label: 'Create' });
      return crumbs;
    }

    let name: string | null = null;
    if (section === 'apps') {
      name = this.apps().find(a => a.id === resource)?.name || null;
    }
    name = name || this.breadcrumbSvc.resourceName();
    crumbs.push({ label: name || '…', link: `${base}/${section}/${resource}` });
    return crumbs;
  }

  get activeTab(): string {
    const urlParts = this.router.url.split('/');
    return urlParts[3] || 'overview';
  }

  isTabActive(tab: string): boolean {
    const active = this.activeTab;
    if (tab === 'overview') {
      return active === 'overview' || active === '';
    }
    return active === tab;
  }

  tabClass(tab: string): string {
    // Same nav language as the global sidebar (.nav-item, indigo tint when active).
    return this.isTabActive(tab) ? 'nav-item is-active' : 'nav-item';
  }
}
