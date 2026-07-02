import { Component, inject } from '@angular/core';
import { DatePipe, DecimalPipe, NgClass } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-overview',
  imports: [DatePipe, DecimalPipe, FormsModule, RouterLink, NgClass],
  templateUrl: './overview.html',
  styles: ``,
})
export class AppOverviewComponent {
  readonly parent = inject(AppDetailComponent);
}
