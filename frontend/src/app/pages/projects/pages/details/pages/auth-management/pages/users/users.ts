import { Component, inject, OnInit } from '@angular/core';
import { CommonModule, DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AuthManagement } from '../../auth-management';

@Component({
  selector: 'app-auth-users',
  standalone: true,
  imports: [CommonModule, FormsModule, DatePipe],
  templateUrl: './users.html',
})
export class AuthUsersComponent implements OnInit {
  readonly parent = inject(AuthManagement);

  ngOnInit(): void {
    this.parent.loadUsers();
  }
}
