# Database Spec: Users

**Date:** [15-04-2026]
**Status:** Proposed

## 1. Domain Overview
This module handles the core identity and access management for the Hermes platform. It stores the developers and system administrators who log into the Hermes dashboard to manage their cloud infrastructure.

## 2. Table Definitions

### 2.1 Table: `users`
**Description:** Stores the Hermes account holders.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `username` | VARCHAR(50) | Unique, Not Null | Developer's handle |
| `email` | VARCHAR(255) | Unique, Not Null | Contact and primary login method |
| `password_hash` | TEXT | Not Null | Bcrypt / Argon2 hashed password |
| `is_superadmin`| BOOLEAN | Default false | If true, grants access to the global /admin dashboard |
| `status` | VARCHAR(20) | Default 'active' | `active`, `suspended`, `deleted` |
| `created_at` | TIMESTAMPTZ | Default now() | Account creation timestamp |

## 3. Relationships & Constraints
* **Root Entity:** The `users` table is a root entity. It does not depend on any other table.
* **Data Deletion:** If a user is deleted, all their associated `projects` and infrastructure must either be transferred to another user or completely wiped (Cascade Delete) to free up server resources.