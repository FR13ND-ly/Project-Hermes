import { Component, inject, effect } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { AuthManagementDetail } from '../../auth-management';

@Component({
  selector: 'app-auth-roles',
  imports: [FormsModule],
  templateUrl: './roles.html',
})
export class AuthRolesComponent {
  readonly parent = inject(AuthManagementDetail);

  constructor() {
    effect(() => {
      if (this.parent.selectedService()) {
        this.parent.loadAuthConfig();
      }
    });
  }
}
