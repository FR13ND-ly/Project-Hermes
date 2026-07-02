import { Component, inject, effect } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AuthManagementDetail } from '../../auth-management';

@Component({
  selector: 'app-auth-api-keys',
  imports: [FormsModule, DatePipe],
  templateUrl: './api-keys.html',
})
export class AuthApiKeysComponent {
  readonly parent = inject(AuthManagementDetail);

  constructor() {
    effect(() => {
      if (this.parent.selectedService()) {
        this.parent.loadApiKeys();
      }
    });
  }
}
