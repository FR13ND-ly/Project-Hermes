import { Component, inject, effect } from '@angular/core';

import { AuthManagementDetail } from '../../auth-management';

@Component({
  selector: 'app-auth-integration',
  imports: [],
  templateUrl: './integration.html',
})
export class AuthIntegrationComponent {
  readonly parent = inject(AuthManagementDetail);

  constructor() {
    effect(() => {
      if (this.parent.selectedService()) {
        this.parent.loadIntegration();
      }
    });
  }
}
