import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Storages } from '../../../../storages';

@Component({
  selector: 'app-storage-settings',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './settings.html',
  styles: ``,
})
export class StorageSettingsComponent {
  readonly parent = inject(Storages);
}
