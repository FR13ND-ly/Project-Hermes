import { Component, inject, OnInit } from '@angular/core';

import { AuthManagement } from '../../auth-management';

@Component({
  selector: 'app-auth-integration',
  imports: [],
  templateUrl: './integration.html',
})
export class AuthIntegrationComponent implements OnInit {
  readonly parent = inject(AuthManagement);

  ngOnInit(): void {
    this.parent.loadIntegration();
  }
}
