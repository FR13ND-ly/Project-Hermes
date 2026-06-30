import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { CommonModule, DecimalPipe } from '@angular/common';
import { Subscription, interval } from 'rxjs';
import { startWith, switchMap } from 'rxjs/operators';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-networking',
  standalone: true,
  imports: [CommonModule, DecimalPipe],
  templateUrl: './networking.html',
  styles: ``,
})
export class AppNetworkingComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);
  
  data: any = null;
  loading = true;
  error: string | null = null;
  
  private pollSub?: Subscription;

  ngOnInit(): void {
    const appId = this.parent.appId();
    const inst = this.parent.getSelectedInstance();
    
    if (appId && inst) {
      this.pollSub = interval(5000)
        .pipe(
          startWith(0),
          switchMap(() => this.parent.projectService.getNetworkObservability(appId, inst.id))
        )
        .subscribe({
          next: (res) => {
            this.data = res;
            this.loading = false;
            this.error = null;
          },
          error: (err) => {
            console.error('Failed to load network stats', err);
            this.error = 'Failed to load live networking and pods status.';
            this.loading = false;
          }
        });
    } else {
      this.loading = false;
      this.error = 'No active application instance selected.';
    }
  }

  ngOnDestroy(): void {
    this.pollSub?.unsubscribe();
  }

  getTrafficClassRatio(val: number | undefined): number {
    if (!this.data || !this.data.traffic || !this.data.traffic.requestRate || this.data.traffic.requestRate === 0) {
      return 0;
    }
    const total = this.data.traffic.requestRate;
    return ((val || 0) / total) * 100;
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
}
