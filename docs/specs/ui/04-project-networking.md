# UI Page: Project Networking

**Route:** `/projects/:id/networking`
**Guards:** `AuthGuard`, `ProjectGuard`

## 1. Visual Layout & Wireframe
This page manages external access, Custom Domains, and SSL certificates.

* **Top Actions:** "Add Domain" button.
* **Main Content (List View):**
    * A list of configured domains. Each item displays:
        * Domain Name (`api.myapp.com`)
        * Routing Target (e.g., `-> api-backend (Port 80)`)
        * SSL Status Badge (`Active` / `Pending` / `Failed`)
        * Type Badge (`Proxy`, `Redirect`, `Custom`)
    * **Row Actions:**
        * View Nginx Config
        * Edit Settings (Max Body Size, Target Port)
        * Delete Domain

## 2. Component Architecture

* **Smart Component:** * `networking-page.ts`: Fetches domains and handles the Let's Encrypt polling (if a domain is 'pending', poll every 10s until 'active').
* **Dumb Components (Core):**
    * `<ui-domain-list-item>`: A stylized row representing a domain's routing rule.
    * `<ui-ssl-badge>`: A specific badge component with a lock icon (Green for active, spinning loader for pending).
    * `<app-nginx-viewer-modal>`: A modal that takes the `last_applied_config` string and renders it using a read-only code editor component (e.g., Monaco Editor or simple `<pre><code>` with syntax highlighting).

## 3. State Management (Signals)
```typescript
isLoading = signal<boolean>(true);
domains = signal<Domain[]>([]);

// UI State
isAddDomainModalOpen = signal<boolean>(false);
selectedConfigToView = signal<string | null>(null);
```