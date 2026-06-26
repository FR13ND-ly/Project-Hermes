import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { RouterLink } from '@angular/router';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-advanced',
  standalone: true,
  imports: [CommonModule, FormsModule, RouterLink],
  templateUrl: './advanced.html',
  styles: ``,
})
export class AppAdvancedComponent {
  readonly parent = inject(AppDetailComponent);
}
