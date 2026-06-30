import { Component, inject, OnInit, OnDestroy, signal, computed } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Subscription, interval } from 'rxjs';
import { startWith, switchMap } from 'rxjs/operators';
import { AppDetailComponent } from '../../app-detail';
import { ChaosExperiment, StartChaosRequest } from '../../../../../../../../core/services/project.service';
import { ConfirmService } from '../../../../../../../../core/services/confirm.service';
import { ToastService } from '../../../../../../../../core/services/toast.service';

@Component({
  selector: 'app-app-chaos',
  standalone: true,
  imports: [CommonModule, DatePipe, FormsModule],
  templateUrl: './chaos.html',
  styles: ``,
})
export class AppChaosComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);
  private readonly confirm = inject(ConfirmService);
  private readonly toast = inject(ToastService);

  readonly experiments = signal<ChaosExperiment[]>([]);
  readonly active = computed(() => this.experiments().find(e => e.status === 'running') || null);
  readonly submitting = signal(false);
  readonly now = signal(Date.now());

  // Experiment form
  kind: 'pod_kill' | 'scale_down' | 'cpu_stress' = 'pod_kill';
  durationSec = 60;
  targetAllPods = false;
  targetReplicas = 0;
  cpuWorkers = 1;

  private pollSub?: Subscription;
  private ticker?: any;

  ngOnInit(): void {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();
    if (appId && inst) {
      this.pollSub = interval(4000)
        .pipe(
          startWith(0),
          switchMap(() => this.parent.projectService.getChaos(appId, inst.id)),
        )
        .subscribe({
          next: (res) => this.experiments.set(res),
          error: () => {},
        });
    }
    this.ticker = setInterval(() => this.now.set(Date.now()), 1000);
  }

  ngOnDestroy(): void {
    this.pollSub?.unsubscribe();
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

  /** Seconds remaining until a running experiment auto-reverts. */
  countdown(e: ChaosExperiment): number {
    if (!e.revertAt) return 0;
    return Math.max(0, Math.round((new Date(e.revertAt).getTime() - this.now()) / 1000));
  }

  private summary(inst: { branchName: string }): string {
    switch (this.kind) {
      case 'pod_kill': return `kill ${this.targetAllPods ? 'ALL pods' : 'one pod'}`;
      case 'scale_down': return `scale down to ${this.targetReplicas} replica(s) for ${this.durationSec}s`;
      case 'cpu_stress': return `stress ${this.cpuWorkers} CPU worker(s) for ${this.durationSec}s`;
      default: return this.kind;
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
      message: `This will ${this.summary(inst)} on "${inst.branchName}". It is for resilience testing and reverts automatically.`,
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
