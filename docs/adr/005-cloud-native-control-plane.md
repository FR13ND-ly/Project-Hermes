# ADR 005: Cloud-Native Control Plane & In-Cluster Edge

**Date:** 17-06-2026
**Status:** Accepted — **supersedes [ADR 003](003-edge-routing.md)**

## Context
ADR-003 put a **host-level Nginx** (managed by the Rust backend writing
`/etc/nginx/sites-*` + `nginx -s reload`) in front of everything. In practice this
produced three overlapping edge layers (host Nginx **and** k8s `Ingress` **and**
LoadBalancer services), only one of which can own `:80/:443`, plus several
operational problems that all trace back to **one root cause: the control plane
runs on the host, not in the cluster**:

* Automatic TLS for custom domains was never finished (the Nginx templates expect
  certbot-issued certs that nothing produces).
* The backend *must* run on the host (systemd) to write Nginx files, so it sits
  **outside** the pod network. That forced workarounds: pod `exec` over WebSocket
  fails on this cluster (kube-rs 0.91 is WebSocket-only; the API server only speaks
  SPDY for exec) and a direct DB connection times out (no route to the service
  network). DB password rotation had to be reworked to run as an in-cluster Job.
* DB backup/restore shells out to **`kubectl`** precisely because `kubectl`
  negotiates SPDY (so `exec`/`cp` work) where kube-rs cannot.

## Decision
Adopt the **cloud-native** model: **everything runs in the cluster**, with a single
in-cluster edge.

* **Edge:** **Traefik** (ships with k3s) is the sole ingress on `:80/:443`.
* **TLS:** **cert-manager** + a Let's Encrypt `ClusterIssuer`. Per-domain certs are
  issued automatically via the `cert-manager.io/cluster-issuer` annotation that
  `deploy_ingress` already emits.
* **Control plane:** the Rust backend runs **in-cluster** as a `Deployment` with a
  dedicated `ServiceAccount` + RBAC (`ClusterRole`), authenticating via the pod's
  service-account token (`kube::Config::infer()` auto-detects in-cluster).
* **Routing:** the backend manages **only** `Ingress` (+ Traefik `Middleware` for
  custom rules); host Nginx is removed entirely.
* **Dashboard:** served by a small static **frontend container** (nginx:alpine
  serving the built Angular bundle) behind Traefik — not by the API process.
* **Transparency (the ADR-003 win we keep):** store the generated `Ingress` /
  `Middleware` YAML in `last_applied_config` so the UI still shows exactly what is
  routing each domain.

### Cascading decisions (consequences of going in-cluster)
| Concern | Decision |
|---|---|
| Platform metadata DB (Postgres) | In-cluster `StatefulSet` + PVC (`hermes-postgres` service). |
| Native object storage (`/var/www/hermes/{storage,secure_storage}`) | PVC mounted at `/var/www/hermes` in the backend pod. |
| DB backups (`/var/lib/hermes/backups`) | PVC mounted at `/var/lib/hermes/backups` in the backend pod (kubectl `cp` lands dumps here). |
| App bind-volumes (`/var/lib/hermes/volumes/*`) | Stay **node hostPath** (used by app pods directly; the backend only writes the path into pod specs, never reads the files). |
| `kubectl` dependency (backup/restore exec/cp) | **Bundled into the backend image** (still needed: kubectl's SPDY fallback is why backups work). In-cluster it uses the service-account token automatically. |
| Secrets (`JWT_SECRET`, `HERMES_ENCRYPTION_KEY`, `HERMES_ROOT_PASSWORD`, `DATABASE_URL`) | Kubernetes `Secret`, injected as env. |
| Registry | Unchanged (`registry.kube-system…` + `registries.yaml`). |

## What gets deleted
* `backend/src/utils/nginx.rs`, `backend/src/utils/nginx_templates.rs` and all
  `NginxManager` call sites.
* Host Nginx, `/etc/ssl/hermes`, the certbot/acme-challenge assumptions.
* The host-systemd deployment of the backend; the `deploy.sh` host install.

## Migration plan (phased; each phase is independently shippable)
1. **Infra:** install **cert-manager** + Let's Encrypt `ClusterIssuer`; confirm
   Traefik owns `:80/:443`. (Additive, non-destructive.)
2. **App domains → Ingress-only:** drop `NginxManager` from the domain flows; map
   `client_max_body_size` / websockets / custom rules to Traefik `Middleware`;
   persist the generated YAML in `last_applied_config`. (Backend still on host;
   delivers automatic TLS for user domains.)
3. **Containerize the control plane:** `backend/Dockerfile` (multi-stage Rust build
   with `SQLX_OFFLINE=true`, runtime image includes `kubectl` + CA certs); k8s
   manifests — namespace, RBAC, Secret, Postgres `StatefulSet`+PVC, storage/backup
   PVCs, backend `Deployment`+`Service`.
4. **Frontend container + platform Ingress:** `frontend/Dockerfile` (build → nginx
   static); Ingress for the platform domain (`/` → frontend svc, `/api` → backend
   svc).
5. **Cutover (atomic) + cleanup:** point the node's `:80/:443` at Traefik, deploy
   the in-cluster stack, run migrations, verify; then delete `nginx.rs` /
   `nginx_templates.rs` and rewrite `scripts/` for the in-cluster model. Old host
   stack stays as the rollback target until verified.

## Rejected / deferred
* **Hybrid (edge in-cluster, brain on host):** rejected — needs an awkward
  host↔cluster `Endpoints` bridge and keeps the operational debt.
* **Gateway API (HTTPRoute) + operator-style reconciliation:** the professional
  end-state, but deferred — we keep imperative `Ingress` management for now.
* **Cloud Native Buildpacks** instead of Kaniko: out of scope.

## Consequences
* **Positive:** one edge; automatic TLS; the control plane is in the pod network so
  `exec`/direct-DB/networking are native (workarounds become optional); no host
  file management or `nginx -s reload`; reproducible, declarative deploys.
* **Negative:** larger one-time migration (image build, RBAC, PVCs, Postgres move);
  custom per-domain rules now go through Traefik `Middleware` instead of raw Nginx;
  the backend image must carry `kubectl`.
