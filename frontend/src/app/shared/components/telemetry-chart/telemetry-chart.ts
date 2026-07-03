import { Component, computed, input } from '@angular/core';

interface ChartPoint {
  x: number;
  y: number;
  t: number;
}

let nextGradientId = 0;

/**
 * Time-anchored telemetry line chart (Prometheus range data).
 *
 * Points are positioned by their real timestamp inside the selected window
 * [now - range, now] — a few hours of data on a 7d window occupies only its
 * actual slice and the rest stays empty. The line breaks wherever samples are
 * missing (pod down, app newer than the window) instead of interpolating
 * across the gap; isolated samples render as dots.
 *
 * Axis notations: Y max/mid labels (top-left overlay) and window start/mid/end
 * timestamps under the chart. All chrome uses the zinc CSS variables so it
 * follows the active theme; pass a theme-aware `color` (e.g.
 * `var(--color-zinc-50)`) for monochrome lines.
 */
@Component({
  selector: 'app-telemetry-chart',
  template: `
    <div class="w-full rounded overflow-hidden relative bg-zinc-950 border border-zinc-900" [style.height.px]="chartHeight()">
      @if (hasData()) {
        <svg class="absolute inset-0 w-full h-full" viewBox="0 0 500 150" preserveAspectRatio="none" aria-hidden="true">
          <defs>
            <linearGradient [attr.id]="gradId" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" [attr.stop-color]="color()" stop-opacity="0.14" />
              <stop offset="100%" [attr.stop-color]="color()" stop-opacity="0" />
            </linearGradient>
          </defs>
          <line x1="0" x2="500" y1="1" y2="1" stroke="var(--color-zinc-850)" stroke-width="1" />
          <line x1="0" x2="500" y1="75" y2="75" stroke="var(--color-zinc-850)" stroke-width="1" stroke-dasharray="4 5" />
          <line x1="0" x2="500" y1="149" y2="149" stroke="var(--color-zinc-850)" stroke-width="1" />
          <path [attr.d]="fillPath()" [attr.fill]="'url(#' + gradId + ')'"></path>
          <path [attr.d]="linePath()" fill="none" [attr.stroke]="color()" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"></path>
          @for (p of dots(); track p.t) {
            <circle [attr.cx]="p.x" [attr.cy]="p.y" r="2.4" [attr.fill]="color()"></circle>
          }
        </svg>
        <span class="absolute top-1 left-1.5 text-[10px] font-mono text-zinc-550 leading-none pointer-events-none select-none">{{ yTopLabel() }}</span>
        <span class="absolute top-1/2 -translate-y-1/2 left-1.5 text-[10px] font-mono text-zinc-600 leading-none pointer-events-none select-none">{{ yMidLabel() }}</span>
        <span class="absolute bottom-1 left-1.5 text-[10px] font-mono text-zinc-650 leading-none pointer-events-none select-none">0</span>
      } @else {
        <div class="h-full flex items-center justify-center text-xs text-zinc-650 font-mono">No data for this interval</div>
      }
    </div>
    <div class="flex items-center justify-between mt-1 px-0.5 text-[10px] font-mono text-zinc-600 leading-none select-none">
      <span>{{ xLabels()[0] }}</span>
      <span>{{ xLabels()[1] }}</span>
      <span>{{ xLabels()[2] }}</span>
    </div>
  `,
})
export class TelemetryChartComponent {
  /** Unix timestamps (seconds), parallel to `values`. */
  readonly timestamps = input<number[]>([]);
  readonly values = input<number[]>([]);
  /** Selected window: '1h' | '24h' | '7d'. Must match the queried range. */
  readonly range = input<string>('1h');
  /** Line colour — any CSS color, ideally a theme variable. */
  readonly color = input<string>('var(--color-zinc-50)');
  /** Unit suffix for Y-axis labels (e.g. 'm', 'MB', 'KB/s', '%'). */
  readonly unit = input<string>('');
  readonly chartHeight = input<number>(120);

  readonly gradId = `tchart-grad-${nextGradientId++}`;

