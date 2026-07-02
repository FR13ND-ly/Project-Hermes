import { Component, inject, OnInit, OnDestroy } from '@angular/core';

import { ActivatedRoute, RouterLink, RouterOutlet, RouterLinkActive } from '@angular/router';
import { Storages } from '../../storages';

@Component({
  selector: 'app-storage-detail',
  imports: [RouterLink, RouterOutlet, RouterLinkActive],
  templateUrl: './detail.html',
  styles: ``,
})
export class StorageDetailComponent implements OnInit, OnDestroy {
  readonly parent = inject(Storages);
  private readonly route = inject(ActivatedRoute);

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
