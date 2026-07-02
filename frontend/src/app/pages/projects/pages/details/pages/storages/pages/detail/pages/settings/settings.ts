import { Component, inject } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { Storages } from '../../../../storages';

@Component({
  selector: 'app-storage-settings',
  imports: [FormsModule],
  templateUrl: './settings.html',
  styles: ``,
})
export class StorageSettingsComponent {
  readonly parent = inject(Storages);
}
