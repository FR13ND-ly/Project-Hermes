# UI Page: Project Authentication & Users

**Route:** `/projects/:id/auth`
**Guards:** `AuthGuard`, `ProjectGuard`

## 1. Visual Layout & Wireframe
Manages the BaaS Identity Provider (End-users, Roles, and API Keys).

* **Navigation:** A horizontal tab menu to switch between: "Users", "Roles", "API Keys".
* **Tab 1: Users:**
    * Data table showing `email`, `last_login`, and assigned `roles`.
    * Actions: Reset Password, Suspend User, Assign Role.
* **Tab 2: Roles:**
    * List of custom roles (e.g., 'Admin', 'Editor').
    * A JSON editor to define raw string permissions (e.g., `["posts:write", "posts:delete"]`).
* **Tab 3: API Keys:**
    * List of server-to-server keys (shows only the `key_prefix` and `name`).
    * "Generate Key" button.

## 2. Component Architecture

* **Smart Component:** * `auth-page.ts`: Manages the active tab state and fetches data corresponding to the selected tab.
* **Dumb Components (Core):**
    * `<ui-tabs>`: Reusable tab navigation.
    * `<app-user-table>`: Specific table for end-users.
    * `<app-api-key-modal>`: A critical security modal. When a key is generated, it displays the full key **exactly once** with a warning: "Copy this now. You will not be able to see it again."

## 3. State Management (Signals)
```typescript
activeTab = signal<'users' | 'roles' | 'keys'>('users');

appUsers = signal<AppUser[]>([]);
appRoles = signal<AppRole[]>([]);
apiKeys = signal<ApiKey[]>([]);
```