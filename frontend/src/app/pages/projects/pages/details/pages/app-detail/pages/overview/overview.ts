import { Component, inject } from '@angular/core';
import { CommonModule, DatePipe, DecimalPipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-overview',
  standalone: true,
  imports: [CommonModule, DatePipe, DecimalPipe, FormsModule, RouterLink],
  templateUrl: './overview.html',
  styles: ``,
})
export class AppOverviewComponent {
  readonly parent = inject(AppDetailComponent);
}
