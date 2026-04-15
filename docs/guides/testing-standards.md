# Testing & TDD Standards (Clear Protocol)

We follow Test-Driven Development (TDD). This means tests are written **before** the actual implementation logic.

## 1. The TDD Cycle (Red-Green-Refactor)
1. **Red:** Write a test for a specific requirement (e.g., "User cannot create project with empty name"). Run it and watch it fail.
2. **Green:** Write the minimum amount of code required to make that specific test pass.
3. **Refactor:** Clean up the code while ensuring the test remains green.

## 2. Backend Testing (Rust/Axum)
We focus on **Integration Tests** because they provide the highest confidence when dealing with APIs and DBs.

* **Location:** All integration tests live in the `backend-rust/tests/` directory.
* **Database Strategy:** Use a dedicated test database (Postgres). Each test must run in a transaction that is rolled back at the end, or use a unique ID to ensure isolation.
* **Axum Testing:** Use `tower::ServiceExt` to simulate real HTTP requests without spinning up a full server.

**Checklist for a "Done" Endpoint:**
* [ ] Test 200/201 Success path.
* [ ] Test 400 Validation (invalid JSON, missing fields).
* [ ] Test 401/403 (Unauthorized/Forbidden).

## 3. Frontend Testing (Angular)
Focus on **Component Isolation** and **State Logic**.

* **Logic Testing:** Test Signals and Computed values in services.
* **Component Testing:** Use `TestBed` to ensure the component renders correctly in "Loading", "Empty", and "Error" states based on mocked service responses.
* **Mocking:** Use an `HttpInterceptor` or mock services to avoid making real API calls during component tests.

## 4. Requirement for "Done"
A feature is considered "Done" only if:
1. All written tests pass.
2. Code coverage for the new logic is > 80%.
3. The API spec in `docs/specs/api/` matches the implementation perfectly.