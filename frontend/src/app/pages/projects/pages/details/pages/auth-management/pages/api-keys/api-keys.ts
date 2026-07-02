import { Component, inject, OnInit } from '@angular/core';
import { DatePipe } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { AuthManagement } from '../../auth-management';

@Component({
  selector: 'app-auth-api-keys',
  imports: [FormsModule, DatePipe],
  templateUrl: './api-keys.html',
})
export class AuthApiKeysComponent implements OnInit {
  readonly parent = inject(AuthManagement);

  ngOnInit(): void {
    this.parent.loadApiKeys();
  }
}
