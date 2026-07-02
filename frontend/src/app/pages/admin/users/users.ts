import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { DatePipe } from '@angular/common';
import { Router, RouterLink, RouterLinkActive } from '@angular/router';
import { AuthService, User } from '../../../core/services/auth';
import { ToastService } from '../../../core/services/toast.service';
import { ConfirmService } from '../../../core/services/confirm.service';

@Component({
  selector: 'app-admin-users',
  imports: [FormsModule, DatePipe, RouterLink, RouterLinkActive],
  templateUrl: './users.html',
  styleUrl: './users.css',
})
export class AdminUsers implements OnInit {
  readonly auth = inject(AuthService);
  private readonly toast = inject(ToastService);
  private readonly router = inject(Router);
  private readonly confirm = inject(ConfirmService);

  // Platform user provisioning signals
  readonly provisioningUser = signal(false);
  readonly newUserName = signal('');
  readonly newUserEmail = signal('');
  readonly newUserIsAdmin = signal(false);
  readonly provisionedPassword = signal<string | null>(null);
  readonly provisioningError = signal<string | null>(null);

  // Modals
  readonly showRegisterModal = signal(false);
  readonly showChangePasswordModal = signal(false);

  // User list signals
  readonly users = signal<User[]>([]);
  readonly loadingUsers = signal(false);

  // Change-my-password signals (self-service for the logged-in admin / root).
  readonly currentPassword = signal('');
  readonly newPassword = signal('');
  readonly confirmPassword = signal('');
  readonly changingPassword = signal(false);
  readonly passwordError = signal<string | null>(null);

  onOpenRegisterModal(): void {
    this.newUserName.set('');
    this.newUserEmail.set('');
    this.newUserIsAdmin.set(false);
    this.provisionedPassword.set(null);
    this.provisioningError.set(null);
    this.showRegisterModal.set(true);
  }

  onOpenChangePasswordModal(): void {
    this.currentPassword.set('');
    this.newPassword.set('');
    this.confirmPassword.set('');
    this.passwordError.set(null);
    this.showChangePasswordModal.set(true);
  }

  constructor() {
    // Security check: only super admins are allowed here
    const user = this.auth.currentUser();
    if (!user || !user.is_super_admin) {
      this.router.navigate(['/dashboard']);
    }
  }

  ngOnInit(): void {
    this.loadUsers();
  }

  loadUsers(): void {
    this.loadingUsers.set(true);
    this.auth.listUsers().subscribe({
      next: (res) => {
        this.users.set(res || []);
        this.loadingUsers.set(false);
      },
      error: (err) => {
        this.toast.error('Failed to load user list.');
        this.loadingUsers.set(false);
      }
    });
  }

  onProvisionUser(): void {
    const email = this.newUserEmail().trim();
    const username = this.newUserName().trim();
    const isAdmin = this.newUserIsAdmin();

    if (!email || !username) {
      this.provisioningError.set('All fields (Email, Username) are required.');
      return;
    }

    this.provisioningUser.set(true);
    this.provisioningError.set(null);
    this.provisionedPassword.set(null);

    this.auth.provisionUser(username, email, isAdmin).subscribe({
      next: (tempPwd) => {
        this.provisionedPassword.set(tempPwd);
        this.newUserName.set('');
        this.newUserEmail.set('');
        this.newUserIsAdmin.set(false);
        this.provisioningUser.set(false);
        this.toast.success('User registered successfully on the platform!');
        this.loadUsers(); // Refresh list
      },
      error: (err) => {
        this.provisioningError.set(err.error?.message || 'Failed to register user.');
        this.provisioningUser.set(false);
      }
    });
  }

  onChangePassword(): void {
    const current = this.currentPassword();
    const next = this.newPassword();
    const confirm = this.confirmPassword();

    if (!current || !next) {
      this.passwordError.set('Current and new password are required.');
      return;
    }
    if (next.length < 8) {
      this.passwordError.set('New password must be at least 8 characters.');
      return;
    }
    if (next !== confirm) {
      this.passwordError.set('New password and confirmation do not match.');
      return;
    }
    if (next === current) {
      this.passwordError.set('New password must be different from the current one.');
      return;
    }

    this.changingPassword.set(true);
    this.passwordError.set(null);
    this.auth.changePassword(current, next).subscribe({
      next: () => {
        this.changingPassword.set(false);
        this.currentPassword.set('');
        this.newPassword.set('');
        this.confirmPassword.set('');
        this.showChangePasswordModal.set(false);
        this.toast.success('Your password has been changed.');
      },
      error: (err) => {
        this.changingPassword.set(false);
        this.passwordError.set(err.error?.message || err.error?.error?.message || 'Failed to change password.');
      }
    });
  }

  async onDeleteUser(user: User): Promise<void> {
    const current = this.auth.currentUser();
    if (current && current.id === user.id) {
      this.toast.error('You cannot delete your own account.');
      return;
    }

    const confirmed = await this.confirm.ask({
      title: 'Delete User',
      message: `Are you sure you want to delete the account of "${user.username}" (${user.email})? This action is irreversible and will remove all their memberships and associated resources.`,
      confirmText: 'Delete',
      cancelText: 'Cancel',
      isDanger: true
    });
    
    if (!confirmed) return;

    this.auth.deleteUser(user.id).subscribe({
      next: () => {
        this.toast.success('User has been removed from the platform.');
        this.loadUsers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to delete user.');
      }
    });
  }

  async onResetPassword(user: User): Promise<void> {
    const current = this.auth.currentUser();
    if (current && current.id === user.id) {
      this.toast.error('You cannot reset your own password this way.');
      return;
    }

    const confirmed = await this.confirm.ask({
      title: 'Reset Password',
      message: `Are you sure you want to reset the password for "${user.username}" (${user.email})? The account will be set back to pending activation and a new temporary password will be generated.`,
      confirmText: 'Reset',
      cancelText: 'Cancel',
      isDanger: true
    });
    
    if (!confirmed) return;

    this.auth.resetUserPassword(user.id).subscribe({
      next: (tempPwd) => {
        this.provisionedPassword.set(tempPwd);
        this.toast.success('Password has been reset successfully!');
        this.loadUsers();
        window.scrollTo({ top: 0, behavior: 'smooth' });
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to reset password.');
      }
    });
  }

  async onToggleSuspend(user: User): Promise<void> {
    const current = this.auth.currentUser();
    if (current && current.id === user.id) {
      this.toast.error('You cannot suspend your own account.');
      return;
    }

    const isSuspended = user.status?.toLowerCase() === 'suspended';
    const actionText = isSuspended ? 'activate' : 'suspend';

    const confirmed = await this.confirm.ask({
      title: isSuspended ? 'Activate Account' : 'Suspend Account',
      message: `Are you sure you want to ${actionText} the account of "${user.username}" (${user.email})?`,
      confirmText: isSuspended ? 'Activate' : 'Suspend',
      cancelText: 'Cancel',
      isDanger: !isSuspended
    });

    if (!confirmed) return;

    this.auth.toggleUserSuspend(user.id).subscribe({
      next: (newStatus) => {
        const msg = newStatus === 'suspended' ? 'Account has been suspended.' : 'Account has been activated.';
        this.toast.success(msg);
        this.loadUsers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Failed to update account status.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copied to clipboard!');
    });
  }
}
