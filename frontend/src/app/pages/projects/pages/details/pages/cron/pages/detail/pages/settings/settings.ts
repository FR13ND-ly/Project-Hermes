import { Component, inject } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { CronComponent } from '../../../../cron';

@Component({
  selector: 'app-cron-settings',
  imports: [FormsModule],
  templateUrl: './settings.html',
  styles: ``,
})
export class CronSettingsComponent {
  readonly parent = inject(CronComponent);
}
