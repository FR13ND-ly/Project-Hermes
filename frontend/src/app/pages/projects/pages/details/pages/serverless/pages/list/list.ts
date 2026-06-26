import { Component, inject, signal, OnInit, effect } from '@angular/core';
import { CommonModule } from '@angular/common';
import { RouterLink } from '@angular/router';
import { Details } from '../../../../details';
import { ProjectService, ServerlessInstance } from '../../../../../../../../core/services/project.service';
import { Pagination } from '../../../../../../../../shared/components/pagination/pagination';
import { DEFAULT_PAGE_SIZE } from '../../../../../../../../core/models/pagination';

@Component({
  selector: 'app-serverless-list',
  standalone: true,
  imports: [CommonModule, Pagination, RouterLink],
  templateUrl: './list.html',
  styles: ``,
})
export class ServerlessListComponent implements OnInit {
  readonly parent = inject(Details);
  private readonly projectService = inject(ProjectService);

  readonly loading = signal(false);
  readonly instances = signal<ServerlessInstance[]>([]);
  readonly page = signal(1);
  readonly pageSize = signal(DEFAULT_PAGE_SIZE);
  readonly total = signal(0);

  constructor() {
    effect(() => {
      const projId = this.parent.projectId();
      if (projId) {
        this.loadInstances();
      }
    });
  }

  ngOnInit(): void {
    this.loadInstances();
  }

  loadInstances(): void {
    const projId = this.parent.projectId();
    if (!projId) return;
    this.loading.set(true);
    this.projectService.listProjectFunctions(projId, this.page(), this.pageSize()).subscribe({
      next: (res) => {
        this.instances.set(res?.items || []);
        this.total.set(res?.total || 0);
        this.loading.set(false);
      },
      error: () => { this.instances.set([]); this.loading.set(false); }
    });
  }

  onPageChange(p: number): void {
    this.page.set(p);
    this.loadInstances();
  }
}
