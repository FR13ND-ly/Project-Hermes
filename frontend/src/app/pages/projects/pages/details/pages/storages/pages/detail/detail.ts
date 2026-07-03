import { Component, inject, OnInit, OnDestroy, effect, DestroyRef } from '@angular/core';

import { ActivatedRoute, RouterLink, RouterOutlet, RouterLinkActive } from '@angular/router';
import { Storages } from '../../storages';
import { DangerZone } from '../../../../../../../../shared/components/danger-zone/danger-zone';
import { BreadcrumbService } from '../../../../../../../../core/services/breadcrumb.service';

@Component({
  selector: 'app-storage-detail',
  imports: [RouterLink, RouterOutlet, RouterLinkActive, DangerZone],
  templateUrl: './detail.html',
  styles: ``,
})
export class StorageDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(Storages);
  private readonly route = inject(ActivatedRoute);

  private readonly breadcrumb = inject(BreadcrumbService);
  private readonly crumbEffect = effect(() => this.breadcrumb.set(this.parent.selectedBucket()?.name ?? null));
  private readonly crumbCleanup = inject(DestroyRef).onDestroy(() => this.breadcrumb.clear());

  ngOnInit(): void {
    this.route.paramMap.subscribe(params => {
      const bucketId = params.get('bucketId');
      if (!bucketId) return;

      // Try to find bucket in already-loaded list first
      const existing = this.parent.buckets().find(b => b.id === bucketId);
      if (existing) {
        this.parent.selectBucket(existing);
      } else {
        // Buckets not loaded yet — fetch them, then select
        this.parent.storageService.listBuckets().subscribe({
          next: (res) => {
            this.parent.buckets.set(res || []);
            const bucket = (res || []).find(b => b.id === bucketId);
            if (bucket) {
              this.parent.selectBucket(bucket);
            }
          }
        });
      }
    });
  }

  ngOnDestroy(): void {
    this.parent.deselectBucket();
  }
}
