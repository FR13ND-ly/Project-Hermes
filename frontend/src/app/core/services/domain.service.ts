import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export type DomainRoutingType = 'reverse_proxy' | 'static_host' | 'custom';
export type DomainStatus = 'pending_verification' | 'active' | 'failed';

export interface Domain {
  id: string;
  fqdn: string;
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
  routingType: DomainRoutingType;
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

  listDomains(): Observable<Domain[]> {
    return this.api.get<Domain[]>('/domains');
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
}
