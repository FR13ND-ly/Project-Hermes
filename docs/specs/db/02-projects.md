# Database Spec: Projects

**Date:** [15-04-2026]
**Status:** Proposed

## 1. Domain Overview
The Projects module is the structural heart of Hermes. A Project acts as a logical container for all cloud resources (Deployments, Databases, Storage). Every project maps 1-to-1 to a **Kubernetes Namespace**, ensuring multi-tenant isolation, security policies, and resource quotas at the cluster level.

## 2. Table Definitions

### 2.1 Table: `projects`
**Description:** Stores metadata and identity for each user's workspace.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `owner_id` | UUID | FK -> users.id, Not Null | The developer who owns this project |
| `name` | VARCHAR(63) | Unique, Not Null | K8s-compliant name (regex: `^[a-z0-9]([-a-z0-9]*[a-z0-9])?$`) |
| `status` | VARCHAR(20) | Default 'provisioning' | `active`, `suspended`, `provisioning`, `error` |
| `description` | TEXT | | Optional project description |
| `created_at` | TIMESTAMPTZ | Default now() | Timestamp of creation |

### 2.2 Table: `project_quotas`
**Description:** Defines hard resource limits for a specific project, used to generate K8s `ResourceQuota` objects.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Unique | The linked project |
| `cpu_limit` | INT | Default 1000 | Total CPU cores in milli-cores (1000 = 1 Core) |
| `memory_limit` | INT | Default 1024 | Total memory limit in MiB |
| `storage_limit`| INT | Default 10 | Total persistent storage in GiB |

## 3. Relationships & Constraints
* **Multi-Tenancy:** The `owner_id` ensures strict ownership. A user can only see or modify projects they own.
* **Kubernetes Mapping:** The `name` column is used as the `Namespace` ID. The Rust backend must ensure this name is unique across the entire K3s cluster.
* **Suspension Logic:** When a project status is updated to `suspended`, the Hermes Orchestrator must scale all associated Pods to zero while keeping the data intact.
* **Hard Deletion:** Deleting a project triggers a `CASCADE` delete for quotas, but infrastructure destruction (K8s Namespace removal) must be confirmed by the backend before the DB record is purged.