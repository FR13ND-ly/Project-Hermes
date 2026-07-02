import { Component, inject, OnInit, OnDestroy, effect } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-logs',
  imports: [FormsModule],
  templateUrl: './logs.html',
  styles: ``,
})
export class AppLogsComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);

  constructor() {
    effect(() => {
      const instId = this.parent.activeInstanceId();
      const buildId = this.parent.selectedBuildId();
      if (instId && !buildId) {
        this.parent.connectLogs(instId);
      } else {
        this.parent.disconnectLogs();
      }
    });
  }

  ngOnInit(): void {
    const instId = this.parent.activeInstanceId();
    if (instId && !this.parent.selectedBuildId()) {
      this.parent.connectLogs(instId);
    }
  }

  ngOnDestroy(): void {
    this.parent.disconnectLogs();
  }
}
