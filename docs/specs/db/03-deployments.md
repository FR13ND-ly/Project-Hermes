# Database Spec: Deployments & Environment Variables

**Date:** 15-04-2026
**Status:** Proposed

## 1. Domain Overview
The Deployments module tracks the actual computing workloads running inside a Project's Kubernetes Namespace. It links the source code repository (Git) to the live infrastructure and manages the runtime configuration (Environment Variables) required by the applications.

## 2. Table Definitions

### 2.1 Table: `deployments`
**Description:** Stores the configuration and state of a single service/application (e.g., a Frontend Angular app or a Backend Rust API).

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Not Null | The parent project (Namespace) |
| `name` | VARCHAR(63) | Not Null | Service name (e.g., 'api', 'web') |
| `type` | VARCHAR(20) | Not Null | `frontend` (Static Ingress), `backend` (API/Worker) |
| `git_repo_url` | TEXT | Not Null | Source code URL for the build runner |
| `git_branch` | VARCHAR(50)| Default 'main' | Branch to deploy from |
| `status` | VARCHAR(20) | Default 'pending' | `pending`, `building`, `running`, `failed`, `stopped` |
| `replicas` | INT | Default 1 | Target number of Kubernetes Pods |
| `created_at` | TIMESTAMPTZ | Default now() | Creation timestamp |
| `updated_at` | TIMESTAMPTZ | Default now() | Last state change |

### 2.2 Table: `environment_variables`
**Description:** Stores the ENV vars injected into the Kubernetes Pods.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `deployment_id`| UUID | FK -> deployments.id, Not Null | The associated deployment |
| `key` | VARCHAR(255) | Not Null | Variable name (e.g., `DATABASE_URL`) |
| `value` | TEXT | Not Null | Variable value |
| `is_secret` | BOOLEAN | Default false | If true, masked in UI and treated as a K8s Secret |

## 3. Relationships & Constraints
* **Compound Unique Index:** We must enforce a unique index on `(project_id, name)` in the `deployments` table. This guarantees no two deployments in the same project share a name, which is critical because the `name` is used to generate the Kubernetes `Service` and Nginx routing paths.
* **Secret Management:** If `is_secret` is true, the Rust backend must ideally encrypt the `value` before saving it to PostgreSQL, and decrypt it only in memory when generating the Kubernetes `Secret` manifest.
* **Cascade Deletion:** `ON DELETE CASCADE` from `projects` -> `deployments` -> `environment_variables`. If a project goes down, everything inside it is wiped automatically from the DB.