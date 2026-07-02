import { Component, inject, OnInit } from '@angular/core';
import { DatePipe } from '@angular/common';
import { CronComponent } from '../../../../cron';

@Component({
  selector: 'app-cron-logs',
  imports: [DatePipe],
  templateUrl: './logs.html',
  styles: ``,
})
export class CronLogsComponent implements OnInit {
  readonly parent = inject(CronComponent);

  ngOnInit(): void {
    this.parent.currentPage.set(1);
    this.parent.loadCronLogs(1, this.parent.pageSize());
  }
}
