import { Component, inject } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { AppDetailComponent } from '../../app-detail';
import { DangerZone } from '../../../../../../../../shared/components/danger-zone/danger-zone';

@Component({
  selector: 'app-app-general',
  imports: [FormsModule, DangerZone],
  templateUrl: './general.html',
  styles: ``,
})
export class AppGeneralComponent {
  readonly parent = inject(AppDetailComponent);
}
