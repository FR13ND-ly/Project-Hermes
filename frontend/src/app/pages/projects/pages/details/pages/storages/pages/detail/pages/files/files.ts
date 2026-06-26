import { Component, inject } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Storages } from '../../../../storages';
import { Pagination } from '../../../../../../../../../../shared/components/pagination/pagination';

@Component({
  selector: 'app-storage-files',
  standalone: true,
  imports: [CommonModule, FormsModule, Pagination],
  templateUrl: './files.html',
  styles: ``,
})
export class StorageFilesComponent {
  readonly parent = inject(Storages);
}
