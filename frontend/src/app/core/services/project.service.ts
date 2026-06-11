import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';

export interface Project {
  id: string;
  name: string;
  description: string | null;
  workspace_id: string;
  created_at: string;
}

export interface AppInstance {
  id: string;
  branchName: string;
  instanceType: string;
  status: string;
  internalPort: number;
  assignedDomain: string | null;
  containerName: string;
  externalPort?: number;
  cpuLimit?: number;
  memoryLimitMb?: number;
  createdAt?: string;
  metaData?: any;
}

export interface AppDetail {
  id: string;
  project_id: string;
  name: string;
  git_repo_url: string;
  build_status: string;
  tcp_udp_ports: any[] | null;
  created_at: string;
  instances: AppInstance[];
  buildCommand?: string | null;
  startCommand?: string | null;
}

export interface AppBuild {
  id: string;
  appId: string;
  appInstanceId: string;
  branchName: string;
  status: string;
  phase?: string;
  failureReason?: string | null;
  failureCategory?: string | null;
  logs: string | null;
  createdAt: string;
  commitMessage?: string;
  commitSha?: string;
  durationSec?: number;
}

export interface MetricsHistory {
  timestamps: number[];
  values: number[];
  simulated?: boolean;
}

@Injectable({
  providedIn: 'root'
})
export class ProjectService {
  private readonly api = inject(ApiService);

  listProjects(): Observable<Project[]> {
    return this.api.get<Project[]>('/projects');
  }

  getProject(id: string): Observable<Project> {
    return this.api.get<Project>(`/projects/${id}`);
  }

  listProjectApps(projectId: string): Observable<AppDetail[]> {
    return this.api.get<AppDetail[]>(`/projects/${projectId}/apps`);
  }

  createProject(name: string, description: string | null): Observable<Project> {
    return this.api.post<Project>('/projects', { name, description });
  }

  createApp(payload: {
    projectId: string;
    name: string;
    gitRepository: string;
    branchName?: string;
    buildCommand?: string;
    startCommand?: string;
    internalPort?: number;
    externalPort?: number;
    gitSubpath?: string;
    envVariables?: EnvVarInput[];
  }): Observable<any> {
    return this.api.post<any>('/apps', payload);
  }

  getAppDetails(appId: string): Observable<AppDetail> {
    return this.api.get<AppDetail>(`/apps/${appId}`);
  }

  deleteAppInstance(appId: string, instanceId: string): Observable<any> {
    return this.api.delete<any>(`/apps/${appId}/instances/${instanceId}`);
  }

  deleteApp(appId: string): Observable<any> {
    return this.api.delete<any>(`/apps/${appId}`);
  }

