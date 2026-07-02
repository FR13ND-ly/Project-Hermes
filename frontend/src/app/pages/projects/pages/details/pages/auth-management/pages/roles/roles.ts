import { Component, inject, OnInit } from '@angular/core';

import { FormsModule } from '@angular/forms';
import { AuthManagementDetail } from '../../auth-management';

@Component({
  selector: 'app-auth-roles',
  imports: [FormsModule],
  templateUrl: './roles.html',
})
export class AuthRolesComponent implements OnInit {
  readonly parent = inject(AuthManagementDetail);

  ngOnInit(): void {
    this.parent.loadAuthConfig();
  }
}
