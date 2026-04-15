# Database Spec: [Domain / Model Group Name]

**Date:** [DD-MM-YYYY]
**Status:** [Proposed | Accepted | Deprecated]

## 1. Domain Overview
Briefly describe what this group of tables is responsible for. (e.g., "This module handles the core authentication for platform developers and the multi-tenant app users.")

## 2. Table Definitions

### 2.1 Table: `[table_name]`
**Description:** What does this table store?

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `[col_name]` | [TYPE] | [e.g., Unique, Not Null] | [Description] |

### 2.2 Table: `[related_table_name]`
**Description:** What does this table store?

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK | Unique identifier |
| `parent_id` | UUID | FK -> [table_name].id | Parent reference |

## 3. Relationships & Constraints
* **1-to-Many:** Explain how these tables link together.
* **Data Integrity:** Mention any cascading deletes or specific rules (e.g., "If a project is deleted, its deployments are ON DELETE CASCADE").
* **Indexes:** List any columns that need indexing for performance (e.g., "Compound index on `(project_id, email)` in `app_users` table").