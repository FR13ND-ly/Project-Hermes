import { Injectable, inject } from '@angular/core';
import { ApiService } from './api.service';
import { Observable } from 'rxjs';
import { Paginated, DEFAULT_PAGE_SIZE } from '../models/pagination';

export interface Project {
  id: string;
  name: string;
  description: string | null;
  workspace_id: string;
  created_at: string;
}

// Cloudflare / Ingress settings (project-level). The API token is never returned.
export interface ProjectSettings {
  cloudflareZoneId: string | null;
  ingressIp: string | null;
  baseDomain: string | null;
  hasCloudflareToken: boolean;
}

export interface UpdateProjectSettingsRequest {
  cloudflareApiToken?: string | null;
  cloudflareZoneId?: string | null;
  ingressIp?: string | null;
  baseDomain?: string | null;
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
  imageTag?: string | null;
  isLive?: boolean;
}

export interface BuildQueueItem {
  id: string;
  kind: 'app' | 'database' | 'serverless';
  resourceId: string;
  name: string;
  detail?: string | null;
  projectId: string;
  projectName: string;
  workspaceId: string;
  workspaceName: string;
  status: string;
  createdAt: string;
}

export interface PlanEnv { key: string; value: string; }
export interface PlanVolume { service: string; name: string; containerPath: string; }
export interface PlanApp {
  service: string;
  name: string;
  image?: string | null;
  buildPath?: string | null;
  internalPort: number;
  externalPort?: number | null;
  buildable: boolean;
  env: PlanEnv[];
  volumes: PlanVolume[];
  dependsOn: string[];
  include: boolean;
}
export interface PlanDatabase {
  service: string;
  name: string;
  dbType: string;
  version: string;
  internalPort: number;
  include: boolean;
}
export interface ComposePlan {
  apps: PlanApp[];
  databases: PlanDatabase[];
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

  getProjectSettings(projectId: string): Observable<ProjectSettings> {
    return this.api.get<ProjectSettings>(`/projects/${projectId}/settings`);
  }

  updateProjectSettings(projectId: string, payload: UpdateProjectSettingsRequest): Observable<ProjectSettings> {
    return this.api.patch<ProjectSettings>(`/projects/${projectId}/settings`, payload);
  }

