import { Component, inject } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';

@Component({
  selector: 'app-app-general',
  imports: [FormsModule],
  templateUrl: './general.html',
  styles: ``,
})
export class AppGeneralComponent {
  readonly parent = inject(AppDetailComponent);
}
