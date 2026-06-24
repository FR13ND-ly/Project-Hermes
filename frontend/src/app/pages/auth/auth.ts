import { Component, inject, signal } from '@angular/core';
import { Router } from '@angular/router';
import { FormsModule } from '@angular/forms';
import { AuthService } from '../../core/services/auth';

@Component({
  selector: 'app-auth',
  imports: [FormsModule],
  templateUrl: './auth.html',
  styleUrl: './auth.css',
})
export class Auth {
  readonly auth = inject(AuthService);
  private readonly router = inject(Router);

  // Toggle between 'login' and 'activate'
  readonly mode = signal<'login' | 'activate'>('login');

  // Login signals
  readonly loginIdentity = signal('');
  readonly password = signal('');
  readonly error = signal<string | null>(null);
  readonly loading = signal(false);

  // Activation signals
  readonly activateEmail = signal('');
  readonly temporaryPassword = signal('');
  readonly newPassword = signal('');
  readonly confirmNewPassword = signal('');
  readonly activationSuccess = signal(false);
  readonly activationError = signal<string | null>(null);
  readonly activationLoading = signal(false);

  constructor() {
    if (this.auth.isAuthenticated()) {
      this.router.navigate(['/dashboard']);
    }
  }

  onActivateSubmit(): void {
    const email = this.activateEmail().trim();
    const tempPwd = this.temporaryPassword().trim();
    const newPwd = this.newPassword().trim();
    const confirmPwd = this.confirmNewPassword().trim();

    if (!email || !tempPwd || !newPwd || !confirmPwd) {
      this.activationError.set('All fields are required.');
      return;
    }

    if (newPwd.length < 6) {
      this.activationError.set('The new password must be at least 6 characters.');
      return;
    }

    if (newPwd !== confirmPwd) {
      this.activationError.set('The new passwords do not match.');
      return;
    }

    this.activationLoading.set(true);
    this.activationError.set(null);

    this.auth.activateAccount(email, tempPwd, newPwd).subscribe({
      next: () => {
        this.activationLoading.set(false);
        this.activationSuccess.set(true);
        this.activateEmail.set('');
        this.temporaryPassword.set('');
        this.newPassword.set('');
        this.confirmNewPassword.set('');
        setTimeout(() => {
          this.activationSuccess.set(false);
          this.mode.set('login');
        }, 3500);
      },
      error: (err) => {
        this.activationLoading.set(false);
        this.activationError.set(err.error?.message || 'Error activating the account.');
      }
    });
  }

  onGoBackToLogin(): void {
    this.mode.set('login');
    this.activationError.set(null);
    this.activateEmail.set('');
    this.temporaryPassword.set('');
  }

  onSubmit(): void {
    if (!this.loginIdentity() || !this.password()) {
      this.error.set('Please enter your sign-in credentials.');
      return;
    }

    this.loading.set(true);
    this.error.set(null);

    this.auth.login(this.loginIdentity(), this.password()).subscribe({
      next: (res) => {
        this.loading.set(false);
        const userStatus = res.user.status?.toLowerCase().replace(/_/g, '');
        if (userStatus === 'pendingverification') {
          // Pre-populate credentials for activation
          this.activateEmail.set(res.user.email);
          this.temporaryPassword.set(this.password());
          
          // Clear credentials/error in login form
          this.password.set('');
          this.error.set(null);
          
          // Clear active session since they must change password first
          this.auth.logout();
          
          // Set mode to activate
          this.mode.set('activate');
          this.activationError.set('Since this is your first sign-in, please choose a new, secure password.');
        } else {
          this.router.navigate(['/dashboard']);
        }
      },
      error: (err) => {
        this.loading.set(false);
        const backendMessage = err.error?.message;
        if (backendMessage === 'This account has been suspended.') {
          this.error.set('This account has been suspended.');
        } else if (backendMessage === 'Invalid credentials.') {
          this.error.set('Incorrect username or password.');
        } else if (backendMessage) {
          this.error.set(backendMessage);
        } else if (err.status === 401) {
          this.error.set('Incorrect username or password.');
        } else {
          this.error.set('Connection error to the Hermes server.');
        }
      }
    });
  }
}
