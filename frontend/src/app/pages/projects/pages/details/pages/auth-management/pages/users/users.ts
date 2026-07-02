import { Component, inject, OnInit } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AuthManagement } from '../../auth-management';

@Component({
  selector: 'app-auth-users',
  imports: [FormsModule, DatePipe],
  templateUrl: './users.html',
})
export class AuthUsersComponent implements OnInit {
  readonly parent = inject(AuthManagement);

  ngOnInit(): void {
    this.parent.loadUsers();
  }
}
