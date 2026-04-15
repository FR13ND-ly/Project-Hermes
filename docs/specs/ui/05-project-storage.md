# UI Page: Project Storage

**Route:** `/projects/:id/storage`
**Guards:** `AuthGuard`, `ProjectGuard`

## 1. Visual Layout & Wireframe
Manages the Native Smart Storage pipeline and file browsing.

* **Top Section (Buckets & Settings):**
    * Dropdown to select active Bucket.
    * "Bucket Settings" button (opens a drawer to configure `compression_enabled`, `img_resize_max_width`, etc.).
    * Usage Bar: `used_bytes` vs `quota_bytes` (e.g., 250MB / 1GB).
* **Main Content (File Explorer):**
    * A classic file manager view (Grid or List toggle).
    * Shows `logical_path` breadcrumbs (e.g., `Home / images / avatars /`).
    * Displays image thumbnails (using the cached `blurhash` before the full image loads).
    * **Actions:** Upload File, Create Folder, Delete File, Copy Public URL.

## 2. Component Architecture

* **Smart Component:** * `storage-page.ts`: Manages the current virtual directory state and fetches files based on the `logical_path`.
* **Dumb Components (Core):**
    * `<app-file-browser>`: The main grid/list component. Emits `(onNavigate)` when a folder is clicked.
    * `<ui-file-card>`: Displays the file icon or image thumbnail, name, and size.
    * `<app-bucket-settings-drawer>`: A slide-out panel containing form toggles for the Rust media processing pipeline.

## 3. State Management (Signals)
```typescript
buckets = signal<StorageBucket[]>([]);
activeBucket = signal<StorageBucket | null>(null);

// File Explorer State
currentPath = signal<string>('/');
filesInCurrentPath = signal<StorageFile[]>([]);
isUploading = signal<boolean>(false);
```