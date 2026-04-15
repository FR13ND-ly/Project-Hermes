# Database Spec: Managed Databases

**Date:** [15-04-2026]
**Status:** Proposed

## 1. Domain Overview
This module manages the lifecycle of dedicated database instances provisioned for each project. Unlike shared hosting, Hermes spins up a full database engine (container) for every entry in this schema, ensuring total isolation of data and performance.

## 2. Table Definitions

### 2.1 Table: `db_instances`
**Description:** Tracks the configuration, credentials, and health of managed database engines.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Not Null | The owner project/namespace |
| `engine` | VARCHAR(20) | Not Null | `postgres`, `mongodb`, `redis` |
| `version` | VARCHAR(10) | Not Null | e.g., '15.4', '6.0' |
| `db_name` | VARCHAR(63) | Not Null | Internal DB name |
| `db_user` | VARCHAR(63) | Not Null | Admin username |
| `db_password`| TEXT | Not Null (Encrypted) | Encrypted password for the instance |
| `status` | VARCHAR(20) | Default 'provisioning' | `provisioning`, `running`, `error`, `stopped` |
| `storage_gb` | INT | Default 5 | Allocated disk space in Gigabytes |
| `cpu_limit` | INT | Default 250 | Milli-cores (e.g., 500 = 0.5 CPU) |
| `memory_limit`| INT | Default 512 | Memory in MiB |
| `created_at` | TIMESTAMPTZ | Default now() | Creation timestamp |

## 3. Relationships & Constraints
* **Resource Tracking:** Each `db_instance` is tied to exactly one `project_id`.
* **Connection Strings:** The Rust backend should not store the full connection string, but construct it dynamically based on the internal K8s DNS: `[db_name].[namespace].svc.cluster.local`.
* **Encryption:** The `db_password` must be encrypted at rest using a Master Key stored in the Hermes environment variables (K8s Secrets).
* **Data Retention:** When a project is deleted, the associated `PersistentVolumes` should be retained for 7 days (via a 'Released' state) before permanent deletion to prevent accidental data loss.