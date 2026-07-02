import { Component, inject } from '@angular/core';
import { DatePipe } from '@angular/common';
import { Storages } from '../../../../storages';

@Component({
  selector: 'app-storage-logs',
  imports: [DatePipe],
  templateUrl: './logs.html',
  styles: ``,
})
export class StorageLogsComponent {
  readonly parent = inject(Storages);
}
