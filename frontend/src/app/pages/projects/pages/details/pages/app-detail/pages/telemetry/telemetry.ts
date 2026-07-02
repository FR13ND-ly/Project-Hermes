import { Component, inject, OnDestroy, effect } from '@angular/core';
import { DecimalPipe, NgClass } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-telemetry',
  imports: [DecimalPipe, FormsModule, NgClass],
  templateUrl: './telemetry.html',
  styles: ``,
})
export class AppTelemetryComponent implements OnDestroy {
  readonly parent = inject(AppDetailComponent);
  private refreshTimer?: any;

  constructor() {
    // Single source of truth: real Prometheus data for the selected window. Reload (and
    // restart the silent auto-refresh) whenever the instance or range changes. Previously
    // a live SSE stream appended samples into the same arrays, so the chart drifted from
    // "the selected range" into a rolling few-minute buffer with a different CPU scale —
    // which is exactly why the data "changed after a few seconds" and felt unreal.
    effect(() => {
      const instId = this.parent.activeInstanceId();
      const range = this.parent.selectedRange();
      if (!instId) {
        this.stopRefresh();
        return;
      }
      this.parent.loadMetrics();
      this.startRefresh(range);
    });
  }

  ngOnDestroy(): void {
    this.stopRefresh();
    // Close any legacy live stream that might still be open from an older code path.
    this.parent.disconnectTelemetry();
  }

  /** Re-query Prometheus on an interval sized to the range (finer windows refresh more
   *  often), silently so the loading overlay doesn't flash on every tick. */
  private startRefresh(range: string): void {
    this.stopRefresh();
    const everyMs = range === '1h' ? 15000 : range === '24h' ? 60000 : 300000;
    this.refreshTimer = setInterval(() => this.parent.loadMetrics(true), everyMs);
  }

  private stopRefresh(): void {
    if (this.refreshTimer) {
      clearInterval(this.refreshTimer);
      this.refreshTimer = undefined;
    }
  }
}
