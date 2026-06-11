import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export interface Workspace {
  id: string;
  name: string;
  slug: string;
  maxMemoryMb: number;
  maxStorageGb: number;
  cloudflareApiToken?: string | null;
  cloudflareZoneId?: string | null;
  ingressIp?: string | null;
  baseDomain?: string | null;
}

export interface WorkspaceUsage {
  workspaceId: string;
  maxMemoryMb: number;
  usedMemoryMb: number;
  maxStorageGb: number;
  usedStorageGb: number;
}

export interface AdminWorkspaceStats {
  id: string;
  name: string;
  slug: string;
  maxMemoryMb: number;
  maxStorageGb: number;
  createdAt: string;
  creator: string | null;
  memberCount: number;
  appCount: number;
  activeAppCount: number;
  databaseCount: number;
  allocatedMemoryMb: number;
  allocatedStorageGb: number;
}

@Injectable({
  providedIn: 'root'
})
export class WorkspaceService {
  private readonly api = inject(ApiService);

  listWorkspaces(): Observable<Workspace[]> {
    return this.api.get<Workspace[]>('/workspaces');
  }

  createWorkspace(name: string): Observable<Workspace> {
    return this.api.post<Workspace>('/workspaces', { name });
  }

  getUsage(): Observable<WorkspaceUsage> {
    return this.api.get<WorkspaceUsage>('/workspaces/usage');
  }

  getCurrentWorkspace(): Observable<Workspace> {
    return this.api.get<Workspace>('/workspaces/current');
  }

  updateWorkspace(payload: {
    name?: string;
    maxMemoryMb?: number;
    maxStorageGb?: number;
    cloudflareApiToken?: string | null;
    cloudflareZoneId?: string | null;
    ingressIp?: string | null;
    baseDomain?: string | null;
  }): Observable<Workspace> {
    return this.api.put<Workspace>('/workspaces', payload);
  }

  listMembers(): Observable<WorkspaceMember[]> {
    return this.api.get<WorkspaceMember[]>('/workspaces/members');
  }

  addMember(email: string, roleName: string): Observable<void> {
    return this.api.post<void>('/workspaces/members', { email, roleName });
  }

  updateMemberRole(userId: string, roleName: string): Observable<void> {
    return this.api.put<void>(`/workspaces/members/${userId}/role`, { roleName });
  }

  removeMember(userId: string): Observable<void> {
    return this.api.delete<void>(`/workspaces/members/${userId}`);
  }

  // --- Admin (super-admin only) ---
  adminListWorkspaces(): Observable<AdminWorkspaceStats[]> {
    return this.api.get<AdminWorkspaceStats[]>('/admin/workspaces');
  }

  adminDeleteWorkspace(workspaceId: string): Observable<void> {
    return this.api.delete<void>(`/workspaces/${workspaceId}`);
  }
}

export interface WorkspaceMember {
  userId: string;
  email: String;
  username: String;
  roleName: String;
}
