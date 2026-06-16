import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';
import { Paginated, DEFAULT_PAGE_SIZE } from '../models/pagination';

export interface DatabaseServiceInfo {
  id: string;
  projectId: string;
  appInstanceId: string | null;
  name: string;
  type: 'postgres' | 'mysql' | 'redis' | 'mongodb';
  version: string;
  dbUser: string;
  dbName: string;
  containerName: string;
  internalPort: number;
  isExternal: boolean;
  externalPort: number | null;
  status: 'provisioning' | 'running' | 'stopped' | 'failed';
  cpuLimit: number;
  memoryLimitMb: number;
  storageSizeGb: number;
  backupEnabled?: boolean;
  backupCount?: number;
  lastBackupAt?: string | null;
  connectionUrl?: string;
  createdAt: string;
  updatedAt: string;
}

export interface DbBackup {
  id: string;
  databaseId: string;
  filename: string;
  fileSizeBytes: number;
  status: string;
  createdAt: string;
}

@Injectable({
  providedIn: 'root'
})
export class DatabaseService {
  private readonly api = inject(ApiService);

  createDatabase(payload: {
    projectId: string;
    appInstanceId?: string | null;
    name: string;
    type: 'postgres' | 'mysql' | 'redis' | 'mongodb';
    version?: string;
    cpuLimit?: number;
    memoryLimitMb?: number;
    storageSizeGb?: number;
    isExternal?: boolean;
    externalPort?: number;
    publishToEnv?: boolean;
    envKey?: string;
  }): Observable<DatabaseServiceInfo> {
    return this.api.post<DatabaseServiceInfo>('/databases', payload);
  }

  listDatabases(projectId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<DatabaseServiceInfo>> {
    return this.api.get<Paginated<DatabaseServiceInfo>>(`/databases?projectId=${projectId}&page=${page}&pageSize=${pageSize}`);
  }

  getDatabase(id: string): Observable<DatabaseServiceInfo> {
    return this.api.get<DatabaseServiceInfo>(`/databases/${id}`);
  }

  revealCredentials(id: string): Observable<{ connectionUrl: string; databaseUser?: string; databasePassword?: string }> {
    return this.api.post<{ connectionUrl: string; databaseUser?: string; databasePassword?: string }>(`/databases/${id}/reveal`, {});
  }

  rotatePassword(id: string): Observable<{ status: string; reloaded_instance: string | null }> {
    return this.api.post<{ status: string; reloaded_instance: string | null }>(`/databases/${id}/rotate-password`, {});
  }

  runQuery(id: string, query: string): Observable<{ output: string; isError: boolean }> {
    return this.api.post<{ output: string; isError: boolean }>(`/databases/${id}/query`, { query });
  }

  deleteDatabase(id: string): Observable<any> {
    return this.api.delete<any>(`/databases/${id}`);
  }

  getLogsStreamUrl(id: string): string {
    return this.api.getStreamUrl(`/databases/${id}/logs`);
  }

  updateSettings(id: string, payload: { 
    cpuLimit: number; 
    memoryLimitMb: number;
    backupEnabled?: boolean;
    backupCount?: number;
  }): Observable<any> {
    return this.api.post<any>(`/databases/${id}/settings`, payload);
  }

  listBackups(dbId: string): Observable<DbBackup[]> {
    return this.api.get<DbBackup[]>(`/databases/${dbId}/backups`);
  }

  createBackup(dbId: string): Observable<DbBackup> {
    return this.api.post<DbBackup>(`/databases/${dbId}/backups`, {});
  }

  deleteBackup(dbId: string, backupId: string): Observable<any> {
    return this.api.delete<any>(`/databases/${dbId}/backups/${backupId}`);
  }

  restoreBackup(dbId: string, backupId: string): Observable<any> {
    return this.api.post<any>(`/databases/${dbId}/backups/${backupId}/restore`, {});
  }

  getMetrics(dbId: string, metric: string, range: string): Observable<DbMetricsHistory> {
    return this.api.get<DbMetricsHistory>(`/databases/${dbId}/metrics?metric=${metric}&range=${range}`);
  }

  // The managed backup cron for this database (null when auto-backup is off).
  getBackupCron(dbId: string): Observable<BackupCron | null> {
    return this.api.get<BackupCron | null>(`/databases/${dbId}/backup-cron`);
  }
}

export interface BackupCron {
  id: string;
  name: string;
  schedule: string;
  command: string;
  status: string;
  nextRunAt?: string | null;
}

export interface DbMetricsHistory {
  timestamps: number[];
  values: number[];
  simulated?: boolean;
}
