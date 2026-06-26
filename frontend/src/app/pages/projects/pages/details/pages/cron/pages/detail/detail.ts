import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { ActivatedRoute, RouterLink, RouterOutlet, RouterLinkActive } from '@angular/router';
import { CronComponent } from '../../cron';
import { ProjectService } from '../../../../../../../../core/services/project.service';

@Component({
  selector: 'app-cron-detail',
  standalone: true,
  imports: [CommonModule, DatePipe, RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './detail.html',
  styles: ``,
})
export class CronDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(CronComponent);
  private readonly route = inject(ActivatedRoute);
  private readonly projectService = inject(ProjectService);

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