  deleteProject(projectId: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}`);
  }

  listBuilds(appId: string): Observable<AppBuild[]> {
    return this.api.get<AppBuild[]>(`/apps/${appId}/builds`);
  }

  getBuildDetails(appId: string, buildId: string): Observable<AppBuild> {
    return this.api.get<AppBuild>(`/apps/${appId}/builds/${buildId}`);
  }

  retryBuild(appId: string, buildId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/builds/${buildId}/retry`, {});
  }

  cancelBuild(appId: string, buildId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/builds/${buildId}/cancel`, {});
  }

  rollbackBuild(appId: string, buildId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/builds/${buildId}/rollback`, {});
  }

  getMetrics(appId: string, instanceId: string, metric: string, range: string): Observable<MetricsHistory> {
    return this.api.get<MetricsHistory>(`/apps/${appId}/instances/${instanceId}/metrics?metric=${metric}&range=${range}`);
  }

  createBranchInstance(appId: string, branchName: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/branches`, { branch: branchName });
  }

  // Helper method for EventSource URL for instance stats telemetry SSE stream
  getStatsStreamUrl(appId: string, instanceId: string): string {
    return this.api.getStreamUrl(`/apps/${appId}/instances/${instanceId}/stats`);
  }

  // WebSocket URL for live container logs (push, no polling)
  getLogsWsUrl(appId: string, instanceId: string): string {
    return this.api.getWsUrl(`/apps/${appId}/instances/${instanceId}/logs/ws`);
  }

  // Helper method for EventSource URL for Kaniko builder/cloner pod log SSE stream
  getBuildLogsStreamUrl(appId: string, buildId: string): string {
    return this.api.getStreamUrl(`/apps/${appId}/builds/${buildId}/logs/stream`);
  }

  listEnvVariables(appInstanceId: string): Observable<EnvResponse[]> {
    return this.api.get<EnvResponse[]>(`/envs?appInstanceId=${appInstanceId}`);
  }

  listProjectEnvsGrouped(projectId: string): Observable<GroupedAppEnv[]> {
    return this.api.get<GroupedAppEnv[]>(`/projects/${projectId}/envs-grouped`);
  }

  setEnvVariable(payload: {
    appInstanceId: string;
    key: string;
    value: string;
    isSecret?: boolean;
  }): Observable<EnvResponse> {
    return this.api.post<EnvResponse>('/envs', {
      appInstanceId: payload.appInstanceId,
      key: payload.key,
      value: payload.value,
      isSecret: payload.isSecret !== undefined ? payload.isSecret : true
    });
  }

  setEnvsBulk(appInstanceId: string, variables: EnvVarInput[]): Observable<EnvResponse[]> {
    return this.api.post<EnvResponse[]>('/envs/bulk', { appInstanceId, variables });
  }

  deleteEnvVariable(id: string): Observable<any> {
    return this.api.delete<any>(`/envs/${id}`);
  }

  // --- Project-level env pool ---

  listProjectEnv(projectId: string): Observable<ProjectEnvResponse[]> {
    return this.api.get<ProjectEnvResponse[]>(`/projects/${projectId}/env`);
  }

  setProjectEnv(projectId: string, payload: { key: string; value: string; isSecret?: boolean }): Observable<ProjectEnvResponse> {
    return this.api.post<ProjectEnvResponse>(`/projects/${projectId}/env`, {
      key: payload.key,
      value: payload.value,
      isSecret: payload.isSecret !== undefined ? payload.isSecret : true
    });
  }

  deleteProjectEnv(projectId: string, id: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}/env/${id}`);
  }

  // Project-pool vars available to an instance, each flagged as linked or not.
  listInstanceProjectEnv(instanceId: string): Observable<ProjectEnvResponse[]> {
    return this.api.get<ProjectEnvResponse[]>(`/instances/${instanceId}/project-env`);
  }

  linkProjectEnv(instanceId: string, projectEnvId: string): Observable<any> {
    return this.api.post<any>(`/instances/${instanceId}/env-links`, { projectEnvId });
  }

  unlinkProjectEnv(instanceId: string, projectEnvId: string): Observable<any> {
    return this.api.delete<any>(`/instances/${instanceId}/env-links/${projectEnvId}`);
  }

  updateInstanceSettings(appId: string, instanceId: string, payload: {
    cpuLimit?: number;
    memoryLimitMb?: number;
    internalPort?: number;
    externalPort?: number | null;
    buildCommand?: string | null;
    startCommand?: string | null;
  }): Observable<any> {
    return this.api.patch<any>(`/apps/${appId}/instances/${instanceId}/settings`, payload);
  }

  stopAppInstance(appId: string, instanceId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/instances/${instanceId}/stop`, {});
  }

  startAppInstance(appId: string, instanceId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/instances/${instanceId}/start`, {});
  }

  redeployAppInstance(appId: string, instanceId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/instances/${instanceId}/redeploy`, {});
  }

  // --- Cron Jobs API ---
  listProjectCronJobs(projectId: string): Observable<CronJob[]> {
    return this.api.get<CronJob[]>(`/projects/${projectId}/cron`);
  }

  listCronJobs(appId: string): Observable<CronJob[]> {
    return this.api.get<CronJob[]>(`/apps/${appId}/cron`);
  }

  createCronJob(payload: {
    projectId: string;
    appId: string;
    name: string;
    schedule: string;
    command: string;
  }): Observable<CronJob> {
    return this.api.post<CronJob>('/cron', payload);
  }

  deleteCronJob(jobId: string): Observable<any> {
    return this.api.delete<any>(`/cron/${jobId}`);
  }

  updateCronJob(jobId: string, payload: {
    name?: string;
    schedule?: string;
    command?: string;
    appId?: string;
    status?: 'active' | 'paused' | 'failed';
  }): Observable<CronJob> {
    return this.api.patch<CronJob>(`/cron/${jobId}`, payload);
  }

  listCronJobLogs(jobId: string, page: number = 1, limit: number = 10): Observable<{ logs: CronJobLog[], total: number, page: number, limit: number, pages: number }> {
    return this.api.get<any>(`/cron/${jobId}/logs?page=${page}&limit=${limit}`);
  }

  configureServerless(appId: string, instanceId: string, payload: {
    enabled: boolean;
    minScale: number;
    maxScale: number;
    targetConcurrency: number;
  }): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/instances/${instanceId}/serverless`, payload);
  }

  listProjectWebhooks(projectId: string): Observable<ProjectWebhook[]> {
    return this.api.get<ProjectWebhook[]>(`/projects/${projectId}/webhooks`);
  }

  createWebhook(projectId: string, payload: {
    name: string;
    url: string;
    webhookType: string;
  }): Observable<ProjectWebhook> {
    return this.api.post<ProjectWebhook>(`/projects/${projectId}/webhooks`, payload);
  }

  deleteWebhook(projectId: string, webhookId: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}/webhooks/${webhookId}`);
  }

  listProjectSshKeys(projectId: string): Observable<ProjectSshKey[]> {
    return this.api.get<ProjectSshKey[]>(`/projects/${projectId}/ssh-keys`);
  }

  createProjectSshKey(projectId: string, payload: {
    name: string;
    host: string;
    privateKey?: string | null;
  }): Observable<ProjectSshKey> {
    return this.api.post<ProjectSshKey>(`/projects/${projectId}/ssh-keys`, payload);
  }

  deleteProjectSshKey(projectId: string, keyId: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}/ssh-keys/${keyId}`);
  }

  listProjectFunctions(projectId: string): Observable<ServerlessFunction[]> {
    return this.api.get<ServerlessFunction[]>(`/projects/${projectId}/functions`);
  }

  createFunction(projectId: string, payload: { name: string, code?: string, method: string, routePath: string, memoryLimitMb?: number }): Observable<ServerlessFunction> {
    return this.api.post<ServerlessFunction>(`/projects/${projectId}/functions`, payload);
  }

  updateFunction(projectId: string, id: string, payload: { name?: string, code?: string, method?: string, routePath?: string, memoryLimitMb?: number, envVariables?: any, assignedDomain?: string | null }): Observable<ServerlessFunction> {
    return this.api.put<ServerlessFunction>(`/projects/${projectId}/functions/${id}`, payload);
  }

  deleteFunction(projectId: string, id: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}/functions/${id}`);
  }

  deployFunction(projectId: string, id: string): Observable<any> {
    return this.api.post<any>(`/projects/${projectId}/functions/${id}/deploy`, {});
  }

  getFunctionDetails(projectId: string, id: string): Observable<ServerlessFunction> {
    return this.api.get<ServerlessFunction>(`/projects/${projectId}/functions/${id}`);
  }
}

export interface ProjectSshKey {
  id: string;
  projectId: string;
  name: string;
  host: string;
  publicKey: string;
  createdAt: string;
}

export interface ProjectWebhook {
  id: string;
  projectId: string;
  name: string;
  url: string;
  webhookType: string;
  isActive: boolean;
  createdAt: string;
}

export interface CronJob {
  id: string;
  projectId?: string;
  project_id?: string;
  appId?: string;
  app_id?: string;
  name: string;
  schedule: string;
  command: string;
  status: 'active' | 'paused' | 'failed';
  nextRunAt?: string | null;
  next_run_at?: string | null;
  source?: 'user' | 'backup';
  databaseId?: string | null;
}

export interface CronJobLog {
  id: string;
  cronJobId?: string;
  cron_job_id?: string;
  exitCode?: number;
  exit_code?: number;
  output?: string | null;
  startedAt?: string;
  started_at?: string;
  finishedAt?: string;
  finished_at?: string;
}

export interface EnvResponse {
  id: string;
  appInstanceId: string;
  key: string;
  value: string | null;
  isSecret: boolean;
}

export interface EnvVarInput {
  key: string;
  value: string;
  isSecret?: boolean;
}

export interface ProjectEnvResponse {
  id: string;
  projectId: string;
  key: string;
  value: string | null;
  isSecret: boolean;
  source: string;        // manual | database | storage | serverless
  linked?: boolean;      // only set when listed in an instance context
}

export interface GroupedInstanceEnv {
  instanceId: string;
  branchName: string;
  variables: EnvResponse[];
}

export interface GroupedAppEnv {
  appId: string;
  appName: string;
  instances: GroupedInstanceEnv[];
}

export interface ServerlessFunction {
  id: string;
  workspaceId: string;
  projectId: string;
  name: string;
  code: string;
  method: string;
  routePath: string;
  memoryLimitMb: number;
  envVariables: any;
  status: 'draft' | 'building' | 'active' | 'failed';
  assignedDomain: string | null;
  buildLogs: string | null;
  externalPort: number | null;
  createdAt: string;
  updatedAt: string;
}
