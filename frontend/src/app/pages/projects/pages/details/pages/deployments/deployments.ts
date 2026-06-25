import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { DatePipe } from '@angular/common';
import { Details } from '../../details';
import { ProjectService, AppBuild } from '../../../../../../core/services/project.service';
import { Pagination } from '../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../core/models/pagination';

@Component({
  selector: 'app-deployments',
  imports: [DatePipe, Pagination],
  templateUrl: './deployments.html',
  styleUrl: './deployments.css',
})
export class Deployments implements OnInit, OnDestroy {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);

  readonly builds = signal<AppBuild[]>([]);
  readonly buildsLoading = signal(false);
  readonly page = signal(1);
  readonly pageSize = signal(DEFAULT_PAGE_SIZE);
  readonly total = signal(0);
  
  readonly selectedBuildId = signal<string | null>(null);
  readonly selectedBuildLogs = signal<string>('');
  readonly loadingBuildLogs = signal(false);
  
  readonly logs = signal<string[]>([]);
  readonly sseConnected = signal(false);
  readonly autoScroll = signal(true);

  private logsSocket: WebSocket | null = null;
  private logsReconnectTimer: any = null;
  private connectedInstanceId: string | null = null;

  constructor() {
    // Auto-connect log stream when the first instance becomes available
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
    this.projectService.listBuilds(appId, this.page(), this.pageSize()).subscribe({
      next: (res) => {
        this.builds.set(res?.items || []);
        this.total.set(res?.total || 0);
        this.buildsLoading.set(false);
      },
      error: () => {
        this.builds.set([]);
        this.buildsLoading.set(false);
      }
    });
  }

  onPageChange(page: number): void {
    this.page.set(page);
    this.loadBuilds();
  }

  connectLogs(instanceId: string): void {
    const appId = this.parent.appDetail()?.id;
    if (!appId) return;

    this.disconnectLogs();
    this.connectedInstanceId = instanceId;
    this.logs.set(['[Console] Connecting to log stream (WebSocket)...']);

    const wsUrl = this.projectService.getLogsWsUrl(appId, instanceId);
    const socket = new WebSocket(wsUrl);
    this.logsSocket = socket;

    socket.onopen = () => {
      this.sseConnected.set(true);
      this.logs.update(lines => [...lines, '[Console] WebSocket connection established. Receiving real-time logs:']);
    };

    socket.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => [...lines, event.data as string]);
        if (this.autoScroll()) {
          this.scrollToBottom();
        }
      }
    };

    socket.onclose = () => {
      if (this.logsSocket !== socket) return;
      this.sseConnected.set(false);
      this.logs.update(lines => [...lines, '[Notice] Stream connection interrupted. Reconnecting...']);
      this.logsReconnectTimer = setTimeout(() => {
        if (this.logsSocket === socket && this.connectedInstanceId === instanceId) {
          this.connectLogs(instanceId);
        }
      }, 2500);
    };

    socket.onerror = () => {
      this.sseConnected.set(false);
    };
  }

  disconnectLogs(): void {
    if (this.logsReconnectTimer) {
      clearTimeout(this.logsReconnectTimer);
      this.logsReconnectTimer = null;
    }
    if (this.logsSocket) {
      const sock = this.logsSocket;
      this.logsSocket = null;
      try { sock.close(); } catch { /* ignore */ }
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
        this.selectedBuildLogs.set(res.logs || 'No logs recorded for this build.');
        this.loadingBuildLogs.set(false);
      },
      error: () => {
        this.selectedBuildLogs.set('Failed to load build logs.');
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
