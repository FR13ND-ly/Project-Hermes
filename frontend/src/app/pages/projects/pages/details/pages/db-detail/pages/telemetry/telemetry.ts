import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { DecimalPipe, NgClass } from '@angular/common';
import { DbDetailComponent } from '../../db-detail';
import { DatabaseService, DbMetricsHistory } from '../../../../../../../../core/services/database.service';

@Component({
  selector: 'app-db-telemetry',
  imports: [DecimalPipe, NgClass],
  templateUrl: './telemetry.html',
  styles: ``,
})
export class DbTelemetryComponent implements OnInit {
  readonly dbDetail = inject(DbDetailComponent);
  private readonly dbService = inject(DatabaseService);

  readonly dbRange = signal('1h');
  readonly dbMetricsLoading = signal(false);
  readonly dbMetricsSimulated = signal(false);
  readonly dbCpuValues = signal<number[]>([]);
  readonly dbMemValues = signal<number[]>([]);
  readonly dbSizeValues = signal<number[]>([]);
  readonly dbConnValues = signal<number[]>([]);
  readonly dbCacheValues = signal<number[]>([]);

  constructor() {
    effect(() => {
      const id = this.dbDetail.dbId();
      const range = this.dbRange();
      if (id) {
        this.loadDbMetrics(id, range);
      }
    });
  }

  ngOnInit(): void {
    // Initial fetch handled by effect
  }

  loadDbMetrics(id: string, range: string): void {
    this.dbMetricsLoading.set(true);

    this.dbService.getMetrics(id, 'cpu', range).subscribe({
      next: (res: DbMetricsHistory) => {
        this.dbCpuValues.set((res.values || []).map((v: number) => v * 1000)); // cores -> millicores
        this.dbMetricsSimulated.set(!!res.simulated);
        this.dbMetricsLoading.set(false);
      },
      error: () => { this.dbCpuValues.set([]); this.dbMetricsLoading.set(false); }
    });

    this.dbService.getMetrics(id, 'memory', range).subscribe({
      next: (res: DbMetricsHistory) => this.dbMemValues.set(res.values || []),
      error: () => this.dbMemValues.set([])
    });

    this.dbService.getMetrics(id, 'db_size', range).subscribe({
      next: (res: DbMetricsHistory) => this.dbSizeValues.set(res.values || []),
      error: () => this.dbSizeValues.set([])
    });

    this.dbService.getMetrics(id, 'db_connections', range).subscribe({
      next: (res: DbMetricsHistory) => this.dbConnValues.set(res.values || []),
      error: () => this.dbConnValues.set([])
    });

    const t = this.dbDetail.db()?.type;
    if (t === 'postgres' || t === 'redis') {
      this.dbService.getMetrics(id, 'db_cache_hit_rate', range).subscribe({
        next: (res: DbMetricsHistory) => this.dbCacheValues.set(res.values || []),
        error: () => this.dbCacheValues.set([])
      });
    } else {
      this.dbCacheValues.set([]);
    }
  }

  onDbRangeChange(range: string): void {
    this.dbRange.set(range);
  }

  cpuUsedPct(): number {
    const v = this.dbCpuValues();
    const cur = v.length > 0 ? v[v.length - 1] : 0;
    const limit = this.dbDetail.db()?.cpuLimit || 0;
    return limit > 0 ? Math.min(100, Math.round((cur / limit) * 100)) : 0;
  }

  memUsedPct(): number {
    const v = this.dbMemValues();
    const cur = v.length > 0 ? v[v.length - 1] : 0;
    const limit = this.dbDetail.db()?.memoryLimitMb || 0;
    return limit > 0 ? Math.min(100, Math.round((cur / limit) * 100)) : 0;
  }

  lastVal(values: number[]): number {
    return values.length > 0 ? values[values.length - 1] : 0;
  }

  getSvgPath(values: number[]): string {
    if (values.length < 2) return '';
    const width = 500;
    const height = 150;
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
