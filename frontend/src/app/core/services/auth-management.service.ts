import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export interface AppUserWithRoles {
  appUserId: string;
  email: string;
  fullName: string;
  status: string;
  lastLogin: string | null;
  roles: string[];
}

export interface ApiKeyInfo {
  id: string;
  name: string;
  keyPrefix: string;
  createdAt: string;
  expiresAt: string | null;
  lastUsedAt: string | null;
}

export interface CreateApiKeyResponse {
  id: string;
  name: string;
  keyPrefix: string;
  rawKey: string;
  createdAt: string;
  expiresAt: string | null;
}

export interface PaginatedUsersResponse {
  users: AppUserWithRoles[];
  total: number;
  page: number;
  limit: number;
  pages: number;
}

@Injectable({
  providedIn: 'root'
})
export class AuthManagementService {
  private readonly api = inject(ApiService);
  private readonly http = inject(HttpClient);
  private readonly baseUrl = 'http://localhost:8000/api/v1';

  listUsers(appId: string, page = 1, limit = 10, search = ''): Observable<PaginatedUsersResponse> {
    let url = `/apps/${appId}/users?page=${page}&limit=${limit}`;
    if (search.trim()) {
      url += `&search=${encodeURIComponent(search.trim())}`;
    }
    return this.api.get<PaginatedUsersResponse>(url);
  }

  updateUserStatus(appId: string, userId: string, status: string): Observable<void> {
    return this.api.post<void>(`/apps/${appId}/users/${userId}/status`, { status });
  }

  resetUserPassword(appId: string, userId: string, newPasswordHash: string): Observable<void> {
    return this.api.post<void>(`/apps/${appId}/users/${userId}/reset-password`, { newPasswordHash });
  }

  assignUserRole(appId: string, email: string, role: string): Observable<void> {
    return this.api.post<void>(`/apps/${appId}/users/roles`, { email, role });
  }

  removeUserRole(appId: string, payload: { appUserId: string; role: string }): Observable<void> {
    const token = localStorage.getItem('hermes_token');
    let headers = new HttpHeaders({ 'Content-Type': 'application/json' });
    if (token) {
      headers = headers.set('Authorization', `Bearer ${token}`);
    }
    return this.http.delete<void>(`${this.baseUrl}/apps/${appId}/users/roles`, {
      headers,
      body: payload
    });
  }

  getAuthConfig(appId: string): Observable<any> {
    return this.api.get<any>(`/apps/${appId}/auth-config`);
  }

  updateAuthConfig(appId: string, authRolesConfig: any): Observable<void> {
    return this.api.post<void>(`/apps/${appId}/auth-config`, { authRolesConfig });
  }

  listApiKeys(appId: string): Observable<ApiKeyInfo[]> {
    return this.api.get<ApiKeyInfo[]>(`/apps/${appId}/api-keys`);
  }

  createApiKey(appId: string, payload: { name: string; expiresAt: string | null }): Observable<CreateApiKeyResponse> {
    return this.api.post<CreateApiKeyResponse>(`/apps/${appId}/api-keys`, payload);
  }

  deleteApiKey(appId: string, keyId: string): Observable<void> {
    return this.api.delete<void>(`/apps/${appId}/api-keys/${keyId}`);
  }
}
