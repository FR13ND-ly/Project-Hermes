import { Component, inject } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { CronComponent } from '../../../../cron';

@Component({
  selector: 'app-cron-details',
  standalone: true,
  imports: [CommonModule, DatePipe],
  templateUrl: './details.html',
  styles: ``,
})
export class CronDetailsComponent {
  readonly parent = inject(CronComponent);
}
