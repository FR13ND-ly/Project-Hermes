import { NgModule } from '@angular/core';
import { RouterModule } from '@angular/router';
import { Projects } from './projects';
import { List } from './pages/list/list';

@NgModule({
  declarations: [],
  imports: [
    RouterModule.forChild([
      {
        path: '', component: Projects, children: [
          { path: '', component: List },
          { path: 'create', loadComponent: () => import('./pages/create/create').then(m => m.Create) },
          { 
            path: ':id', 
            loadComponent: () => import('./pages/details/details').then(m => m.Details), 
            children: [
              { path: '', loadComponent: () => import('./pages/details/pages/overview/overview').then(m => m.Overview) },
              { path: 'apps', loadComponent: () => import('./pages/details/pages/apps/apps').then(m => m.Apps) },
              { path: 'apps/create', loadComponent: () => import('./pages/details/pages/app-create/app-create').then(m => m.AppCreate) },
              { 
                path: 'apps/:appId', 
                loadComponent: () => import('./pages/details/pages/app-detail/app-detail').then(m => m.AppDetailComponent),
                children: [
                  { path: '', redirectTo: 'overview', pathMatch: 'full' },
                  { path: 'overview', loadComponent: () => import('./pages/details/pages/app-detail/pages/overview/overview').then(m => m.AppOverviewComponent) },
                  { path: 'telemetry', loadComponent: () => import('./pages/details/pages/app-detail/pages/telemetry/telemetry').then(m => m.AppTelemetryComponent) },
                  { path: 'builds', loadComponent: () => import('./pages/details/pages/app-detail/pages/builds/builds').then(m => m.AppBuildsComponent) },
                  { path: 'logs', loadComponent: () => import('./pages/details/pages/app-detail/pages/logs/logs').then(m => m.AppLogsComponent) },
                  { path: 'terminal', loadComponent: () => import('./pages/details/pages/app-detail/pages/terminal/terminal').then(m => m.AppTerminalComponent) },
                  { path: 'general', loadComponent: () => import('./pages/details/pages/app-detail/pages/general/general').then(m => m.AppGeneralComponent) },
                  { path: 'env', loadComponent: () => import('./pages/details/pages/app-detail/pages/env/env').then(m => m.AppEnvComponent) },
                  { path: 'advanced', loadComponent: () => import('./pages/details/pages/app-detail/pages/advanced/advanced').then(m => m.AppAdvancedComponent) },
                ]
              },
              { 
                path: 'auth-management', 
                loadComponent: () => import('./pages/details/pages/auth-management/auth-management').then(m => m.AuthManagement),
                children: [
                  { path: '', redirectTo: 'users', pathMatch: 'full' },
                  { path: 'users', loadComponent: () => import('./pages/details/pages/auth-management/pages/users/users').then(m => m.AuthUsersComponent) },
                  { path: 'roles', loadComponent: () => import('./pages/details/pages/auth-management/pages/roles/roles').then(m => m.AuthRolesComponent) },
                  { path: 'api-keys', loadComponent: () => import('./pages/details/pages/auth-management/pages/api-keys/api-keys').then(m => m.AuthApiKeysComponent) },
                  { path: 'integration', loadComponent: () => import('./pages/details/pages/auth-management/pages/integration/integration').then(m => m.AuthIntegrationComponent) },
                ]
              },
              { path: 'auth-management/create', loadComponent: () => import('./pages/details/pages/auth-management-create/auth-management-create').then(m => m.AuthManagementCreate) },
              { path: 'databases', loadComponent: () => import('./pages/details/pages/databases/databases').then(m => m.Databases) },
              { path: 'databases/create', loadComponent: () => import('./pages/details/pages/db-create/db-create').then(m => m.DbCreate) },
              { 
                path: 'databases/:dbId', 
                loadComponent: () => import('./pages/details/pages/db-detail/db-detail').then(m => m.DbDetailComponent),
                children: [
                  { path: '', redirectTo: 'overview', pathMatch: 'full' },
                  { path: 'overview', loadComponent: () => import('./pages/details/pages/db-detail/pages/overview/overview').then(m => m.DbOverviewComponent) },
                  { path: 'console', loadComponent: () => import('./pages/details/pages/db-detail/pages/console/console').then(m => m.DbConsoleComponent) },
                  { path: 'telemetry', loadComponent: () => import('./pages/details/pages/db-detail/pages/telemetry/telemetry').then(m => m.DbTelemetryComponent) },
                  { path: 'logs', loadComponent: () => import('./pages/details/pages/db-detail/pages/logs/logs').then(m => m.DbLogsComponent) },
                  { path: 'backups', loadComponent: () => import('./pages/details/pages/db-detail/pages/backups/backups').then(m => m.DbBackupsComponent) },
                  { path: 'settings', loadComponent: () => import('./pages/details/pages/db-detail/pages/settings/settings').then(m => m.DbSettingsComponent) },
                ]
              },
              { path: 'environments', loadComponent: () => import('./pages/details/pages/environments/environments').then(m => m.Environments) },
              { path: 'networking', loadComponent: () => import('./pages/details/pages/networking/networking').then(m => m.Networking) },
              { path: 'networking/create', loadComponent: () => import('./pages/details/pages/domain-create/domain-create').then(m => m.DomainCreate) },
              { path: 'settings', loadComponent: () => import('./pages/details/pages/settings/settings').then(m => m.Settings) },
              { 
                path: 'storages', 
                loadComponent: () => import('./pages/details/pages/storages/storages').then(m => m.Storages),
                children: [
                  { path: '', loadComponent: () => import('./pages/details/pages/storages/pages/list/list').then(m => m.StorageListComponent) },
                  { 
                    path: ':bucketId', 
                    loadComponent: () => import('./pages/details/pages/storages/pages/detail/detail').then(m => m.StorageDetailComponent),
                    children: [
                      { path: '', redirectTo: 'files', pathMatch: 'full' },
                      { path: 'files', loadComponent: () => import('./pages/details/pages/storages/pages/detail/pages/files/files').then(m => m.StorageFilesComponent) },
                      { path: 'logs', loadComponent: () => import('./pages/details/pages/storages/pages/detail/pages/logs/logs').then(m => m.StorageLogsComponent) },
                      { path: 'settings', loadComponent: () => import('./pages/details/pages/storages/pages/detail/pages/settings/settings').then(m => m.StorageSettingsComponent) },
                      { path: 'api', loadComponent: () => import('./pages/details/pages/storages/pages/detail/pages/api/api').then(m => m.StorageApiComponent) },
                    ]
                  }
                ]
              },
              { path: 'storages/create', loadComponent: () => import('./pages/details/pages/storage-create/storage-create').then(m => m.StorageCreate) },
              { 
                path: 'cron', 
                loadComponent: () => import('./pages/details/pages/cron/cron').then(m => m.CronComponent),
                children: [
                  { path: '', loadComponent: () => import('./pages/details/pages/cron/pages/list/list').then(m => m.CronListComponent) },
                  { 
                    path: ':cronId', 
                    loadComponent: () => import('./pages/details/pages/cron/pages/detail/detail').then(m => m.CronDetailComponent),
                    children: [
                      { path: '', redirectTo: 'details', pathMatch: 'full' },
                      { path: 'details', loadComponent: () => import('./pages/details/pages/cron/pages/detail/pages/details/details').then(m => m.CronDetailsComponent) },
                      { path: 'logs', loadComponent: () => import('./pages/details/pages/cron/pages/detail/pages/logs/logs').then(m => m.CronLogsComponent) },
                      { path: 'settings', loadComponent: () => import('./pages/details/pages/cron/pages/detail/pages/settings/settings').then(m => m.CronSettingsComponent) },
                    ]
                  }
                ]
              },
              { path: 'cron/create', loadComponent: () => import('./pages/details/pages/cron-create/cron-create').then(m => m.CronCreate) },
              { 
                path: 'serverless', 
                loadComponent: () => import('./pages/details/pages/serverless/serverless').then(m => m.ServerlessComponent),
                children: [
                  { path: '', loadComponent: () => import('./pages/details/pages/serverless/pages/list/list').then(m => m.ServerlessListComponent) },
                  { 
                    path: ':functionId', 
                    loadComponent: () => import('./pages/details/pages/serverless/pages/detail/detail').then(m => m.ServerlessDetailComponent),
                    children: [
                      { path: '', redirectTo: 'details', pathMatch: 'full' },
                      { path: 'details', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/details/details').then(m => m.ServerlessDetailsComponent) },
                      { path: 'routes', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/routes/routes').then(m => m.ServerlessRoutesComponent) },
                      { path: 'settings', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/settings/settings').then(m => m.ServerlessSettingsComponent) },
                      { path: 'env', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/env/env').then(m => m.ServerlessEnvComponent) },
                      { path: 'metrics', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/metrics/metrics').then(m => m.ServerlessMetricsComponent) },
                      { path: 'builds', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/builds/builds').then(m => m.ServerlessBuildsComponent) },
                      { path: 'logs', loadComponent: () => import('./pages/details/pages/serverless/pages/detail/pages/logs/logs').then(m => m.ServerlessLogsComponent) },
                    ]
                  }
                ]
              },
              { path: 'serverless/create', loadComponent: () => import('./pages/details/pages/serverless-create/serverless-create').then(m => m.ServerlessCreate) },
              { path: 'incidents', loadComponent: () => import('./pages/details/pages/incidents/incidents').then(m => m.Incidents) },
            ]
          },
        ]
      }
    ])
  ]
})
export class ProjectsModule { }
