import { Component, inject, signal, computed, OnInit, OnDestroy, effect } from '@angular/core';

import { ActivatedRoute, Router, RouterLink, RouterOutlet, RouterLinkActive, NavigationEnd } from '@angular/router';
import { Details } from '../../../../details';
import { ProjectService, ServerlessInstance } from '../../../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../../../core/services/toast.service';
import { WebSocketService } from '../../../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-serverless-detail',
  imports: [RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './detail.html',
  styles: ``,
})
export class ServerlessDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly route = inject(ActivatedRoute);
  private readonly router = inject(Router);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly wsService = inject(WebSocketService);

  readonly functionId = signal<string | null>(null);
  readonly selectedInstance = signal<ServerlessInstance | null>(null);
  readonly loading = signal(false);
  readonly deploying = signal(false);

  // Two-tier tab nav: groups the 7 flat routes by purpose (URLs unchanged).
  readonly tabGroups: { id: string; label: string; default: string; tabs: { path: string; label: string }[] }[] = [
    { id: 'details', label: 'Overview', default: 'details', tabs: [{ path: 'details', label: 'Overview' }] },
    { id: 'observability', label: 'Observability', default: 'metrics', tabs: [
      { path: 'metrics', label: 'Metrics' },
      { path: 'logs', label: 'Logs' },
    ] },
    { id: 'routes', label: 'Routes', default: 'routes', tabs: [{ path: 'routes', label: 'Routes' }] },
    { id: 'builds', label: 'Builds', default: 'builds', tabs: [{ path: 'builds', label: 'Builds' }] },
    { id: 'settings', label: 'Settings', default: 'settings', tabs: [
      { path: 'settings', label: 'General' },
      { path: 'env', label: 'Env Variables' },
    ] },
  ];
  readonly currentTabPath = signal<string>('details');
  readonly activeGroup = computed(
    () => this.tabGroups.find((g) => g.tabs.some((t) => t.path === this.currentTabPath())) ?? this.tabGroups[0],
  );
  private navSub?: Subscription;
  private leafTabPath(): string {
    const clean = this.router.url.split('?')[0].split('#')[0];
    return clean.split('/').filter(Boolean).pop() ?? 'details';
  }

  private wsSubscriptions = new Subscription();

  constructor() {
    effect(() => {
      const id = this.functionId();
      if (id) {
        this.loadFunctionDetails(id);
      }
    });
  }

  ngOnInit(): void {
    const id = this.route.snapshot.paramMap.get('functionId');
    if (id) {
      this.functionId.set(id);
    }

    this.wsSubscriptions.add(this.wsService.onEvent<any>('serverless_function_updated').subscribe(payload => {
      if (payload.function_id === this.functionId()) {
        this.loadFunctionDetails(payload.function_id, true);
      }
    }));

    this.currentTabPath.set(this.leafTabPath());
    this.navSub = this.router.events.subscribe((e) => {
      if (e instanceof NavigationEnd) this.currentTabPath.set(this.leafTabPath());
    });
  }

  ngOnDestroy(): void {
    this.navSub?.unsubscribe();
    this.wsSubscriptions.unsubscribe();
  }

  loadFunctionDetails(id: string, silent = false): void {
    const projId = this.parent.projectId();
    if (!projId) return;

    if (!silent) this.loading.set(true);
    this.projectService.getFunctionDetails(projId, id).subscribe({
      next: (res) => {
        this.selectedInstance.set(res);
        this.loading.set(false);
      },
      error: (err: any) => {
        this.toast.error(err.error?.message || 'Failed to load serverless function details.');
        this.loading.set(false);
      }
    });
  }

  onDeployInstance(): void {
    const projId = this.parent.projectId();
    const inst = this.selectedInstance();
    if (!projId || !inst) return;
    if ((inst.routes || []).length === 0) {
      this.toast.error('Add at least one route before deploying.');
      return;
    }
    this.deploying.set(true);
    this.projectService.deployFunction(projId, inst.id).subscribe({
      next: (res) => {
        this.deploying.set(false);
        this.toast.success('Build and deployment initiated!');
        this.selectedInstance.set({ ...inst, status: 'building' });
        if (res?.buildId) {
          this.router.navigate(['builds'], { relativeTo: this.route, queryParams: { buildId: res.buildId } });
        }
      },
      error: (err: any) => {
        this.deploying.set(false);
        this.toast.error(err.error?.message || 'Error deploying.');
      }
    });
  }
}
