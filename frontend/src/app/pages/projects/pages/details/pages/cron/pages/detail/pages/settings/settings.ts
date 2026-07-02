import { Component, inject } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { CronComponent } from '../../../../cron';
import { DangerZone } from '../../../../../../../../../../shared/components/danger-zone/danger-zone';

@Component({
  selector: 'app-cron-settings',
  imports: [FormsModule, DangerZone],
  templateUrl: './settings.html',
  styles: ``,
})
export class CronSettingsComponent {
  readonly parent = inject(CronComponent);
}
