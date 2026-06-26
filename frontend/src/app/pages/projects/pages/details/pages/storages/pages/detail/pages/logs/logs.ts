import { Component, inject } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { Storages } from '../../../../storages';

@Component({
  selector: 'app-storage-logs',
  standalone: true,
  imports: [CommonModule, DatePipe],
  templateUrl: './logs.html',
  styles: ``,
})
export class StorageLogsComponent {
  readonly parent = inject(Storages);
}
