# UI Routing Architecture

**Date:** 15-04-2026
**Framework:** Angular 17+

## 1. Routing Strategy
The application uses a strict Lazy Loading strategy. Access is strictly controlled via Route Guards. The platform operates as a closed enterprise system (no public registration or password recovery flows).

## 2. Route Map

### 2.1 Public Flow (GuestGuard)
* `/login` - Single entry point for authorized developers/admins. Redirects to `/dashboard` if already authenticated.

### 2.2 Global App Flow (AuthGuard)
* `/dashboard` - The main overview (Cluster health & Project list).
* `/projects/new` - Creation wizard for provisioning a new Hermes Project/Namespace.

### 2.3 Project Scoped Flow (AuthGuard + ProjectGuard)
Once inside a project, the UI utilizes a nested routing approach. The project sidebar remains static, while the content area swaps based on these dedicated child routes:

* `/projects/:id` (Auto-redirects to `/projects/:id/overview`)
    * `/overview` - Project specific metrics (CPU/RAM usage, Quick Actions).
    * `/deployments` - Dedicated page for Container status, environment variables, replicas.
    * `/databases` - Dedicated page for Managed Postgres/Mongo/Redis connection strings.
    * `/networking` - Dedicated page for Custom domains, SSL status, Nginx routing rules.
    * `/storage` - Dedicated page for Smart buckets, image processing rules, native file browser.
    * `/auth` - Dedicated page for Identity Provider settings, App Users, API Keys.
    * `/settings` - Danger zone (delete project, transfer ownership).

## 3. Directory Mapping (`/src/app/pages`)
The physical folder structure perfectly mirrors the route map to ensure easy navigation and strict separation of concerns for each resource:

```text
/pages
  /auth
    login-page.html
    login-page.ts
  /dashboard
    dashboard-page.html
    dashboard-page.ts
  /projects
    /create
      project-create-page.html
      project-create-page.ts
    /details
      project-layout.html        # Contains the Sidebar and <router-outlet>
      project-layout.ts      
      /overview
        project-overview-page.ts
      /deployments
        deployments-page.ts      # Independent Smart Page
      /databases
        databases-page.ts        # Independent Smart Page
      /networking
        networking-page.ts       # Independent Smart Page
      /storage
        storage-page.ts          # Independent Smart Page
      /auth
        auth-page.ts             # Independent Smart Page
      /settings
        settings-page.ts         # Independent Smart Page
```