  private rangeSec(): number {
    switch (this.range()) {
      case '24h': return 86400;
      case '7d': return 604800;
      default: return 3600;
    }
  }

  /** Expected sample step — mirrors the backend's query_range step per range. */
  private stepSec(): number {
    switch (this.range()) {
      case '24h': return 900;
      case '7d': return 7200;
      default: return 60;
    }
  }

  /** Chart window. Re-evaluated whenever fresh data lands (inputs change). */
  readonly window = computed(() => {
    this.timestamps(); // dependency: advance the window on every refresh
    const end = Date.now() / 1000;
    return { start: end - this.rangeSec(), end };
  });

  readonly yMax = computed(() => Math.max(...this.values(), 0.001) * 1.1);

  private points = computed<ChartPoint[]>(() => {
    const ts = this.timestamps();
    const vs = this.values();
    const { start, end } = this.window();
    const span = end - start;
    const max = this.yMax();
    const n = Math.min(ts.length, vs.length);
    const out: ChartPoint[] = [];
    for (let i = 0; i < n; i++) {
      if (ts[i] < start || ts[i] > end + 1) continue;
      const x = ((ts[i] - start) / span) * 500;
      const y = 150 - (Math.max(0, vs[i]) / max) * 150;
      out.push({ x, y, t: ts[i] });
    }
    return out;
  });

  /** Contiguous runs of samples: split where a gap exceeds 2.5 expected steps. */
  private segments = computed<ChartPoint[][]>(() => {
    const maxGap = this.stepSec() * 2.5;
    const segs: ChartPoint[][] = [];
    let cur: ChartPoint[] = [];
    for (const p of this.points()) {
      if (cur.length > 0 && p.t - cur[cur.length - 1].t > maxGap) {
        segs.push(cur);
        cur = [];
      }
      cur.push(p);
    }
    if (cur.length > 0) segs.push(cur);
    return segs;
  });

  readonly hasData = computed(() => this.points().length > 0);

  readonly linePath = computed(() =>
    this.segments()
      .filter((s) => s.length > 1)
      .map((seg) => seg.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x.toFixed(1)} ${p.y.toFixed(1)}`).join(' '))
      .join(' '),
  );

  readonly fillPath = computed(() =>
    this.segments()
      .filter((s) => s.length > 1)
      .map((seg) => {
        const line = seg.map((p, i) => `${i === 0 ? 'M' : 'L'} ${p.x.toFixed(1)} ${p.y.toFixed(1)}`).join(' ');
        const first = seg[0];
        const last = seg[seg.length - 1];
        return `${line} L ${last.x.toFixed(1)} 150 L ${first.x.toFixed(1)} 150 Z`;
      })
      .join(' '),
  );

  /** Samples with no neighbour close enough to draw a line to. */
  readonly dots = computed(() => this.segments().filter((s) => s.length === 1).map((s) => s[0]));

  readonly yTopLabel = computed(() => this.formatValue(this.yMax()));
  readonly yMidLabel = computed(() => this.formatValue(this.yMax() / 2));

  readonly xLabels = computed(() => {
    const { start, end } = this.window();
    return [start, (start + end) / 2, end].map((t) => this.formatTime(t));
  });

  private formatValue(v: number): string {
    const unit = this.unit();
    let num: string;
    if (v >= 1_000_000) num = `${(v / 1_000_000).toFixed(1)}M`;
    else if (v >= 10_000) num = `${(v / 1000).toFixed(0)}k`;
    else if (v >= 1000) num = `${(v / 1000).toFixed(1)}k`;
    else if (v >= 100) num = v.toFixed(0);
    else if (v >= 1) num = v.toFixed(1);
    else num = v.toFixed(2);
    return unit ? `${num} ${unit}` : num;
  }

  private formatTime(t: number): string {
    const d = new Date(t * 1000);
    const hm = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
    if (this.range() === '7d') {
      return `${String(d.getDate()).padStart(2, '0')}.${String(d.getMonth() + 1).padStart(2, '0')} ${hm}`;
    }
    return hm;
  }
}
