import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { CronComponent } from '../../../../cron';

@Component({
  selector: 'app-cron-settings',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './settings.html',
  styles: ``,
})
export class CronSettingsComponent {
  readonly parent = inject(CronComponent);
}
