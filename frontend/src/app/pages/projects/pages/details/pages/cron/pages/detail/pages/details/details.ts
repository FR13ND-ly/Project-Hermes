import { Component, inject } from '@angular/core';
import { DatePipe } from '@angular/common';
import { CronComponent } from '../../../../cron';

@Component({
  selector: 'app-cron-details',
  imports: [DatePipe],
  templateUrl: './details.html',
  styles: ``,
})
export class CronDetailsComponent {
  readonly parent = inject(CronComponent);
}
