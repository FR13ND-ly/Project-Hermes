import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { DecimalPipe } from '@angular/common';
import { ServerlessDetailComponent } from '../../detail';
import { ProjectService } from '../../../../../../../../../../core/services/project.service';

@Component({
  selector: 'app-serverless-metrics',
  imports: [DecimalPipe],
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
  readonly memValues = signal<number[]>([]);

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
        this.metricsSimulated.set(!!res.simulated);
        this.metricsLoading.set(false);
      },
      error: () => { this.cpuValues.set([]); this.metricsLoading.set(false); }
    });
    this.projectService.getInstanceMetrics(projId, inst.id, 'memory', range).subscribe({
      next: (res: any) => this.memValues.set(res.values || []),
      error: () => this.memValues.set([])
    });
  }

  onMetricsRangeChange(range: string): void {
    this.metricsRange.set(range);
    this.loadMetrics();
  }

  lastVal(values: number[]): number {
    return values.length > 0 ? values[values.length - 1] : 0;
  }

  getSvgPath(values: number[]): string {
    if (values.length < 2) return '';
    const width = 500, height = 150;
    const max = Math.max(...values, 0.1) * 1.1;
    const min = Math.min(...values, 0);
    const span = (max - min) || 1;
    return values.map((val, idx) => {
      const x = (idx / (values.length - 1)) * width;
      const y = height - ((val - min) / span) * height;
      return `${idx === 0 ? 'M' : 'L'} ${x.toFixed(1)} ${y.toFixed(1)}`;
    }).join(' ');
  }

  getSvgFillPath(values: number[]): string {
    const linePath = this.getSvgPath(values);
    if (!linePath) return '';
    return `${linePath} L 500 150 L 0 150 Z`;
  }
}
