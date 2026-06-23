import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';
import { environment } from '../../../environments/environment';

export interface AppUserWithRoles {
  appUserId: string;
  identifier: string;
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

export interface BaasService {
  id: string;
  projectId: string;
  name: string;
  slug: string;
  createdAt: string;
}

export interface AuthIntegration {
  baasId: string;
  apiBaseUrl: string;
  authSecretEnvKey: string;
  authSecret: string;
  registerEndpoint: string;
  loginEndpoint: string;
  refreshEndpoint: string;
  logoutEndpoint: string;
  verifyTokenEndpoint: string;
  verifyKeyEndpoint: string;
}

@Injectable({
  providedIn: 'root'
})
export class AuthManagementService {
  private readonly api = inject(ApiService);
  private readonly http = inject(HttpClient);
  private readonly baseUrl = environment.apiBaseUrl;

  // --- Standalone BaaS service resource (no app required) ---
  listServices(projectId: string): Observable<BaasService[]> {
    return this.api.get<BaasService[]>(`/projects/${projectId}/baas`);
  }

  createService(projectId: string, name: string): Observable<BaasService> {
    return this.api.post<BaasService>(`/baas`, { projectId, name });
  }

  deleteService(baasId: string): Observable<void> {
    return this.api.delete<void>(`/baas/${baasId}`);
  }

  listUsers(appId: string, page = 1, limit = 10, search = ''): Observable<PaginatedUsersResponse> {
    let url = `/baas/${appId}/users?page=${page}&limit=${limit}`;
    if (search.trim()) {
      url += `&search=${encodeURIComponent(search.trim())}`;
    }
    return this.api.get<PaginatedUsersResponse>(url);
  }

  updateUserStatus(appId: string, userId: string, status: string): Observable<void> {
    return this.api.post<void>(`/baas/${appId}/users/${userId}/status`, { status });
  }

  resetUserPassword(appId: string, userId: string, newPassword: string): Observable<void> {
    return this.api.post<void>(`/baas/${appId}/users/${userId}/reset-password`, { newPassword });
  }

  assignUserRole(appId: string, identifier: string, role: string): Observable<void> {
    return this.api.post<void>(`/baas/${appId}/users/roles`, { identifier, role });
  }

  removeUserRole(appId: string, payload: { appUserId: string; role: string }): Observable<void> {
    const token = localStorage.getItem('hermes_token');
    let headers = new HttpHeaders({ 'Content-Type': 'application/json' });
    if (token) {
      headers = headers.set('Authorization', `Bearer ${token}`);
    }
    return this.http.delete<void>(`${this.baseUrl}/baas/${appId}/users/roles`, {
      headers,
      body: payload
    });
  }

  getAuthConfig(appId: string): Observable<any> {
    return this.api.get<any>(`/baas/${appId}/auth-config`);
  }

  updateAuthConfig(appId: string, authRolesConfig: any): Observable<void> {
    return this.api.post<void>(`/baas/${appId}/auth-config`, { authRolesConfig });
  }

  listApiKeys(appId: string): Observable<ApiKeyInfo[]> {
    return this.api.get<ApiKeyInfo[]>(`/baas/${appId}/api-keys`);
  }

  createApiKey(appId: string, payload: { name: string; expiresAt: string | null }): Observable<CreateApiKeyResponse> {
    return this.api.post<CreateApiKeyResponse>(`/baas/${appId}/api-keys`, payload);
  }

  deleteApiKey(appId: string, keyId: string): Observable<void> {
    return this.api.delete<void>(`/baas/${appId}/api-keys/${keyId}`);
  }

  getIntegration(appId: string): Observable<AuthIntegration> {
    return this.api.get<AuthIntegration>(`/baas/${appId}/auth/integration`);
  }

  rotateAuthSecret(appId: string): Observable<{ auth_secret: string; auth_secret_env_key: string }> {
    return this.api.post<{ auth_secret: string; auth_secret_env_key: string }>(`/baas/${appId}/auth/rotate-secret`, {});
  }
}
