import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { DecimalPipe, NgClass } from '@angular/common';
import { DbDetailComponent } from '../../db-detail';
import { DatabaseService, DbMetricsHistory } from '../../../../../../../../core/services/database.service';
import { TelemetryChartComponent } from '../../../../../../../../shared/components/telemetry-chart/telemetry-chart';

@Component({
  selector: 'app-db-telemetry',
  imports: [DecimalPipe, NgClass, TelemetryChartComponent],
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
  readonly dbCpuTs = signal<number[]>([]);
  readonly dbMemValues = signal<number[]>([]);
  readonly dbMemTs = signal<number[]>([]);
  readonly dbSizeValues = signal<number[]>([]);
  readonly dbSizeTs = signal<number[]>([]);
  readonly dbConnValues = signal<number[]>([]);
  readonly dbConnTs = signal<number[]>([]);
  readonly dbCacheValues = signal<number[]>([]);
  readonly dbCacheTs = signal<number[]>([]);

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
        this.dbCpuTs.set(res.timestamps || []);
        this.dbMetricsSimulated.set(!!res.simulated);
        this.dbMetricsLoading.set(false);
      },
      error: () => { this.dbCpuValues.set([]); this.dbCpuTs.set([]); this.dbMetricsLoading.set(false); }
    });

    this.dbService.getMetrics(id, 'memory', range).subscribe({
      next: (res: DbMetricsHistory) => { this.dbMemValues.set(res.values || []); this.dbMemTs.set(res.timestamps || []); },
      error: () => { this.dbMemValues.set([]); this.dbMemTs.set([]); }
    });

    this.dbService.getMetrics(id, 'db_size', range).subscribe({
      next: (res: DbMetricsHistory) => { this.dbSizeValues.set(res.values || []); this.dbSizeTs.set(res.timestamps || []); },
      error: () => { this.dbSizeValues.set([]); this.dbSizeTs.set([]); }
    });

    this.dbService.getMetrics(id, 'db_connections', range).subscribe({
      next: (res: DbMetricsHistory) => { this.dbConnValues.set(res.values || []); this.dbConnTs.set(res.timestamps || []); },
      error: () => { this.dbConnValues.set([]); this.dbConnTs.set([]); }
    });

    const t = this.dbDetail.db()?.type;
    if (t === 'postgres' || t === 'redis') {
      this.dbService.getMetrics(id, 'db_cache_hit_rate', range).subscribe({
        next: (res: DbMetricsHistory) => { this.dbCacheValues.set(res.values || []); this.dbCacheTs.set(res.timestamps || []); },
        error: () => { this.dbCacheValues.set([]); this.dbCacheTs.set([]); }
      });
    } else {
      this.dbCacheValues.set([]);
      this.dbCacheTs.set([]);
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
}
