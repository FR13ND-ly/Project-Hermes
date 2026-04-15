# API Spec: Projects Management

## List All Projects
**Description:** Fetches all projects (Namespaces) owned by the authenticated user. Used to populate the main Dashboard grid.

* **URL:** `/api/v1/projects`
* **Method:** `GET`
* **Auth Required:** `Yes (Bearer Token)`

### Request
**Headers:**
```json
{
  "Authorization": "Bearer <JWT>"
}
```

**Body:**
None

### Response
**Success Response (200 OK):**
```json
{
  "data": [
    {
      "id": "a1b2c3d4-e5f6...",
      "name": "Production E-commerce",
      "slug": "prod-ecommerce",
      "k8s_namespace": "prj-prod-ecommerce-a1b2",
      "status": "active",
      "createdAt": "2026-04-15T10:00:00Z"
    }
  ]
}
```

**Error Responses:**
* **401 Unauthorized:** Invalid, expired, or missing JWT.

---

## Create New Project
**Description:** Triggers the creation of a new project. The Rust backend will save the record to Postgres and asynchronously tell Kubernetes to create a new isolated Namespace.

* **URL:** `/api/v1/projects`
* **Method:** `POST`
* **Auth Required:** `Yes (Bearer Token)`

### Request
**Headers:**
```json
{
  "Authorization": "Bearer <JWT>",
  "Content-Type": "application/json"
}
```

**Body:**
```json
{
  "name": "My New Startup"
}
```
*(Note: The Rust backend will automatically generate the `slug` and `k8s_namespace` from the name).*

### Response
**Success Response (201 Created):**
```json
{
  "data": {
    "id": "f7g8h9i0...",
    "name": "My New Startup",
    "slug": "my-new-startup",
    "k8s_namespace": "prj-my-new-startup-f7g8",
    "status": "provisioning",
    "createdAt": "2026-04-15T12:00:00Z"
  }
}
```

**Error Responses:**
* **400 Bad Request:** Missing or invalid `name` field.
* **401 Unauthorized:** Invalid or missing JWT.
* **409 Conflict:** A project with a very similar name already exists (slug collision).

---

## Get Project Metrics (Overview)
**Description:** Fetches real-time cluster metrics (CPU/RAM usage, active pods count) for a specific project's namespace.

* **URL:** `/api/v1/projects/:id/metrics`
* **Method:** `GET`
* **Auth Required:** `Yes (Bearer Token)`

### Request
**Headers:**
```json
{
  "Authorization": "Bearer <JWT>"
}
```

**Body:**
None

### Response
**Success Response (200 OK):**
```json
{
  "data": {
    "activeDeployments": 3,
    "activeDatabases": 1,
    "cpuUsageCores": 0.45,
    "ramUsageMb": 1024,
    "storageUsedMb": 250
  }
}
```

**Error Responses:**
* **401 Unauthorized:** Invalid or missing JWT.
* **403 Forbidden:** User does not own the project with this `:id`.
* **404 Not Found:** Project `:id` does not exist.
* **503 Service Unavailable:** The Rust backend could not communicate with the Kubernetes API to fetch metrics.