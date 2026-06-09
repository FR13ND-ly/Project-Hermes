import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { DatePipe } from '@angular/common';
import { Details } from '../../details';
import { ProjectService, AppBuild } from '../../../../../../core/services/project.service';

@Component({
  selector: 'app-deployments',
  imports: [DatePipe],
  templateUrl: './deployments.html',
  styleUrl: './deployments.css',
})
export class Deployments implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);

  readonly builds = signal<AppBuild[]>([]);
  readonly buildsLoading = signal(false);
  
  readonly selectedBuildId = signal<string | null>(null);
  readonly selectedBuildLogs = signal<string>('');
  readonly loadingBuildLogs = signal(false);
  
  readonly logs = signal<string[]>([]);
  readonly sseConnected = signal(false);
  readonly autoScroll = signal(true);

  private eventSource: EventSource | null = null;
  private connectedInstanceId: string | null = null;

  constructor() {
    // Conectează automat fluxul de logs când prima instanță este disponibilă
    effect(() => {
      const app = this.parent.appDetail();
      if (app && app.instances?.length > 0) {
        const firstInstanceId = app.instances[0].id;
        if (this.connectedInstanceId !== firstInstanceId) {
          this.connectLogs(firstInstanceId);
        }
      } else {
        this.disconnectLogs();
      }
    });
  }

  ngOnInit(): void {
    this.loadBuilds();
  }

  ngOnDestroy(): void {
    this.disconnectLogs();
  }

  loadBuilds(): void {
    const appId = this.parent.appDetail()?.id;
    if (!appId) return;

    this.buildsLoading.set(true);
    this.projectService.listBuilds(appId).subscribe({
      next: (res) => {
        this.builds.set(res || []);
        this.buildsLoading.set(false);
      },
      error: () => {
        this.builds.set([]);
        this.buildsLoading.set(false);
      }
    });
  }

  connectLogs(instanceId: string): void {
    const appId = this.parent.appDetail()?.id;
    if (!appId) return;

    this.disconnectLogs();
    this.connectedInstanceId = instanceId;
    this.logs.set(['[Console] Se conectează la fluxul de logs Kubernetes...']);

    const streamUrl = this.projectService.getLogsStreamUrl(appId, instanceId);
    this.eventSource = new EventSource(streamUrl);

    this.eventSource.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] Conexiune stabilă. Recepționare logs în timp real:']);
    };

    this.eventSource.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => [...lines, event.data]);
        if (this.autoScroll()) {
          this.scrollToBottom();
        }
      }
    };

    this.eventSource.onerror = () => {
      this.sseConnected.set(false);
      this.logs.update(lines => [...lines, '[Aviz] Conexiunea la stream a fost întreruptă. Se încearcă reconectarea...']);
      this.disconnectLogs();
    };
  }

  disconnectLogs(): void {
    if (this.eventSource) {
      this.eventSource.close();
      this.eventSource = null;
    }
    this.connectedInstanceId = null;
    this.sseConnected.set(false);
  }

  toggleAutoScroll(): void {
    this.autoScroll.update(val => !val);
    if (this.autoScroll()) {
      this.scrollToBottom();
    }
  }

  onViewBuildLogs(build: AppBuild): void {
    this.selectedBuildId.set(build.id);
    this.loadingBuildLogs.set(true);
    this.disconnectLogs(); // Pause live container logs
    
    const appId = this.parent.appDetail()?.id;
    if (!appId) return;

    this.projectService.getBuildDetails(appId, build.id).subscribe({
      next: (res) => {
        this.selectedBuildLogs.set(res.logs || 'Nu există loguri înregistrate pentru acest build.');
        this.loadingBuildLogs.set(false);
      },
      error: () => {
        this.selectedBuildLogs.set('Eroare la încărcarea logurilor de build.');
        this.loadingBuildLogs.set(false);
      }
    });
  }

  onBackToLiveLogs(): void {
    this.selectedBuildId.set(null);
    this.selectedBuildLogs.set('');
    
    const app = this.parent.appDetail();
    if (app && app.instances?.length > 0) {
      this.connectLogs(app.instances[0].id);
    }
  }

  private scrollToBottom(): void {
    setTimeout(() => {
      const el = document.getElementById('terminal-log-box');
      if (el) {
        el.scrollTop = el.scrollHeight;
      }
    }, 50);
  }
}
