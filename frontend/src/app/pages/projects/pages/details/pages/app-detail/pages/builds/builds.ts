import { Component, inject, OnInit, OnDestroy } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';
import { Pagination } from '../../../../../../../../shared/components/pagination/pagination';

@Component({
  selector: 'app-app-builds',
  standalone: true,
  imports: [CommonModule, DatePipe, FormsModule, Pagination],
  templateUrl: './builds.html',
  styles: ``,
})
export class AppBuildsComponent implements OnInit, OnDestroy {
  readonly parent = inject(AppDetailComponent);

  ngOnInit(): void {
    if (!this.parent.selectedBuildId() && this.parent.builds().length > 0) {
      this.parent.onViewBuildLogs(this.parent.builds()[0]);
    }
  }

  ngOnDestroy(): void {
    this.parent.selectedBuildId.set(null);
    this.parent.disconnectBuildLogs();
  }
}
