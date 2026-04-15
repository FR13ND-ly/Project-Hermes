# UI Spec: [Page Name]

**Route:** `/[route-path]`
**Guards:** [e.g., AuthGuard, SuperAdminGuard]

## 1. User Flow
* **Trigger:** How does the user get to this page? (e.g., Clicks "Databases" from the Project Sidebar).
* **Primary Actions:** What are the main things the user can do here? (e.g., Create a new database, delete a database).
* **Success Result:** What happens when the action succeeds? (e.g., Redirects to overview, shows success toast).

## 2. UI States
* **Loading State:** How does the page look while fetching data? (e.g., Skeleton loaders for the table).
* **Empty State:** What if there is no data? (e.g., Show a "No databases found" illustration and a large "Create One" button).
* **Error State:** How do we handle API failures? (e.g., Show an inline error banner: "Failed to load databases").

## 3. API Contract Requirements
* **Fetch Data:** `GET /api/v1/...`
* **Mutate Data:** `POST /api/v1/...` (Include a brief JSON structure expected by the frontend).
* *(Note: Link to the full API spec from `docs/specs/api/` if it's complex).*

## 4. Backend & Infrastructure Implications
* **Database:** What tables are being read/modified?
* **Kubernetes:** Does this action trigger a K8s resource creation? (e.g., Creates a `StatefulSet` and a `Service`).