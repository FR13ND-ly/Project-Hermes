import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { Storages } from '../../../../storages';

@Component({
  selector: 'app-storage-api',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './api.html',
  styles: ``,
})
export class StorageApiComponent {
  readonly parent = inject(Storages);
}
