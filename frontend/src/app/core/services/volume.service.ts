import { Injectable, inject } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export interface VolumeInfo {
  id: string;
  appId: string;
  name: string;
  containerPath: string;
  hostPath: string;
}

export interface VolumeFileItem {
  name: string;
  isDir: boolean;
  sizeBytes: number;
  modifiedTime: number;
}

@Injectable({
  providedIn: 'root'
})
export class VolumeService {
  private readonly api = inject(ApiService);
  private readonly http = inject(HttpClient);
  private readonly baseUrl = 'http://localhost:8000/api/v1';

  private getAuthHeaders(): HttpHeaders {
    const token = localStorage.getItem('hermes_token');
    let headers = new HttpHeaders();
    if (token) {
      headers = headers.set('Authorization', `Bearer ${token}`);
    }
    return headers;
  }

  listVolumes(appId: string): Observable<VolumeInfo[]> {
    return this.api.get<VolumeInfo[]>(`/apps/${appId}/volumes`);
  }

  createVolume(payload: {
    appId: string;
    name: string;
    containerPath: string;
  }): Observable<VolumeInfo> {
    return this.api.post<VolumeInfo>('/volumes', payload);
  }

  deleteVolume(id: string): Observable<any> {
    return this.api.delete<any>(`/volumes/${id}`);
  }

  listFiles(volumeId: string, path: string): Observable<VolumeFileItem[]> {
    const encodedPath = encodeURIComponent(path);
    return this.api.get<VolumeFileItem[]>(`/volumes/${volumeId}/files?path=${encodedPath}`);
  }

  createFolder(volumeId: string, path: string, name: string): Observable<any> {
    return this.api.post<any>(`/volumes/${volumeId}/files/mkdir`, { path, name });
  }

  deleteFile(volumeId: string, path: string): Observable<any> {
    const encodedPath = encodeURIComponent(path);
    return this.api.delete<any>(`/volumes/${volumeId}/files?path=${encodedPath}`);
  }

  uploadFile(volumeId: string, path: string, file: File): Observable<any> {
    const formData = new FormData();
    formData.append('file', file, file.name);
    const encodedPath = encodeURIComponent(path);
    return this.http.post<any>(
      `${this.baseUrl}/volumes/${volumeId}/files/upload?path=${encodedPath}`,
      formData,
      { headers: this.getAuthHeaders() }
    );
  }

  uploadFileProgress(volumeId: string, path: string, file: File): Observable<any> {
    const formData = new FormData();
    formData.append('file', file, file.name);
    const encodedPath = encodeURIComponent(path);
    return this.http.post<any>(
      `${this.baseUrl}/volumes/${volumeId}/files/upload?path=${encodedPath}`,
      formData,
      {
        headers: this.getAuthHeaders(),
        reportProgress: true,
        observe: 'events'
      }
    );
  }

  downloadFileUrl(volumeId: string, path: string): string {
    const token = localStorage.getItem('hermes_token') || '';
    const encodedPath = encodeURIComponent(path);
    return `${this.baseUrl}/volumes/${volumeId}/files/download?path=${encodedPath}&token=${encodeURIComponent(token)}`;
  }
}
