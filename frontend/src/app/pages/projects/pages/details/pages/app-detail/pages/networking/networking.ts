import { Component, inject, OnInit, OnDestroy, signal } from '@angular/core';
import { DecimalPipe, NgClass } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Subscription, interval } from 'rxjs';
import { startWith, switchMap } from 'rxjs/operators';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-networking',
  imports: [DecimalPipe, FormsModule, NgClass],
  templateUrl: './networking.html',
  styles: `
    .wire { position: relative; width: 100%; height: 2px; background: #27272a; border-radius: 9999px; overflow: hidden; }
    .wire.active::after {
      content: ''; position: absolute; top: 0; bottom: 0; width: 40%;
      background: linear-gradient(90deg, transparent, #34d399, transparent);
      animation: wireflow 1.5s linear infinite;
    }
    .wire.broken { background: #7f1d1d; }
    @keyframes wireflow { 0% { transform: translateX(-120%); } 100% { transform: translateX(320%); } }
  `,
})
export class AppNetworkingComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);

  // Signals (not plain fields): the app runs zoneless change detection, so async
  // mutations only refresh the view when they go through a signal.
  readonly data = signal<any>(null);
  readonly loading = signal(true);
  readonly error = signal<string | null>(null);

  private pollSub?: Subscription;

  ngOnInit(): void {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();

    if (appId && inst) {
      this.pollSub = interval(5000)
        .pipe(
          startWith(0),
          switchMap(() => this.parent.projectService.getNetworkObservability(appId, inst.id)),
        )
        .subscribe({
          next: (res) => {
            this.data.set(res);
            this.loading.set(false);
            this.error.set(null);
          },
          error: (err) => {
            console.error('Failed to load network stats', err);
            this.error.set('Failed to load live networking and pods status.');
            this.loading.set(false);
          },
        });
    } else {
      this.loading.set(false);
      this.error.set('No active application instance selected.');
    }
  }

  ngOnDestroy(): void {
    this.pollSub?.unsubscribe();
  }

  getTrafficClassRatio(val: number | undefined): number {
    const d = this.data();
    if (!d || !d.traffic || !d.traffic.requestRate || d.traffic.requestRate === 0) {
      return 0;
    }
    return ((val || 0) / d.traffic.requestRate) * 100;
  }

  /** Tailwind classes for a hop/health status dot. */
  dotClass(status: string): string {
    switch (status) {
      case 'ok': return 'bg-emerald-500';
      case 'degraded': return 'bg-amber-500 animate-pulse';
      case 'down': return 'bg-red-500 animate-pulse';
      default: return 'bg-zinc-700';
    }
  }

  /** Border accent for a hop card by status. */
  hopBorder(status: string): string {
    switch (status) {
      case 'ok': return 'border-emerald-900/40';
      case 'degraded': return 'border-amber-900/40';
      case 'down': return 'border-red-900/50';
      default: return 'border-zinc-900';
    }
  }

  /** Human-readable bytes/sec. */
  formatBps(bps: number | null | undefined): string {
    if (bps === null || bps === undefined) return 'N/A';
    if (bps < 1024) return `${bps.toFixed(0)} B/s`;
    if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
    return `${(bps / (1024 * 1024)).toFixed(2)} MB/s`;
  }

  /** Health of a single pod from its phase + ready ratio ("a/b"). */
  podHealth(pod: any): 'ok' | 'degraded' | 'down' {
    if (!pod || pod.status !== 'Running') return 'down';
    const [a, b] = String(pod.ready || '').split('/').map((n) => +n || 0);
    if (b > 0 && a === b) return 'ok';
    if (a > 0) return 'degraded';
    return 'down';
  }

  /** Short pod label: drop the long deployment prefix, keep the replica suffix. */
  podShortName(name: string): string {
    if (!name) return '?';
    const parts = name.split('-');
    return parts.length > 2 ? parts.slice(-2).join('-') : name;
  }

  /** Is any traffic flowing (HTTP rate or pod network throughput)? */
  hasTraffic(d: any): boolean {
    const t = d?.traffic;
    if (!t) return false;
    return (t.requestRate ?? 0) > 0 || (t.netRxBps ?? 0) > 0 || (t.netTxBps ?? 0) > 0;
  }

  /** A compact traffic figure for the client→entry wire. */
  trafficLabel(d: any): string {
    const t = d?.traffic;
    if (!t) return '';
    if (t.source === 'traefik') return `${(t.requestRate ?? 0).toFixed(2)} r/s`;
    if (t.source === 'network') return `↓${this.formatBps(t.netRxBps)}`;
    return '';
  }

  /** Count of ready pods (parsed from each pod's "a/b" ratio). */
  readyPods(d: any): number {
    return (d?.pods || []).filter((p: any) => this.podHealth(p) === 'ok').length;
  }
}
