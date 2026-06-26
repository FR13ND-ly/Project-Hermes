import { Component, inject, OnInit } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { CronComponent } from '../../../../cron';

@Component({
  selector: 'app-cron-logs',
  standalone: true,
  imports: [CommonModule, DatePipe],
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
