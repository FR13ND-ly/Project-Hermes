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
              { path: 'databases', loadComponent: () => import('./pages/details/pages/databases/databases').then(m => m.Databases) },
              { path: 'deployements', loadComponent: () => import('./pages/details/pages/deployments/deployments').then(m => m.Deployments) },
              { path: 'environments', loadComponent: () => import('./pages/details/pages/environments/environments').then(m => m.Environments) },
              { path: 'networking', loadComponent: () => import('./pages/details/pages/networking/networking').then(m => m.Networking) },
              { path: 'storages', loadComponent: () => import('./pages/details/pages/storages/storages').then(m => m.Storages) },
            ]
          },
        ]
      }
    ])
  ]
})
export class ProjectsModule { }
