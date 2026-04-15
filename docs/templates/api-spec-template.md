# API Spec: [Endpoint Group Name]

## [Endpoint Name / Action]
**Description:** Briefly describe what this endpoint does.

* **URL:** `/api/v1/[path]`
* **Method:** `GET | POST | PUT | DELETE`
* **Auth Required:** `Yes (Bearer Token) | No`

### Request
**Headers:**
```json
{
  "Authorization": "Bearer <JWT>"
}
```

**Body:**
```json
{
  "key": "value"
}
```

### Response
**Success Response (200 OK / 201 Created):**
```json
{
  "data": {
    "id": "uuid",
    "status": "success"
  }
}
```

**Error Responses:**
* **400 Bad Request:** Missing required fields.
* **401 Unauthorized:** Invalid or missing JWT.
* **403 Forbidden:** User does not have permission for this project.