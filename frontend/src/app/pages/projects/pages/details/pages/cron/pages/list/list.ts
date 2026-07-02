import { Component, inject, OnInit } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { CronComponent } from '../../cron';
import { Pagination } from '../../../../../../../../shared/components/pagination/pagination';

@Component({
  selector: 'app-cron-list',
  imports: [FormsModule, DatePipe, RouterLink, Pagination],
  templateUrl: './list.html',
  styles: ``,
})
export class CronListComponent implements OnInit {
  readonly parent = inject(CronComponent);

  ngOnInit(): void {
    this.parent.loadCronJobs();
  }
}
