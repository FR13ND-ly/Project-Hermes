import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { ActivatedRoute, Router, RouterLink, RouterOutlet, RouterLinkActive } from '@angular/router';
import { Details } from '../../../../details';
import { ProjectService, ServerlessInstance } from '../../../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../../../core/services/toast.service';
import { WebSocketService } from '../../../../../../../../core/services/websocket.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-serverless-detail',
  standalone: true,
  imports: [CommonModule, RouterLink, RouterOutlet, RouterLinkActive],
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
  }

  ngOnDestroy(): void {
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
