import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';
import { Paginated, DEFAULT_PAGE_SIZE } from '../models/pagination';

export interface Incident {
  id: string;
  workspaceId: string;
  appInstanceId: string;
  instanceName: string;
  incidentType: string;
  message: string;
  resolvedAt: string | null;
  createdAt: string;
}

@Injectable({
  providedIn: 'root'
})
export class IncidentService {
  private readonly api = inject(ApiService);

  listProjectIncidents(projectId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<Incident>> {
    return this.api.get<Paginated<Incident>>(`/projects/${projectId}/incidents?page=${page}&pageSize=${pageSize}`);
  }

  resolveIncident(incidentId: string): Observable<void> {
    return this.api.post<void>(`/incidents/${incidentId}/resolve`, {});
  }
}
