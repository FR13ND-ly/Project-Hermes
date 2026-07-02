import { Component, inject, OnInit, OnDestroy, signal, computed } from '@angular/core';
import { DatePipe, NgClass } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Subscription, interval, EMPTY } from 'rxjs';
import { startWith, switchMap, catchError } from 'rxjs/operators';
import { AppDetailComponent } from '../../app-detail';
import { ChaosExperiment, StartChaosRequest } from '../../../../../../../../core/services/project.service';
import { ConfirmService } from '../../../../../../../../core/services/confirm.service';
import { ToastService } from '../../../../../../../../core/services/toast.service';

export type ChaosKind = 'pod_kill' | 'scale_down' | 'cpu_stress';

@Component({
  selector: 'app-app-chaos',
  imports: [DatePipe, FormsModule, NgClass],
  templateUrl: './chaos.html',
  styles: ``,
})
export class AppChaosComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);
  private readonly confirm = inject(ConfirmService);
  private readonly toast = inject(ToastService);

  readonly experiments = signal<ChaosExperiment[]>([]);
  // Newest first, so the history reads top-down regardless of server ordering.
  readonly history = computed(() =>
    [...this.experiments()].sort((a, b) => new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime()),
  );
  readonly active = computed(() => this.experiments().find((e) => e.status === 'running') || null);
  readonly submitting = signal(false);
  readonly now = signal(Date.now());

  // Live cluster state (pods, replicas, traffic) polled straight into this page so the
  // blast radius is visible here — no need to jump to the Networking tab to see impact.
  readonly net = signal<any>(null);

  // Experiment form
  kind: ChaosKind = 'pod_kill';
  durationSec = 60;
  targetAllPods = false;
  targetReplicas = 0;
  cpuWorkers = 1;

  readonly durationPresets = [30, 60, 120, 300];

  private pollSub?: Subscription;
  private netSub?: Subscription;
  private ticker?: any;

  ngOnInit(): void {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();
    if (appId && inst) {
      this.pollSub = interval(4000)
        .pipe(
          startWith(0),
          switchMap(() =>
            this.parent.projectService.getChaos(appId, inst.id).pipe(
              // A transient poll failure must NOT tear down the interval — with a bare
              // switchMap an inner error completes the whole stream and freezes live
              // updates. Swallow the tick and keep polling on the next interval.
              catchError(() => EMPTY),
            ),
          ),
        )
        .subscribe({ next: (res) => this.experiments.set(res) });

      // Faster cadence (2.5s) than the experiment poll so pods dying and rescheduling
      // are actually visible frame-to-frame while a fault is landing.
      this.netSub = interval(2500)
        .pipe(
          startWith(0),
          switchMap(() =>
            this.parent.projectService.getNetworkObservability(appId, inst.id).pipe(catchError(() => EMPTY)),
          ),
        )
        .subscribe({ next: (res) => this.net.set(res) });
    }
    this.ticker = setInterval(() => this.now.set(Date.now()), 1000);
  }

  ngOnDestroy(): void {
    this.pollSub?.unsubscribe();
    this.netSub?.unsubscribe();
    if (this.ticker) clearInterval(this.ticker);
  }

  private reload(): void {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();
    if (appId && inst) {
      this.parent.projectService.getChaos(appId, inst.id).subscribe({ next: (r) => this.experiments.set(r) });
    }
  }

  kindLabel(k: string): string {
    switch (k) {
      case 'pod_kill': return 'Pod kill';
      case 'scale_down': return 'Scale-down';
      case 'cpu_stress': return 'CPU stress';
      default: return k;
    }
  }

  kindIcon(k: string): string {
    switch (k) {
      case 'pod_kill': return '💥';
      case 'scale_down': return '📉';
      case 'cpu_stress': return '🔥';
      default: return '⚗️';
    }
  }

  /** Seconds remaining until a running experiment auto-reverts. */
  countdown(e: ChaosExperiment): number {
    if (!e.revertAt) return 0;
    return Math.max(0, Math.round((new Date(e.revertAt).getTime() - this.now()) / 1000));
  }

  /** Total auto-revert window length in seconds (from params, else start→revert). */
  windowSec(e: ChaosExperiment): number {
    if (e.params?.duration_sec) return e.params.duration_sec;
    if (e.revertAt) {
      return Math.round((new Date(e.revertAt).getTime() - new Date(e.startedAt).getTime()) / 1000);
    }
    return 0;
  }

  /** Percentage of the auto-revert window elapsed (0–100), for the live progress bar. */
  progressPct(e: ChaosExperiment): number {
    if (!e.revertAt) return 0;
    const start = new Date(e.startedAt).getTime();
    const end = new Date(e.revertAt).getTime();
    if (end <= start) return 100;
    return Math.min(100, Math.max(0, ((this.now() - start) / (end - start)) * 100));
  }

  /** Compact per-experiment parameter detail for the history rows. */
  detail(e: ChaosExperiment): string {
    const p = e.params || {};
    switch (e.kind) {
      case 'pod_kill': {
        const n = Array.isArray(p.killed) ? p.killed.length : (p.all ? 'all' : 1);
        return `killed ${n} pod${n === 1 ? '' : 's'}`;
      }
      case 'scale_down': return `→ ${p.target_replicas ?? 0} replicas · ${p.duration_sec ?? 0}s`;
      case 'cpu_stress': return `${p.cpu_workers ?? 1} worker(s) · ${p.duration_sec ?? 0}s`;
      default: return '';
    }
  }

  setDuration(n: number): void {
    this.durationSec = n;
  }

  // --- Live impact helpers (pod tiles + replica gauge) ---

  /** Health bucket for a pod, from its phase + ready ratio ("a/b"). */
  podHealth(pod: any): 'ok' | 'degraded' | 'terminating' | 'down' {
    if (!pod) return 'down';
    if (pod.status === 'Terminating') return 'terminating';
    if (pod.status !== 'Running') return 'down';
    const [a, b] = String(pod.ready || '').split('/').map((n) => +n || 0);
    if (b > 0 && a === b) return 'ok';
    if (a > 0) return 'degraded';
    return 'down';
  }

  /** Status-dot colour for a pod tile. */
  podDot(pod: any): string {
    switch (this.podHealth(pod)) {
      case 'ok': return 'bg-emerald-500';
      case 'degraded': return 'bg-amber-500 animate-pulse';
      case 'terminating': return 'bg-zinc-500 animate-pulse';
      default: return 'bg-red-500 animate-pulse';
    }
  }

  /** Border accent for a pod tile by health. */
  podRing(pod: any): string {
    switch (this.podHealth(pod)) {
      case 'ok': return 'border-emerald-900/40';
      case 'degraded': return 'border-amber-900/40';
      case 'terminating': return 'border-zinc-800';
      default: return 'border-red-900/50';
    }
  }

  /** Short pod label: drop the long deployment prefix, keep the replica suffix. */
  podShortName(name: string): string {
    if (!name) return '?';
    const parts = name.split('-');
    return parts.length > 2 ? parts.slice(-2).join('-') : name;
  }

  /** Human-readable bytes/sec. */
  formatBps(bps: number | null | undefined): string {
    if (bps === null || bps === undefined) return 'N/A';
    if (bps < 1024) return `${bps.toFixed(0)} B/s`;
    if (bps < 1024 * 1024) return `${(bps / 1024).toFixed(1)} KB/s`;
    return `${(bps / (1024 * 1024)).toFixed(2)} MB/s`;
  }

  /** Compact traffic figure for the live-impact header. */
  trafficLabel(): string {
    const t = this.net()?.traffic;
    if (!t) return '';
    if (t.source === 'traefik') return `${(t.requestRate ?? 0).toFixed(2)} r/s`;
    if (t.source === 'network') return `↓${this.formatBps(t.netRxBps)}`;
    return 'no traffic data';
  }

  /** Ready-vs-desired fill for the replica gauge (0–100). */
  replicaPct(): number {
    const n = this.net();
    if (!n) return 0;
    if (n.desiredReplicas > 0) return Math.min(100, (n.readyReplicas / n.desiredReplicas) * 100);
    return n.readyReplicas > 0 ? 100 : 0;
  }

  /** True when every desired replica is Ready. */
  healthy(): boolean {
    const n = this.net();
    return !!n && n.desiredReplicas > 0 && n.readyReplicas >= n.desiredReplicas;
  }

  /** Live "what will happen" preview for the current form (also used in the confirm). */
  preview(): string {
    switch (this.kind) {
      case 'pod_kill': return `Delete ${this.targetAllPods ? 'ALL pods' : 'one pod'} — the Deployment reschedules them.`;
      case 'scale_down': return `Scale down to ${this.targetReplicas} replica(s) for ${this.durationSec}s, then auto-restore.`;
      case 'cpu_stress': return `Burn ${this.cpuWorkers} CPU worker(s) for ${this.durationSec}s (best-effort).`;
      default: return '';
    }
  }

  async run(): Promise<void> {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();
    if (!appId || !inst || this.submitting()) return;
    if (inst.status !== 'running') {
      this.toast.error('The instance must be Running to run a chaos experiment.');
      return;
    }

    const ok = await this.confirm.ask({
      title: 'Run chaos experiment?',
      message: `${this.preview()} Target: "${inst.branchName}". This is for resilience testing and reverts automatically.`,
      confirmText: 'Run experiment',
      cancelText: 'Cancel',
    });
    if (!ok) return;

    const body: StartChaosRequest = {
      kind: this.kind,
      durationSec: this.durationSec,
      targetAllPods: this.targetAllPods,
      targetReplicas: this.targetReplicas,
      cpuWorkers: this.cpuWorkers,
    };
    this.submitting.set(true);
    this.parent.projectService.startChaos(appId, inst.id, body).subscribe({
      next: () => {
        this.submitting.set(false);
        this.toast.success('Chaos experiment started — watch the Networking tab to see the impact.');
        this.reload();
      },
      error: (err) => {
        this.submitting.set(false);
        this.toast.error(err.error?.message || err.error?.error?.message || 'Failed to start experiment.');
      },
    });
  }

  stop(e: ChaosExperiment): void {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();
    if (!appId || !inst) return;
    this.parent.projectService.cancelChaos(appId, inst.id, e.id).subscribe({
      next: () => {
        this.toast.success('Experiment stopped — state restored.');
        this.reload();
      },
      error: () => this.toast.error('Failed to stop experiment.'),
    });
  }
}
