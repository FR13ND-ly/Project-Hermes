import { Routes } from '@angular/router';
import { authGuard, superAdminGuard, guestGuard } from './core/guards/auth.guard';

export const routes: Routes = [
    {
        path: '',
        pathMatch: 'full',
        redirectTo: 'dashboard'
    },
    {
        path: 'auth',
        canActivate: [guestGuard],
        loadComponent: () => import('./pages/auth/auth').then(m => m.Auth)
    },
    {
        path: 'dashboard',
        canActivate: [authGuard],
        loadComponent: () => import('./pages/dashboard/dashboard').then(m => m.Dashboard)
    },
    {
        path: 'projects',
        canActivate: [authGuard],
        loadChildren: () => import('./pages/projects/projects-routes').then(m => m.ProjectsModule)
    },
    {
        path: 'workspace/settings',
        canActivate: [authGuard],
        loadComponent: () => import('./pages/workspace/settings/settings').then(m => m.WorkspaceSettings)
    },
    {
        path: 'admin/users',
        canActivate: [superAdminGuard],
        loadComponent: () => import('./pages/admin/users/users').then(m => m.AdminUsers)
    },
    {
        path: 'admin/workspaces',
        canActivate: [superAdminGuard],
        loadComponent: () => import('./pages/admin/workspaces/workspaces').then(m => m.AdminWorkspaces)
    },
    {
        path: 'admin/logs',
        canActivate: [superAdminGuard],
        loadComponent: () => import('./pages/admin/logs/logs').then(m => m.AdminLogs)
    },
    {
        path: '**',
        loadComponent: () => import('./pages/not-found/not-found').then(m => m.NotFound)
    }
];
