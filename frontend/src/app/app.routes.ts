import { Routes } from '@angular/router';

export const routes: Routes = [
    {
        path: 'auth',
        loadComponent: () => import('./pages/auth/auth').then(m => m.Auth)
    },
    {
        path: 'dashboard',
        loadComponent: () => import('./pages/dashboard/dashboard').then(m => m.Dashboard)
    },
    {
        path: 'projects',
        loadChildren: () => import('./pages/projects/projects-routes').then(m => m.ProjectsModule)
    },
    {
        path: 'workspace/environments',
        loadComponent: () => import('./pages/workspace/environments/environments').then(m => m.WorkspaceEnvironments)
    },
    {
        path: 'workspace/settings',
        loadComponent: () => import('./pages/workspace/settings/settings').then(m => m.WorkspaceSettings)
    },
    {
        path: 'admin/users',
        loadComponent: () => import('./pages/admin/users/users').then(m => m.AdminUsers)
    },
    {
        path: '**',
        loadComponent: () => import('./pages/not-found/not-found').then(m => m.NotFound)
    }
];
