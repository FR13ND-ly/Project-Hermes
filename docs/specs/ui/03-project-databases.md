# UI Page: Project Databases

**Route:** `/projects/:id/databases`
**Guards:** `AuthGuard`, `ProjectGuard`

## 1. Visual Layout & Wireframe
Displays the dedicated database instances running within the project's namespace.

* **Top Actions:** "Provision Database" button.
* **Main Content (Grid of Cards):**
    * Each DB gets a prominent card showing:
        * Engine Logo (Postgres, Redis, MongoDB).
        * Internal DB Name.
        * Status Badge.
        * Resource Usage Bar (e.g., RAM: 256MB / 512MB, Storage: 1GB / 5GB).
    * **Card Actions:**
        * "Connection Details" (Primary button)
        * "Open Web UI" (e.g., pgAdmin or Mongo Express if implemented)
        * "Resize Resources"

## 2. Component Architecture

* **Smart Component:** * `databases-page.ts`: Fetches the DB instances and their real-time K8s resource usage.
* **Dumb Components (Core):**
    * `<app-db-card>`: Displays the engine, status, and resource progress bars.
    * `<ui-progress-bar>`: Reusable component indicating quota usage. Warns visually (turns red) if usage is > 85%.
    * `<app-connection-modal>`: A secure modal that displays the internal DNS (for app use) and the external connection string (with a copy-to-clipboard button and eye-toggle to reveal the generated password).

## 3. State Management (Signals)
```typescript
databases = signal<DatabaseInstance[]>([]);

// Controls which DB's connection string is currently being viewed
viewingConnectionFor = signal<DatabaseInstance | null>(null);
```

## 4. Security Note (Connection Strings)
The Angular UI must NEVER store the `db_password` in local state permanently. Passwords should be fetched just-in-time when the `<app-connection-modal>` is opened and purged from the Signal state the moment the modal is closed.