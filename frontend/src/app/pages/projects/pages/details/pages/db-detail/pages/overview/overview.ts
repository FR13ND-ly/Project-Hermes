import { Component, inject } from '@angular/core';

import { DbDetailComponent } from '../../db-detail';

@Component({
  selector: 'app-db-overview',
  imports: [],
  templateUrl: './overview.html',
  styles: ``,
})
export class DbOverviewComponent {
  readonly dbDetail = inject(DbDetailComponent);
}
