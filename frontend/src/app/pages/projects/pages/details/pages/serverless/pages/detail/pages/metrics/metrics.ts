import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { ServerlessDetailComponent } from '../../detail';
import { ProjectService } from '../../../../../../../../../../core/services/project.service';
import { TelemetryChartComponent } from '../../../../../../../../../../shared/components/telemetry-chart/telemetry-chart';

@Component({
  selector: 'app-serverless-metrics',
  imports: [DecimalPipe, TelemetryChartComponent],
  templateUrl: './metrics.html',
  styles: ``,
})
export class ServerlessMetricsComponent implements OnInit {
  readonly detailParent = inject(ServerlessDetailComponent);
  private readonly projectService = inject(ProjectService);

  readonly metricsRange = signal('1h');
  readonly metricsLoading = signal(false);
  readonly metricsSimulated = signal(false);
  readonly cpuValues = signal<number[]>([]);
  readonly cpuTs = signal<number[]>([]);
  readonly memValues = signal<number[]>([]);
  readonly memTs = signal<number[]>([]);

  constructor() {
    effect(() => {
      const id = this.detailParent.functionId();
      if (id) {
        this.loadMetrics();
      }
    });
  }

  ngOnInit(): void {
    // Initial fetch handled by effect
  }

  loadMetrics(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;
    const range = this.metricsRange();
    this.metricsLoading.set(true);
    this.projectService.getInstanceMetrics(projId, inst.id, 'cpu', range).subscribe({
      next: (res: any) => {
        this.cpuValues.set((res.values || []).map((v: number) => v * 1000)); // cores -> millicores
        this.cpuTs.set(res.timestamps || []);
        this.metricsSimulated.set(!!res.simulated);
        this.metricsLoading.set(false);
      },
      error: () => { this.cpuValues.set([]); this.cpuTs.set([]); this.metricsLoading.set(false); }
    });
    this.projectService.getInstanceMetrics(projId, inst.id, 'memory', range).subscribe({
      next: (res: any) => { this.memValues.set(res.values || []); this.memTs.set(res.timestamps || []); },
      error: () => { this.memValues.set([]); this.memTs.set([]); }
    });
  }

  onMetricsRangeChange(range: string): void {
    this.metricsRange.set(range);
    this.loadMetrics();
  }

  lastVal(values: number[]): number {
    return values.length > 0 ? values[values.length - 1] : 0;
  }
}
