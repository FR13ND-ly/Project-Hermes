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
              { path: 'apps/:appId', loadComponent: () => import('./pages/details/pages/app-detail/app-detail').then(m => m.AppDetailComponent) },
              { path: 'auth-management', loadComponent: () => import('./pages/details/pages/auth-management/auth-management').then(m => m.AuthManagement) },
              { path: 'auth-management/create', loadComponent: () => import('./pages/details/pages/auth-management-create/auth-management-create').then(m => m.AuthManagementCreate) },
              { path: 'databases', loadComponent: () => import('./pages/details/pages/databases/databases').then(m => m.Databases) },
              { path: 'databases/create', loadComponent: () => import('./pages/details/pages/db-create/db-create').then(m => m.DbCreate) },
              { path: 'databases/:dbId', loadComponent: () => import('./pages/details/pages/db-detail/db-detail').then(m => m.DbDetailComponent) },
              { path: 'environments', loadComponent: () => import('./pages/details/pages/environments/environments').then(m => m.Environments) },
              { path: 'networking', loadComponent: () => import('./pages/details/pages/networking/networking').then(m => m.Networking) },
              { path: 'networking/create', loadComponent: () => import('./pages/details/pages/domain-create/domain-create').then(m => m.DomainCreate) },
              { path: 'settings', loadComponent: () => import('./pages/details/pages/settings/settings').then(m => m.Settings) },
              { path: 'storages', loadComponent: () => import('./pages/details/pages/storages/storages').then(m => m.Storages) },
              { path: 'storages/create', loadComponent: () => import('./pages/details/pages/storage-create/storage-create').then(m => m.StorageCreate) },
              { path: 'cron', loadComponent: () => import('./pages/details/pages/cron/cron').then(m => m.CronComponent) },
              { path: 'cron/create', loadComponent: () => import('./pages/details/pages/cron-create/cron-create').then(m => m.CronCreate) },
              { path: 'serverless', loadComponent: () => import('./pages/details/pages/serverless/serverless').then(m => m.ServerlessComponent) },
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
