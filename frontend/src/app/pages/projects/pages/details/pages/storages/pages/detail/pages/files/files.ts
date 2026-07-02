import { Component, inject } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { Storages } from '../../../../storages';
import { Pagination } from '../../../../../../../../../../shared/components/pagination/pagination';

@Component({
  selector: 'app-storage-files',
  imports: [FormsModule, Pagination],
  templateUrl: './files.html',
  styles: ``,
})
export class StorageFilesComponent {
  readonly parent = inject(Storages);
}
