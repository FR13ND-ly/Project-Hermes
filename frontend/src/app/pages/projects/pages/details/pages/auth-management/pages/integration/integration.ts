import { Component, inject, OnInit } from '@angular/core';
import { CommonModule } from '@angular/common';
import { AuthManagement } from '../../auth-management';

@Component({
  selector: 'app-auth-integration',
  standalone: true,
  imports: [CommonModule],
  templateUrl: './integration.html',
})
export class AuthIntegrationComponent implements OnInit {
  readonly parent = inject(AuthManagement);

  ngOnInit(): void {
    this.parent.loadIntegration();
  }
}
