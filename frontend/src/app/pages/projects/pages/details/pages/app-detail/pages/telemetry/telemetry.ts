import { Component, inject, OnInit, OnDestroy, effect } from '@angular/core';
import { CommonModule, DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-telemetry',
  standalone: true,
  imports: [CommonModule, DecimalPipe, FormsModule],
  templateUrl: './telemetry.html',
  styles: ``,
})
export class AppTelemetryComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);

  constructor() {
    effect(() => {
      const instId = this.parent.activeInstanceId();
      const range = this.parent.selectedRange();
      if (instId) {
        this.parent.loadMetrics();
        if (range === '1h') {
          this.parent.connectTelemetry(instId);
        } else {
          this.parent.disconnectTelemetry();
        }
      }
    });
  }

  ngOnInit(): void {
    const instId = this.parent.activeInstanceId();
    if (instId) {
      this.parent.loadMetrics();
      if (this.parent.selectedRange() === '1h') {
        this.parent.connectTelemetry(instId);
      }
    }
  }

  ngOnDestroy(): void {
    this.parent.disconnectTelemetry();
  }
}
