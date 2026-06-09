import { Component, inject, signal, OnInit } from '@angular/core';
import { FormsModule } from '@angular/forms';
import { CommonModule, DatePipe } from '@angular/common';
import { Router } from '@angular/router';
import { AuthService, User } from '../../../core/services/auth';
import { ToastService } from '../../../core/services/toast.service';
import { ConfirmService } from '../../../core/services/confirm.service';

@Component({
  selector: 'app-admin-users',
  imports: [CommonModule, FormsModule, DatePipe],
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

  // User list signals
  readonly users = signal<User[]>([]);
  readonly loadingUsers = signal(false);

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
        this.toast.error('Eroare la încărcarea listei de utilizatori.');
        this.loadingUsers.set(false);
      }
    });
  }

  onProvisionUser(): void {
    const email = this.newUserEmail().trim();
    const username = this.newUserName().trim();
    const isAdmin = this.newUserIsAdmin();

    if (!email || !username) {
      this.provisioningError.set('Toate câmpurile (Email, Username) sunt obligatorii.');
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
        this.toast.success('Utilizatorul a fost înregistrat cu succes pe platformă!');
        this.loadUsers(); // Refresh list
      },
      error: (err) => {
        this.provisioningError.set(err.error?.message || 'Eroare la înregistrarea utilizatorului.');
        this.provisioningUser.set(false);
      }
    });
  }

  async onDeleteUser(user: User): Promise<void> {
    const current = this.auth.currentUser();
    if (current && current.id === user.id) {
      this.toast.error('Nu vă puteți șterge propriul cont.');
      return;
    }

    const confirmed = await this.confirm.ask({
      title: 'Ștergere Utilizator',
      message: `Sigur doriți să ștergeți contul utilizatorului "${user.username}" (${user.email})? Această acțiune este ireversibilă și va șterge toate apartenențele și resursele sale asociate.`,
      confirmText: 'Șterge',
      cancelText: 'Anulează',
      isDanger: true
    });
    
    if (!confirmed) return;

    this.auth.deleteUser(user.id).subscribe({
      next: () => {
        this.toast.success('Utilizatorul a fost șters de pe platformă.');
        this.loadUsers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la ștergerea utilizatorului.');
      }
    });
  }

  async onResetPassword(user: User): Promise<void> {
    const current = this.auth.currentUser();
    if (current && current.id === user.id) {
      this.toast.error('Nu vă puteți reseta propria parolă în acest fel.');
      return;
    }

    const confirmed = await this.confirm.ask({
      title: 'Resetare Parolă',
      message: `Sigur doriți să resetați parola utilizatorului "${user.username}" (${user.email})? Contul va fi trecut înapoi în starea de activare (neactivat) și va fi generată o parolă temporară nouă.`,
      confirmText: 'Resetează',
      cancelText: 'Anulează',
      isDanger: true
    });
    
    if (!confirmed) return;

    this.auth.resetUserPassword(user.id).subscribe({
      next: (tempPwd) => {
        this.provisionedPassword.set(tempPwd);
        this.toast.success('Parola a fost resetată cu succes!');
        this.loadUsers();
        window.scrollTo({ top: 0, behavior: 'smooth' });
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la resetarea parolei.');
      }
    });
  }

  async onToggleSuspend(user: User): Promise<void> {
    const current = this.auth.currentUser();
    if (current && current.id === user.id) {
      this.toast.error('Nu vă puteți suspenda propriul cont.');
      return;
    }

    const isSuspended = user.status?.toLowerCase() === 'suspended';
    const actionText = isSuspended ? 'activați' : 'suspendați';
    
    const confirmed = await this.confirm.ask({
      title: isSuspended ? 'Activare Cont' : 'Suspendare Cont',
      message: `Sigur doriți să ${actionText} contul utilizatorului "${user.username}" (${user.email})?`,
      confirmText: isSuspended ? 'Activează' : 'Suspendă',
      cancelText: 'Anulează',
      isDanger: !isSuspended
    });

    if (!confirmed) return;

    this.auth.toggleUserSuspend(user.id).subscribe({
      next: (newStatus) => {
        const msg = newStatus === 'suspended' ? 'Contul a fost suspendat.' : 'Contul a fost activat.';
        this.toast.success(msg);
        this.loadUsers();
      },
      error: (err) => {
        this.toast.error(err.error?.message || 'Eroare la modificarea statutului contului.');
      }
    });
  }

  copyToClipboard(text: string): void {
    navigator.clipboard.writeText(text).then(() => {
      this.toast.success('Copiat în clipboard!');
    });
  }
}
