import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-general',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './general.html',
  styles: ``,
})
export class AppGeneralComponent {
  readonly parent = inject(AppDetailComponent);
}
