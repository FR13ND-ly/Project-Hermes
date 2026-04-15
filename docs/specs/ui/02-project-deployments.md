# UI Page: Project Deployments

**Route:** `/projects/:id/deployments`
**Guards:** `AuthGuard`, `ProjectGuard`

## 1. Visual Layout & Wireframe
This page lists all active computing workloads for the selected project.

* **Top Actions:** "New Deployment" button (opens a wizard to connect a Git repo).
* **Main Content (Data Table / Grid):**
    * A list of deployments. Each row/card displays:
        * Name & Type (e.g., `api-server` - Backend)
        * Git Branch (`main`)
        * Status Badge (`Running`, `Building`, `Failed`)
        * Pod Replicas (e.g., `1/1`)
    * **Row Actions (Three-dot menu):**
        * View Logs
        * Environment Variables
        * Restart Pods
        * Delete Deployment

## 2. Component Architecture

* **Smart Component:** * `deployments-page.ts`: Fetches the list of deployments from the Rust API. Manages the state for the side-drawers (Logs and Env Vars).
* **Dumb Components (Core):**
    * `<ui-data-table>`: A reusable generic table component that takes an array of columns and data.
    * `<ui-status-badge>`: Standardized colored badges (Green for running, Yellow for pending, Red for failed).
    * `<app-env-var-drawer>`: A slide-out panel that receives a `deploymentId`, fetches its variables, and allows editing the key-value pairs (with masking for `is_secret`).

## 3. State Management (Signals)
```typescript
isLoading = signal<boolean>(true);
deployments = signal<Deployment[]>([]);

// UI State for slide-out panels
selectedDeploymentForLogs = signal<string | null>(null);
selectedDeploymentForEnv = signal<string | null>(null);
```

## 4. API Integration
* `GET /api/v1/projects/:id/deployments`
* `POST /api/v1/deployments/:dep_id/restart` (Triggers a K8s rollout restart)