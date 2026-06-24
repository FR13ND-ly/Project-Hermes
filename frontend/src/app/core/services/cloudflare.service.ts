import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';
import { ApiService } from './api.service';

/** A workspace-level Cloudflare credential (token never returned). */
export interface CloudflareCredential {
  id: string;
  label: string;
  zoneId: string;
  baseDomain: string | null;
  createdAt: string;
}

@Injectable({ providedIn: 'root' })
export class CloudflareService {
  private readonly api = inject(ApiService);

  listCredentials(): Observable<CloudflareCredential[]> {
    return this.api.get<CloudflareCredential[]>('/cloudflare-credentials');
  }

  createCredential(payload: { label: string; token: string; zoneId: string; baseDomain?: string }): Observable<CloudflareCredential> {
    return this.api.post<CloudflareCredential>('/cloudflare-credentials', payload);
  }

  deleteCredential(id: string): Observable<void> {
    return this.api.delete<void>(`/cloudflare-credentials/${id}`);
  }
}
