import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { ActivatedRoute } from '@angular/router';
import { ServerlessDetailComponent } from '../../detail';
import { ProjectService, ServerlessBuild } from '../../../../../../../../../../core/services/project.service';
import { ToastService } from '../../../../../../../../../../core/services/toast.service';
import { Subscription } from 'rxjs';

@Component({
  selector: 'app-serverless-builds',
  standalone: true,
  imports: [CommonModule, DatePipe],
  templateUrl: './builds.html',
  styles: ``,
})
export class ServerlessBuildsComponent implements OnInit, OnDestroy {
  readonly detailParent = inject(ServerlessDetailComponent);
  private readonly projectService = inject(ProjectService);
  private readonly toast = inject(ToastService);
  private readonly route = inject(ActivatedRoute);

  readonly builds = signal<ServerlessBuild[]>([]);
  readonly selectedBuildId = signal<string | null>(null);
  readonly buildLogs = signal<string[]>([]);
  private buildLogSource: EventSource | null = null;
  private routeSubscription: Subscription | null = null;

  constructor() {
    effect(() => {
      const id = this.detailParent.functionId();
      if (id) {
        this.loadBuilds();
      }
    });
  }

  ngOnInit(): void {
    this.routeSubscription = this.route.queryParams.subscribe(params => {
      const bId = params['buildId'];
      if (bId) {
        this.selectedBuildId.set(bId);
        this.startBuildLogsStream(bId);
      }
    });
  }

  ngOnDestroy(): void {
    this.stopBuildLogsStream();
    if (this.routeSubscription) {
      this.routeSubscription.unsubscribe();
    }
  }

  loadBuilds(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    this.projectService.listFunctionBuilds(projId, inst.id).subscribe({
      next: (res) => this.builds.set(res || []),
      error: () => this.builds.set([])
    });
  }

  selectBuild(build: ServerlessBuild): void {
    this.selectedBuildId.set(build.id);
    this.startBuildLogsStream(build.id);
  }

  startBuildLogsStream(buildId: string): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;

    this.stopBuildLogsStream();
    this.buildLogs.set(['[Console] Connecting to build logs stream...']);

    const url = this.projectService.getFunctionBuildLogsStreamUrl(projId, inst.id, buildId);
    this.buildLogSource = new EventSource(url);

    this.buildLogSource.onmessage = (event) => {
      if (event.data) {
        this.buildLogs.update(lines => {
          const next = [...lines, event.data];
          if (next.length > 1000) next.shift();
          return next;
        });
      }
    };

    this.buildLogSource.onerror = () => {
      this.stopBuildLogsStream();
      this.loadBuilds();
    };
  }

  stopBuildLogsStream(): void {
    if (this.buildLogSource) {
      this.buildLogSource.close();
      this.buildLogSource = null;
    }
  }
}
