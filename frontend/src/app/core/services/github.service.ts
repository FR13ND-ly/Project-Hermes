import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export interface GithubRepoOwner {
  login: string;
}

export interface GithubRepo {
  id: number;
  name: string;
  full_name: string;
  owner: GithubRepoOwner;
  private: boolean;
  html_url: string;
  description: string | null;
  updated_at: string | null;
}

export interface GithubBranch {
  name: string;
}

export interface ProjectDetectionResponse {
  projectType: string;
  buildCommand: string;
  startCommand: string;
  internalPort: number;
  description: string;
}

@Injectable({
  providedIn: 'root'
})
export class GithubService {
  private readonly api = inject(ApiService);

  saveToken(token: string | null): Observable<any> {
    return this.api.post<any>('/github/token', { token });
  }

  listRepos(): Observable<GithubRepo[]> {
    return this.api.get<GithubRepo[]>('/github/repos');
  }

  listBranches(owner: string, repo: string): Observable<GithubBranch[]> {
    return this.api.get<GithubBranch[]>(`/github/repos/${owner}/${repo}/branches`);
  }

  detectProjectType(owner: string, repo: string, path?: string): Observable<ProjectDetectionResponse> {
    const url = `/github/repos/${owner}/${repo}/detect` + (path ? `?path=${encodeURIComponent(path)}` : '');
    return this.api.get<ProjectDetectionResponse>(url);
  }
}
