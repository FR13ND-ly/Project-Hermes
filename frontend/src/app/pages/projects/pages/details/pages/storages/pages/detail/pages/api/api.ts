import { Component, inject } from '@angular/core';

import { Storages } from '../../../../storages';

@Component({
  selector: 'app-storage-api',
  imports: [],
  templateUrl: './api.html',
  styles: ``,
})
export class StorageApiComponent {
  readonly parent = inject(Storages);
}
