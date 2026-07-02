import { Component, inject, effect } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AuthManagementDetail } from '../../auth-management';

@Component({
  selector: 'app-auth-users',
  imports: [FormsModule, DatePipe],
  templateUrl: './users.html',
})
export class AuthUsersComponent {
  readonly parent = inject(AuthManagementDetail);

  constructor() {
    effect(() => {
      if (this.parent.selectedService()) {
        this.parent.loadUsers();
      }
    });
  }
}
