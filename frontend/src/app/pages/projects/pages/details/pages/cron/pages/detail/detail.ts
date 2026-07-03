import { Component, inject, OnInit, OnDestroy, effect, DestroyRef } from '@angular/core';
import { DatePipe } from '@angular/common';
import { ActivatedRoute, RouterLink, RouterOutlet, RouterLinkActive } from '@angular/router';
import { CronComponent } from '../../cron';
import { ProjectService } from '../../../../../../../../core/services/project.service';
import { BreadcrumbService } from '../../../../../../../../core/services/breadcrumb.service';

@Component({
  selector: 'app-cron-detail',
  imports: [DatePipe, RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './detail.html',
  styles: ``,
})
export class CronDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(CronComponent);
  private readonly route = inject(ActivatedRoute);
  private readonly projectService = inject(ProjectService);

  private readonly breadcrumb = inject(BreadcrumbService);
  private readonly crumbEffect = effect(() => this.breadcrumb.set(this.parent.selectedCronJob()?.name ?? null));
  private readonly crumbCleanup = inject(DestroyRef).onDestroy(() => this.breadcrumb.clear());

  ngOnInit(): void {
    this.route.paramMap.subscribe(params => {
      const cronId = params.get('cronId');
      const projId = this.parent.parent.projectId();
      if (cronId && projId) {
        this.projectService.listProjectCronJobs(projId, 1, 1000).subscribe({
          next: (res) => {
            const job = (res?.items || []).find(j => j.id === cronId);
            if (job) {
              this.parent.selectCronJob(job);
            }
          }
        });
      }
    });
  }

  ngOnDestroy(): void {
    this.parent.deselectCronJob();
  }
}
