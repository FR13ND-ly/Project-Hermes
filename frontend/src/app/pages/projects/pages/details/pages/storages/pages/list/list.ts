import { Component, inject, OnInit } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router, RouterLink } from '@angular/router';
import { Storages } from '../../storages';

@Component({
  selector: 'app-storage-list',
  imports: [FormsModule, DatePipe, RouterLink],
  templateUrl: './list.html',
  styles: ``,
})
export class StorageListComponent implements OnInit {
  readonly parent = inject(Storages);
  private readonly router = inject(Router);

  ngOnInit(): void {
    this.parent.loadBuckets();
    this.parent.loadPvcs();
  }

  navigateToBucket(bucketId: string): void {
    this.router.navigate(['/projects', this.parent.parent.projectId(), 'storages', bucketId]);
  }
}
