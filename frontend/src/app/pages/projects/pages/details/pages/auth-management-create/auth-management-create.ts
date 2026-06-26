import { Component, inject, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { Router } from '@angular/router';
import { Details } from '../../details';
import { AuthManagementService } from '../../../../../../core/services/auth-management.service';
import { ToastService } from '../../../../../../core/services/toast.service';

@Component({
  selector: 'app-auth-management-create',
  standalone: true,
  imports: [CommonModule, FormsModule],
  templateUrl: './auth-management-create.html',
  styleUrl: './auth-management-create.css',
})
export class AuthManagementCreate {
  readonly parent = inject(Details);
  private readonly authMgmtService = inject(AuthManagementService);
  private readonly router = inject(Router);
  private readonly toast = inject(ToastService);

  readonly creatingService = signal(false);
  readonly newServiceName = signal('');
  readonly publishAppId = signal(false);
  readonly appIdEnvKeyName = signal('');
  readonly publishSecret = signal(false);
  readonly secretEnvKeyName = signal('');
  readonly publishApiKey = signal(false);
  readonly apiKeyEnvKeyName = signal('');

  onCreateService(): void {
    const projectId = this.parent.projectId();
    const name = this.newServiceName().trim();
    if (!projectId || !name) {
      this.toast.error('Service name is required.');
      return;
    }

    if (this.publishAppId() && !this.appIdEnvKeyName().trim()) {
      this.toast.error('The environment variable name for App ID is required when checked.');
      return;
    }
    if (this.publishSecret() && !this.secretEnvKeyName().trim()) {
      this.toast.error('The environment variable name for Secret is required when checked.');
      return;
    }
    if (this.publishApiKey() && !this.apiKeyEnvKeyName().trim()) {
      this.toast.error('The environment variable name for API Key is required when checked.');
      return;
    }

    this.creatingService.set(true);
    this.authMgmtService.createService(projectId, name, {
      publishAppId: this.publishAppId(),
      appIdEnvKey: this.publishAppId() && this.appIdEnvKeyName().trim() ? this.appIdEnvKeyName().trim() : undefined,
      publishSecret: this.publishSecret(),
      secretEnvKey: this.publishSecret() && this.secretEnvKeyName().trim() ? this.secretEnvKeyName().trim() : undefined,
      publishApiKey: this.publishApiKey(),
      apiKeyEnvKey: this.publishApiKey() && this.apiKeyEnvKeyName().trim() ? this.apiKeyEnvKeyName().trim() : undefined
    }).subscribe({
      next: (svc) => {
        this.creatingService.set(false);
        this.toast.success(`Authentication service "${svc.name}" has been created.`);
        this.router.navigate(['/projects', projectId, 'auth-management']);
      },
      error: (err) => {
        this.creatingService.set(false);
        this.toast.error(err.error?.message || 'Error creating service.');
      }
    });
  }

  onCancel(): void {
    this.router.navigate(['/projects', this.parent.projectId(), 'auth-management']);
  }
}
