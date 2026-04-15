# Database Spec: Authorizations (BaaS Identity Provider)

**Date:** 15-04-2026
**Status:** Proposed

## 1. Domain Overview
This module acts as a multi-tenant Identity Provider (IdP). It manages the end-users of the applications hosted on Hermes, including their credentials, roles, and API permissions. This allows developers to offload authentication and Role-Based Access Control (RBAC) to the Hermes platform.

## 2. Table Definitions

### 2.1 Table: `app_users`
**Description:** The end-users belonging to a specific project.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Not Null | The project this user belongs to |
| `email` | VARCHAR(255) | Not Null | User login email |
| `password_hash` | TEXT | Not Null | Hashed password (Argon2) |
| `metadata` | JSONB | Default '{}' | Custom user profile data |
| `last_login` | TIMESTAMPTZ | | Audit trail |
| `created_at` | TIMESTAMPTZ | Default now() | Account creation date |

### 2.2 Table: `app_roles`
**Description:** Customizable roles defined by the developer for their project.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Not Null | Scope of the role |
| `name` | VARCHAR(50) | Not Null | Role name (e.g., 'admin', 'editor') |
| `permissions` | JSONB | Default '[]' | List of string constants (e.g. `["posts:write"]`) |

### 2.3 Table: `app_user_roles`
**Description:** Mapping table between users and roles (Many-to-Many).

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `user_id` | UUID | FK -> app_users.id, PK | The user |
| `role_id` | UUID | FK -> app_roles.id, PK | The assigned role |

### 2.4 Table: `app_api_keys`
**Description:** Keys for server-to-server communication or external API access.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Not Null | Owner project |
| `key_prefix` | VARCHAR(8) | Not Null | Public part of the key (e.g. `hr_live_`) |
| `key_hash` | TEXT | Unique, Not Null | Hashed key for verification |
| `name` | VARCHAR(100)| | Label for the developer |
| `scopes` | JSONB | Default '["*"]' | Restricted permissions for this key |
| `expires_at` | TIMESTAMPTZ | | Optional expiration date |

## 3. Relationships & Constraints
* **Multi-Tenant Isolation:** All tables are strictly scoped by `project_id`. An `app_user` from Project A cannot log into Project B unless explicitly registered there.
* **Unique Constraint:** A unique index on `(project_id, email)` in `app_users` ensures unique accounts per application.
* **JWT Integration:** When a user logs in, the Rust backend queries `app_user_roles` and `app_roles` to inject the aggregate permissions into the JWT `claims` section.
* **Security:** API Keys are never stored in plain text. Only the hash is stored, making it impossible to retrieve a lost key.