  listProjectApps(projectId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<AppDetail>> {
    return this.api.get<Paginated<AppDetail>>(`/projects/${projectId}/apps?page=${page}&pageSize=${pageSize}`);
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
    gitCredentialId?: string;
    envVariables?: EnvVarInput[];
    linkedProjectEnvIds?: string[];
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

  listBuilds(appId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<AppBuild>> {
    return this.api.get<Paginated<AppBuild>>(`/apps/${appId}/builds?page=${page}&pageSize=${pageSize}`);
  }

  // Global build queue (queued + building), oldest first.
  listBuildQueue(): Observable<BuildQueueItem[]> {
    return this.api.get<BuildQueueItem[]>('/build-queue');
  }

  // Docker-compose auto-split: preview a plan, then apply it.
  planComposeSplit(composeYaml: string): Observable<ComposePlan> {
    return this.api.post<ComposePlan>('/stacks/plan', { composeYaml });
  }

  applyComposeSplit(payload: { projectId: string; gitRepository?: string; gitCredentialId?: string; branchName?: string; plan: ComposePlan }): Observable<any> {
    return this.api.post<any>('/stacks/apply', payload);
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

  listEnvVariables(appInstanceId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<EnvResponse>> {
    return this.api.get<Paginated<EnvResponse>>(`/envs?appInstanceId=${appInstanceId}&page=${page}&pageSize=${pageSize}`);
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

  // Rename a project-pool var's key (works for manual and resource-published vars).
  renameProjectEnv(projectId: string, id: string, key: string): Observable<ProjectEnvResponse> {
    return this.api.patch<ProjectEnvResponse>(`/projects/${projectId}/env/${id}`, { key });
  }

  // Decrypt and fetch a single project-pool var's value on explicit reveal.
  revealProjectEnv(projectId: string, id: string): Observable<{ value: string }> {
    return this.api.get<{ value: string }>(`/projects/${projectId}/env/${id}/reveal`);
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

  // --- Serverless instance project-pool links (instance-level env) ---
  listFunctionProjectEnv(projectId: string, instanceId: string): Observable<ProjectEnvResponse[]> {
    return this.api.get<ProjectEnvResponse[]>(`/projects/${projectId}/serverless/${instanceId}/project-env`);
  }

  linkFunctionProjectEnv(projectId: string, instanceId: string, projectEnvId: string): Observable<any> {
    return this.api.post<any>(`/projects/${projectId}/serverless/${instanceId}/env-links`, { projectEnvId });
  }

  unlinkFunctionProjectEnv(projectId: string, instanceId: string, projectEnvId: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}/serverless/${instanceId}/env-links/${projectEnvId}`);
  }

  // --- Serverless instance own env vars ---
  listFunctionEnv(projectId: string, instanceId: string): Observable<FunctionEnvResponse[]> {
    return this.api.get<FunctionEnvResponse[]>(`/projects/${projectId}/serverless/${instanceId}/env`);
  }

  setFunctionEnv(projectId: string, instanceId: string, payload: { key: string, value: string, isSecret: boolean }): Observable<FunctionEnvResponse> {
    return this.api.post<FunctionEnvResponse>(`/projects/${projectId}/serverless/${instanceId}/env`, payload);
  }

  deleteFunctionEnv(projectId: string, instanceId: string, envId: string): Observable<void> {
    return this.api.delete<void>(`/projects/${projectId}/serverless/${instanceId}/env/${envId}`);
  }

  // Re-apply env on the running Knative service without a rebuild.
  reloadFunctionEnv(projectId: string, instanceId: string): Observable<ServerlessInstance> {
    return this.api.post<ServerlessInstance>(`/projects/${projectId}/serverless/${instanceId}/reload-env`, {});
  }

  // --- Serverless routes (inside an instance) ---
  listRoutes(projectId: string, instanceId: string): Observable<ServerlessRoute[]> {
    return this.api.get<ServerlessRoute[]>(`/projects/${projectId}/serverless/${instanceId}/routes`);
  }

  createRoute(projectId: string, instanceId: string, payload: { method: string, routePath: string, code?: string }): Observable<ServerlessRoute> {
    return this.api.post<ServerlessRoute>(`/projects/${projectId}/serverless/${instanceId}/routes`, payload);
  }

  updateRoute(projectId: string, instanceId: string, routeId: string, payload: { method?: string, routePath?: string, code?: string }): Observable<ServerlessRoute> {
    return this.api.put<ServerlessRoute>(`/projects/${projectId}/serverless/${instanceId}/routes/${routeId}`, payload);
  }

  deleteRoute(projectId: string, instanceId: string, routeId: string): Observable<void> {
    return this.api.delete<void>(`/projects/${projectId}/serverless/${instanceId}/routes/${routeId}`);
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

  // Redeploy = full rebuild from Git.
  redeployAppInstance(appId: string, instanceId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/instances/${instanceId}/redeploy`, {});
  }

  // Reload = re-apply the current image with fresh config/env (no rebuild).
  reloadAppInstance(appId: string, instanceId: string): Observable<any> {
    return this.api.post<any>(`/apps/${appId}/instances/${instanceId}/reload`, {});
  }

  // --- Cron Jobs API ---
  listProjectCronJobs(projectId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<CronJob>> {
    return this.api.get<Paginated<CronJob>>(`/projects/${projectId}/cron?page=${page}&pageSize=${pageSize}`);
  }

  listCronJobs(appId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<CronJob>> {
    return this.api.get<Paginated<CronJob>>(`/apps/${appId}/cron?page=${page}&pageSize=${pageSize}`);
  }

  createCronJob(payload: {
    projectId: string;
    targetType: 'app' | 'database' | 'storage';
    targetId: string;
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

  // --- Serverless instances ---
  listProjectFunctions(projectId: string, page = 1, pageSize = DEFAULT_PAGE_SIZE): Observable<Paginated<ServerlessInstance>> {
    return this.api.get<Paginated<ServerlessInstance>>(`/projects/${projectId}/serverless?page=${page}&pageSize=${pageSize}`);
  }

  createInstance(projectId: string, payload: { name: string, runtime?: string, memoryLimitMb?: number }): Observable<ServerlessInstance> {
    return this.api.post<ServerlessInstance>(`/projects/${projectId}/serverless`, payload);
  }

  updateInstance(projectId: string, id: string, payload: { name?: string, runtime?: string, memoryLimitMb?: number, assignedDomain?: string | null, inheritProjectEnvs?: boolean }): Observable<ServerlessInstance> {
    return this.api.put<ServerlessInstance>(`/projects/${projectId}/serverless/${id}`, payload);
  }

  deleteFunction(projectId: string, id: string): Observable<any> {
    return this.api.delete<any>(`/projects/${projectId}/serverless/${id}`);
  }

  deployFunction(projectId: string, id: string): Observable<{ buildId: string }> {
    return this.api.post<{ buildId: string }>(`/projects/${projectId}/serverless/${id}/deploy`, {});
  }

  getFunctionDetails(projectId: string, id: string): Observable<ServerlessInstance> {
    return this.api.get<ServerlessInstance>(`/projects/${projectId}/serverless/${id}`);
  }

  // Historical CPU/memory for a serverless instance (Prometheus).
  getInstanceMetrics(projectId: string, instanceId: string, metric: string, range: string): Observable<MetricsHistory> {
    return this.api.get<MetricsHistory>(`/projects/${projectId}/serverless/${instanceId}/metrics?metric=${metric}&range=${range}`);
  }

  // --- Serverless build history + live build logs ---
  listFunctionBuilds(projectId: string, instanceId: string): Observable<ServerlessBuild[]> {
    return this.api.get<ServerlessBuild[]>(`/projects/${projectId}/serverless/${instanceId}/builds`);
  }

  getFunctionBuildLogsStreamUrl(projectId: string, instanceId: string, buildId: string): string {
    return this.api.getStreamUrl(`/projects/${projectId}/serverless/${instanceId}/builds/${buildId}/logs/stream`);
  }
}

export interface ServerlessBuild {
  id: string;
  status: string;        // building | success | failed
  imageTag: string | null;
  durationSec: number | null;
  createdAt: string;
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
  targetType?: 'app' | 'database' | 'storage';
  target_type?: 'app' | 'database' | 'storage';
  targetId?: string | null;
  target_id?: string | null;
  targetName?: string | null;
  isBackup?: boolean;
  is_backup?: boolean;
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

export interface FunctionEnvResponse {
  id: string;
  functionId: string;
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

export interface MetricsHistory {
  timestamps: number[];
  values: number[];
  simulated?: boolean;
}

export interface ServerlessRoute {
  id: string;
  instanceId: string;
  method: string;
  routePath: string;
  code: string;
}

export interface ServerlessInstance {
  id: string;
  workspaceId: string;
  projectId: string;
  name: string;
  runtime: string;
  memoryLimitMb: number;
  status: 'draft' | 'building' | 'active' | 'failed';
  assignedDomain: string | null;
  externalPort: number | null;
  inheritProjectEnvs: boolean;
  routes: ServerlessRoute[];
  createdAt: string;
  updatedAt: string;
}
