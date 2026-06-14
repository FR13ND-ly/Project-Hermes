import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';
import { Paginated, DEFAULT_PAGE_SIZE } from '../models/pagination';

export type DomainRoutingType = 'reverse_proxy' | 'static_host' | 'custom';
export type DomainStatus = 'pending_verification' | 'active' | 'failed';
export type DomainTargetType = 'app' | 'serverless' | 'database' | 'custom';

export interface Domain {
  id: string;
  fqdn: string;
  targetType: DomainTargetType;
  targetId?: string | null;
  targetName?: string | null;
  routingType: DomainRoutingType;
  status: DomainStatus;
  clientMaxBodySize: number;
  isSsl: boolean;
  nginxConfigContent?: string | null;
  cfProxyActive: boolean;
  nginxTargetHost?: string | null;
  nginxRootPath?: string | null;
}

export interface AddDomainRequest {
  fqdn: string;
  targetType: DomainTargetType;
  targetId?: string;
  routingType?: DomainRoutingType;
  clientMaxBodySize?: number;
  isSsl?: boolean;
  nginxTargetHost?: string;
  nginxRootPath?: string;
  nginxConfigContent?: string;
}

@Injectable({
  providedIn: 'root'
})
export class DomainService {
  private readonly api = inject(ApiService);

  listDomains(page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<Domain>> {
    return this.api.get<Paginated<Domain>>(`/domains?page=${page}&pageSize=${pageSize}`);
  }

  addDomain(payload: AddDomainRequest): Observable<Domain> {
    return this.api.post<Domain>('/domains', payload);
  }

  verifyAndSyncDomain(id: string): Observable<Domain> {
    return this.api.post<Domain>(`/domains/${id}/verify`, {});
  }

  removeDomain(id: string): Observable<any> {
    return this.api.delete<any>(`/domains/${id}`);
  }

  updateDomain(id: string, payload: AddDomainRequest): Observable<Domain> {
    return this.api.put<Domain>(`/domains/${id}`, payload);
  }

  // Real nginx access logs for a custom/attached domain (tailed server-side).
  getDomainLogs(id: string, lines = 200): Observable<{ lines: string[]; supported: boolean }> {
    return this.api.get<{ lines: string[]; supported: boolean }>(`/domains/${id}/logs?lines=${lines}`);
  }
}
