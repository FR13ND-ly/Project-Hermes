# UI Page: Main Dashboard

**Route:** `/dashboard`
**Guards:** `AuthGuard`

## 1. Visual Layout & Wireframe
The Dashboard serves as the command center. It focuses on scannability and quick actions.

* **Top Bar:** * Left: Breadcrumbs (`Hermes / Dashboard`)
    * Right: Global 'Create Project' primary button, User Profile dropdown.
* **Hero Section (Cluster Health):** * A row of 4 horizontal statistic cards showing aggregate data across all projects (e.g., Total Projects, Active Deployments, Managed DBs, Storage Used).
* **Main Content Area:**
    * A search/filter input field.
    * A responsive CSS Grid (3 columns on desktop) displaying `Project Cards`.

## 2. Component Architecture (Smart vs. Dumb)
Following the Core + Pages architecture.

* **Smart Component (The Page):** * `pages/dashboard/dashboard-page.ts` - Injects `ProjectApi` and `MetricsApi`. Manages the signals and handles the 'Create' modal logic.
* **Dumb Components (Core):**
    * `<ui-stat-card>` (`core/components/stat-card.ts`) - Inputs: `icon`, `label`, `value`, `trend`.
    * `<app-project-card>` (`core/components/project-card.ts`) - Inputs: `project: Project`. Outputs: `(onClick)`. Shows project name, K8s status badge, and quick links.
    * `<ui-empty-state>` (`core/components/empty-state.ts`) - Inputs: `title`, `message`, `imageSrc`.

## 3. State Management (Signals)
The `dashboard-page.ts` will track the following local state:

* `isLoading`: `WritableSignal<boolean>` (Defaults to true, controls Skeleton loaders)
* `isError`: `WritableSignal<string | null>` (Controls error banners if K3s is unreachable)
* `projects`: `Signal<Project[]>` (Populated from API, controls the grid)
* `filteredProjects`: `computed(() => ...)` (Reacts to the search input signal)
* `clusterMetrics`: `Signal<Metrics>` (Aggregated data for the Top bar)

## 4. API Integration
* **On Init (Concurrent Fetching):** * `GET /api/v1/projects` (Fetches the list of namespaces owned by the user)
    * `GET /api/v1/metrics/overview` (Fetches global DB/Deployment counts)

## 5. Edge Cases & UX
* **Empty State:** If `projects().length === 0`, hide the grid and search bar. Display `<ui-empty-state>` with an illustration and a prominent "Launch Your First Project" button.
* **Loading State:** Do not show a blank screen. Use skeleton loaders that mimic the shape of the `Project Cards` while `isLoading` is true.
* **Data Stale:** Implement background polling every 30 seconds to update K8s pod statuses without refreshing the whole page.