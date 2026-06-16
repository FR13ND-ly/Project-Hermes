import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export type GitProvider = 'github' | 'gitlab';

export interface GitCredential {
  id: string;
  provider: GitProvider;
  host: string;
  label: string;
  username: string | null;
  createdAt: string;
  skipTlsVerify: boolean;
}

export interface GitRepo {
  /** owner/name (GitHub) or namespace/path (GitLab) — the repo ref used everywhere. */
  fullPath: string;
  name: string;
  private: boolean;
  defaultBranch: string | null;
  htmlUrl: string | null;
}

export interface DetectedEnvVar {
  key: string;
  value: string;
}

export interface ProjectDetectionResponse {
  projectType: string;
  buildCommand: string;
  startCommand: string;
  internalPort: number;
  description: string;
  detectedEnvs?: DetectedEnvVar[];
  subdirectories?: string[];
}

export interface ComposeFileResponse {
  found: boolean;
  filename?: string | null;
  yaml: string;
}

@Injectable({ providedIn: 'root' })
export class GitService {
  private readonly api = inject(ApiService);

  // --- Workspace credentials (PATs) ---
  listCredentials(): Observable<GitCredential[]> {
    return this.api.get<GitCredential[]>('/git/credentials');
  }

  createCredential(payload: { provider: GitProvider; host?: string; label: string; token: string; skipTlsVerify?: boolean }): Observable<GitCredential> {
    return this.api.post<GitCredential>('/git/credentials', payload);
  }

  deleteCredential(id: string): Observable<void> {
    return this.api.delete<void>(`/git/credentials/${id}`);
  }

  // --- Repo browsing through a chosen credential ---
  listRepos(credentialId: string): Observable<GitRepo[]> {
    return this.api.get<GitRepo[]>(`/git/credentials/${credentialId}/repos`);
  }

  listBranches(credentialId: string, repo: string): Observable<string[]> {
    return this.api.get<string[]>(`/git/credentials/${credentialId}/branches?repo=${encodeURIComponent(repo)}`);
  }

  detect(credentialId: string, repo: string, path?: string, ref?: string): Observable<ProjectDetectionResponse> {
    let url = `/git/credentials/${credentialId}/detect?repo=${encodeURIComponent(repo)}`;
    if (path) url += `&path=${encodeURIComponent(path)}`;
    if (ref) url += `&ref=${encodeURIComponent(ref)}`;
    return this.api.get<ProjectDetectionResponse>(url);
  }

  getCompose(credentialId: string, repo: string, path?: string, ref?: string): Observable<ComposeFileResponse> {
    let url = `/git/credentials/${credentialId}/compose?repo=${encodeURIComponent(repo)}`;
    if (path) url += `&path=${encodeURIComponent(path)}`;
    if (ref) url += `&ref=${encodeURIComponent(ref)}`;
    return this.api.get<ComposeFileResponse>(url);
  }
}
