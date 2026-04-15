# Backend Architecture (Rust & Axum)

**Date:** 15-04-2026

## 1. Directory Structure
We use a Module-Driven Architecture. Cross-cutting utilities live in `/core`, background tasks in `/workers`, and business features live in `/modules` (matching our Database Specs).

```text
/src
  main.rs                 # Entry point: Axum setup, DB pool, Worker bootstrap
  
  /core                   # Cross-cutting concerns & integrations
    error.rs              # Global AppError (implements IntoResponse)
    config.rs             # Environment variable validation
    k8s_client.rs         # Wrapper around kube-rs
    nginx_manager.rs      # Templates and `nginx -s reload` execution
    jwt.rs                # Axum Extractors for Auth
    
  /modules                # Business Domains
    /projects
      models.rs           # DB Structs (SQLx FromRow)
      repo.rs             # Database queries (CRUD)
      service.rs          # Business logic (e.g., creating K8s Namespace)
      handler.rs          # Axum HTTP routes (extractors & JSON responses)
    /networking           # Domains & Gateway logic
      models.rs
      ...
    /storage              # Smart media pipeline & Image processing
      models.rs
      ...
    /auth                 # Identity Provider & RBAC
      models.rs
      ...
      
  /workers                # Asynchronous background tasks (tokio::spawn)
    certbot_worker.rs     # Polls 'pending' SSL domains and runs Let's Encrypt
    image_processor.rs    # Handles heavy WebP/Blurhash conversions asynchronously
```

## 2. The Golden Rules of the Layers
To maintain a clean codebase, data must flow strictly downwards through these layers within any module:

1. **Handler (`handler.rs`):** * **Job:** Speaks HTTP. Extracts JSON payloads, path variables, and JWTs.
   * **Constraint:** MUST NEVER write SQL queries or talk to K8s directly. It only calls functions from `service.rs`.
2. **Service (`service.rs`):** * **Job:** The Brain. Contains the actual business logic (e.g., "Check if user has quota, if yes, ask Repo to save DB record, then ask Core to generate K8s manifest").
   * **Constraint:** It handles `Result` types and returns our custom `AppError` if something fails.
3. **Repository (`repo.rs`):** * **Job:** Speaks SQL. Executes `sqlx::query!` and returns `models.rs` structs.
   * **Constraint:** Contains zero business logic. It just reads/writes to PostgreSQL.

## 3. Error Handling Pattern
We never use `.unwrap()` in HTTP handlers. All layers return `Result<T, AppError>`. The `AppError` enum automatically translates backend errors (like a K8s connection timeout or SQL duplicate key) into safe HTTP responses (like `500 Internal Server Error` or `409 Conflict`) to prevent leaking internal infrastructure details to the client.