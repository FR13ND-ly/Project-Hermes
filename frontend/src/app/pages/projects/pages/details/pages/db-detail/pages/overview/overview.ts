import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { DbDetailComponent } from '../../db-detail';

@Component({
  selector: 'app-db-overview',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './overview.html',
  styles: ``,
})
export class DbOverviewComponent {
  readonly dbDetail = inject(DbDetailComponent);
}
