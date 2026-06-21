# Hermes

A self-hosted Platform-as-a-Service (PaaS). Hermes deploys apps, managed
databases, serverless functions and object storage onto a Kubernetes cluster,
with multi-tenant workspaces, custom domains, cron jobs, a built-in BaaS auth
layer, Git-based imports and resource quotas.

## Stack

| Layer      | Tech                                                              |
|------------|------------------------------------------------------------------|
| Backend    | Rust · Axum · SQLx (PostgreSQL) · kube-rs (Kubernetes)           |
| Frontend   | Angular (standalone components + signals) · Tailwind CSS         |
| Infra      | Kubernetes · S3-compatible storage · OpenTelemetry · Prometheus  |

## Prerequisites

- Rust (stable) and `cargo`
- Node.js 20+ and npm
- PostgreSQL 14+
- A reachable Kubernetes cluster (in-cluster or via kubeconfig)
- `sqlx-cli` for offline query prep: `cargo install sqlx-cli --no-default-features --features rustls,postgres`

## Backend

```bash
cd backend
cp .env.example .env          # then edit the (REQUIRED) values
cargo run                     # runs migrations, seeds the root admin, starts the API
```

Mandatory secrets (`JWT_SECRET`, `HERMES_ENCRYPTION_KEY`) and `HERMES_ROOT_PASSWORD`
are validated at startup — the server refuses to boot if they are missing or
malformed. See [`backend/.env.example`](backend/.env.example) for the full list.

### Compiling without a live database

SQLx verifies queries against the database at compile time. A prepared offline
cache is committed in `backend/.sqlx`, so CI and fresh checkouts can build with:

```bash
SQLX_OFFLINE=true cargo build
```

After changing any SQL query, regenerate the cache against a live DB:

```bash
cargo sqlx prepare
```

## Frontend

```bash
cd frontend
npm install
npm start                     # dev server (proxies to the backend)
npm run build                 # production build
```

API/WS URLs are configured in `frontend/src/environments/`. In production they
default to the browser's current origin (reverse-proxied deploy).

## Documentation

In-depth design docs live in [`docs/`](docs/): architecture decision records
(`docs/adr`), DB/API/UI specs (`docs/specs`) and contributor guides
(`docs/guides`).
