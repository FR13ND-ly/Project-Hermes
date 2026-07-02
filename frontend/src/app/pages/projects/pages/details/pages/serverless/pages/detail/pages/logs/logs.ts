import { Component, inject, signal, OnInit, OnDestroy, effect } from '@angular/core';

import { ServerlessDetailComponent } from '../../detail';
import { environment } from '../../../../../../../../../../../environments/environment';

@Component({
  selector: 'app-serverless-logs',
  imports: [],
  templateUrl: './logs.html',
  styles: ``,
})
export class ServerlessLogsComponent implements OnInit, OnDestroy {
  readonly detailParent = inject(ServerlessDetailComponent);

  readonly logs = signal<string[]>([]);
  private logSource: EventSource | null = null;

  constructor() {
    effect(() => {
      const id = this.detailParent.functionId();
      if (id) {
        this.startLogsStream();
      } else {
        this.stopLogsStream();
      }
    });
  }

  ngOnInit(): void {
    // Managed by effect
  }

  ngOnDestroy(): void {
    this.stopLogsStream();
  }

  startLogsStream(): void {
    const projId = this.detailParent.parent.projectId();
    const inst = this.detailParent.selectedInstance();
    if (!projId || !inst) return;

    this.stopLogsStream();
    this.logs.set(['[Console] Connecting to logs stream...']);

    const url = `${environment.apiBaseUrl}/projects/${projId}/serverless/${inst.id}/logs/stream?token=${encodeURIComponent(localStorage.getItem('hermes_token') || '')}`;
    this.logSource = new EventSource(url);

    this.logSource.onmessage = (event) => {
      if (event.data) {
        this.logs.update(lines => {
          const next = [...lines, event.data];
          if (next.length > 500) next.shift();
          return next;
        });
      }
    };

    this.logSource.onerror = () => {
      // Knative scale-to-zero can terminate the pod, causing a disconnect. This is normal.
    };
  }

  stopLogsStream(): void {
    if (this.logSource) {
      this.logSource.close();
      this.logSource = null;
    }
  }
}
