import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { ServerlessDetailComponent } from '../../detail';
import { ProjectService, ServerlessRoute } from '../../../../../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../../../../../core/services/toast.service';
import { ConfirmService } from '../../../../../../../../../../core/services/confirm.service';

declare const monaco: any;

@Component({
  selector: 'app-serverless-routes',
  imports: [FormsModule],
  templateUrl: './routes.html',
  styles: ``,
})
export class ServerlessRoutesComponent implements OnInit, OnDestroy {
  readonly detailParent = inject(ServerlessDetailComponent);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly confirm = inject(ConfirmService);

  readonly routes = signal<ServerlessRoute[]>([]);
  readonly selectedRoute = signal<ServerlessRoute | null>(null);
  readonly routeMethod = signal('GET');
  readonly routePath = signal('');
  readonly routeCode = signal('');
  readonly savingRoute = signal(false);

  editorInstance: any = null;
  readonly httpMethods = ['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'ANY'];

  constructor() {
    effect(() => {
      const id = this.detailParent.functionId();
      if (id) {
        this.loadRoutes();
      }
    });
  }

  ngOnInit(): void {
    // Handled by effect
  }

  ngOnDestroy(): void {
    if (this.editorInstance) {
      this.editorInstance.dispose();
      this.editorInstance = null;
    }
  }

  loadRoutes(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listRoutes(projId, inst.id).subscribe({
      next: (res) => this.routes.set(res || []),
      error: () => {}
    });
  }

  onAddRoute(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.createRoute(projId, inst.id, { method: 'GET', routePath: '/noua-ruta-' + (this.routes().length + 1) }).subscribe({
      next: (r) => {
        this.toast.success('Route added. Edit it and then Deploy.');
        this.loadRoutes();
        this.selectRoute(r);
      },
      error: (err: any) => this.toast.error(err.error?.message || 'Failed to add route.')
    });
  }

  selectRoute(r: ServerlessRoute): void {
    this.selectedRoute.set(r);
    this.routeMethod.set(r.method);
    this.routePath.set(r.routePath);
    this.routeCode.set(r.code);
    setTimeout(() => this.mountEditor(), 50);
  }

  closeRouteEditor(): void {
    this.selectedRoute.set(null);
    if (this.editorInstance) {
      this.editorInstance.dispose();
      this.editorInstance = null;
    }
  }

  onSaveRoute(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    const r = this.selectedRoute();
    if (!projId || !inst || !r) return;
    const routePath = this.routePath().trim();
    if (!routePath) {
      this.toast.error('Route path is required.');
      return;
    }
    this.savingRoute.set(true);
    this.projectService.updateRoute(projId, inst.id, r.id, {
      method: this.routeMethod(),
      routePath,
      code: this.editorInstance ? this.editorInstance.getValue() : this.routeCode(),
    }).subscribe({
      next: (updated) => {
        this.savingRoute.set(false);
        this.selectedRoute.set(updated);
        this.toast.success('Route saved. Deploy to apply.');
        this.loadRoutes();
      },
      error: (err: any) => {
        this.savingRoute.set(false);
        this.toast.error(err.error?.message || 'Error saving route.');
      }
    });
  }

  async onDeleteRoute(r: ServerlessRoute): Promise<void> {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    const confirmed = await this.confirm.ask({
      title: 'Delete route',
      message: `Are you sure you want to delete route ${r.method} ${r.routePath}?`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true,
    });
    if (!confirmed) return;
    this.projectService.deleteRoute(projId, inst.id, r.id).subscribe({
      next: () => {
        this.toast.success('Route deleted.');
        if (this.selectedRoute()?.id === r.id) {
          this.closeRouteEditor();
        }
        this.loadRoutes();
      },
      error: (err: any) => this.toast.error(err.error?.message || 'Failed to delete.')
    });
  }

  private mountEditor(): void {
    if (typeof monaco !== 'undefined') {
      this.createEditor();
      return;
    }
    const loaderUrl = 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.48.0/min/vs/loader.min.js';
    if (!document.querySelector(`script[src="${loaderUrl}"]`)) {
      const script = document.createElement('script');
      script.src = loaderUrl;
      script.onload = () => {
        const req = (window as any).require;
        req.config({ paths: { vs: 'https://cdnjs.cloudflare.com/ajax/libs/monaco-editor/0.48.0/min/vs' } });
        req(['vs/editor/editor.main'], () => this.createEditor());
      };
      document.body.appendChild(script);
    } else {
      const i = setInterval(() => {
        if (typeof monaco !== 'undefined') {
          clearInterval(i);
          this.createEditor();
        }
      }, 100);
    }
  }

  private createEditor(): void {
    const container = document.getElementById('route-editor-container');
    if (!container) return;
    if (this.editorInstance) {
      this.editorInstance.dispose();
      this.editorInstance = null;
    }
    const language = (this.detailParent.selectedInstance()?.runtime || '').startsWith('python') ? 'python' : 'javascript';
    this.editorInstance = monaco.editor.create(container, {
      value: this.routeCode(),
      language,
      theme: 'vs-dark',
      automaticLayout: true,
      minimap: { enabled: false },
      fontSize: 12,
      fontFamily: 'Fira Code, JetBrains Mono, monospace',
      lineHeight: 20,
      tabSize: 2,
    });
    this.editorInstance.onDidChangeModelContent(() => this.routeCode.set(this.editorInstance.getValue()));
  }
}
