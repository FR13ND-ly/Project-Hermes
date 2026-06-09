import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

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

  listProjectIncidents(projectId: string): Observable<Incident[]> {
    return this.api.get<Incident[]>(`/projects/${projectId}/incidents`);
  }

  resolveIncident(incidentId: string): Observable<void> {
    return this.api.post<void>(`/incidents/${incidentId}/resolve`, {});
  }
}
