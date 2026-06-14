import { Injectable, inject, signal, computed } from '@angular/core';
import { Router } from '@angular/router';
import { ApiService } from './api.service';
import { Observable, tap, throwError } from 'rxjs';
import { HttpClient } from '@angular/common/http';
import { environment } from '../../../environments/environment';

export interface User {
  id: string;
  username: string;
  email: string;
  status: string;
  is_super_admin: boolean;
  current_workspace_id: string | null;
  last_login_at: string | null;
  created_at: string;
  github_username?: string | null;
}

export interface AuthResponse {
  accessToken: string;
  refreshToken: string;
  expiresIn: number;
  user: User;
}

@Injectable({
  providedIn: 'root',
})
export class AuthService {
  private readonly api = inject(ApiService);
  private readonly router = inject(Router);
  private readonly http = inject(HttpClient);

  readonly currentUser = signal<User | null>(null);
  readonly isAuthenticated = computed(() => this.currentUser() !== null);
  readonly currentWorkspaceId = computed(() => this.currentUser()?.current_workspace_id || null);

  constructor() {
    this.loadSession();
  }

  login(login_identity: string, password: string): Observable<AuthResponse> {
    return this.api.post<AuthResponse>('/auth/login', { login_identity, password }).pipe(
      tap((res) => {
        localStorage.setItem('hermes_token', res.accessToken);
        localStorage.setItem('hermes_refresh_token', res.refreshToken);
        localStorage.setItem('hermes_user', JSON.stringify(res.user));
        this.currentUser.set(res.user);
      })
    );
  }

  logout(): void {
    localStorage.removeItem('hermes_token');
    localStorage.removeItem('hermes_refresh_token');
    localStorage.removeItem('hermes_user');
    this.currentUser.set(null);
    this.router.navigate(['/auth']);
  }

  switchWorkspace(workspace_id: string): Observable<AuthResponse> {
    return this.api.post<AuthResponse>('/auth/switch-workspace', { workspace_id }).pipe(
      tap((res) => {
        localStorage.setItem('hermes_token', res.accessToken);
        localStorage.setItem('hermes_refresh_token', res.refreshToken);
        localStorage.setItem('hermes_user', JSON.stringify(res.user));
        this.currentUser.set(res.user);
      })
    );
  }

  refreshToken(): Observable<AuthResponse> {
    const refreshToken = localStorage.getItem('hermes_refresh_token');
    if (!refreshToken) {
      return throwError(() => new Error('No refresh token available'));
    }
    return this.http.post<AuthResponse>(`${environment.apiBaseUrl}/auth/refresh`, { refresh_token: refreshToken }).pipe(
      tap((res) => {
        localStorage.setItem('hermes_token', res.accessToken);
        localStorage.setItem('hermes_refresh_token', res.refreshToken);
        localStorage.setItem('hermes_user', JSON.stringify(res.user));
        this.currentUser.set(res.user);
      })
    );
  }

  updateUser(user: User): void {
    localStorage.setItem('hermes_user', JSON.stringify(user));
    this.currentUser.set(user);
  }

  provisionUser(username: string, email: string, isSuperAdmin: boolean): Observable<string> {
    return this.api.post<string>('/users/provision-user', { username, email, is_super_admin: isSuperAdmin });
  }

  listUsers(): Observable<User[]> {
    return this.api.get<User[]>('/users/users');
  }

  deleteUser(userId: string): Observable<void> {
    return this.api.delete<void>(`/users/users/${userId}`);
  }

  resetUserPassword(userId: string): Observable<string> {
    return this.api.post<string>(`/users/users/${userId}/reset-password`, {});
  }

  toggleUserSuspend(userId: string): Observable<string> {
    return this.api.post<string>(`/users/users/${userId}/toggle-suspend`, {});
  }

  activateAccount(email: string, temporaryPassword: string, newPassword: string): Observable<void> {
    return this.api.post<void>('/auth/activate', { email, temporary_password: temporaryPassword, new_password: newPassword });
  }

  private loadSession(): void {
    const token = localStorage.getItem('hermes_token');
    const userStr = localStorage.getItem('hermes_user');
    if (token && userStr) {
      try {
        const user = JSON.parse(userStr) as User;
        this.currentUser.set(user);
      } catch {
        this.logout();
      }
    }
  }
}
