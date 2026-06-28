import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { Subscription, interval } from 'rxjs';
import { startWith, switchMap } from 'rxjs/operators';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-networking',
  standalone: true,
  imports: [CommonModule, DatePipe, DecimalPipe],
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
}
