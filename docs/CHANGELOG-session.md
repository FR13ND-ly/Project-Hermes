# Hermes — session changelog & rollout guide

All changes below compile (`cargo check` / `ng build` exit 0). **None are validated against a
live cluster** — risky pieces are behind feature flags (default OFF) or are manifest changes.

> ⚠️ **Back up the database before deploying.** The BaaS migration RESETS `app_users`
> (drops existing end-user accounts — agreed early-stage reset).

---

## 1. New migrations (run automatically on boot, in order)

| Migration | Effect |
|---|---|
| `20260618140000_baas_auth_per_app` | ⚠️ **RESETS `app_users`** → per-app identity (identifier + password) + `app_refresh_tokens` |
| `20260618160000_app_autoscale_cpu_target` | `autoscale_cpu_percent` column (default 80) |
| `20260618170000_app_auto_sleep` | `auto_sleep_enabled` + `auto_sleep_after_minutes`; production backfilled to disabled |
| `20260619000000_leader_lease` | single-row lease for leader election |
| `20260619010000_rate_limit_counters` | shared (cross-replica) rate limiting |

---

## 2. Feature flags (opt-in — default OFF, behaviour unchanged until set)

| Flag | Default | Effect |
|---|---|---|
| `HERMES_BUILDER=kpack` | kaniko | build via kpack/Buildpacks — **requires Stage 2 infra first**, else builds fail |
| `HERMES_RECONCILE=on` | off | steady-state reconcile loop (self-heals drifted Deployments) |
| `BAAS_ACCESS_EXPIRY` / `BAAS_REFRESH_EXPIRY` | 900s / 30d | BaaS access / refresh token TTLs |
| `HERMES_MAX_CONCURRENT_BUILDS` | 3 | now a GLOBAL (cluster-wide) build cap, not per-replica |

### How to set flags
Flags are environment variables on the backend Deployment.

```bash
# Temporary (testing) — triggers a rollout immediately:
kubectl -n hermes-system set env deploy/hermes-backend HERMES_RECONCILE=on

# Inspect what's set / unset a flag:
kubectl -n hermes-system set env deploy/hermes-backend --list | grep HERMES
kubectl -n hermes-system set env deploy/hermes-backend HERMES_RECONCILE-   # trailing dash = unset
```

For a **permanent** flag, add it to the `env:` list in `deploy/60-backend.yaml` (next to
`HERMES_BASE_DOMAIN`) and run `sudo ./scripts/hermes.sh update`. Note: a `set env` value is
overwritten on the next `hermes.sh update` (which re-applies the manifest) — so persist
permanent flags in the manifest.

---

## 3. Active by default (take effect on the next deploy — no flag)

- **PG/MySQL backup + restore run in an in-cluster Job** (no more control-plane `pg_dump`;
  fixes "program not found"). Mongo/Redis/custom still use the control-plane path.
- **BaaS auth refactor**: per-app `identifier` (not email), access + refresh tokens,
  request-time custom claims via the `X-Hermes-Auth-Secret` header, rate-limiting, new
  `/auth/refresh` + `/auth/logout` endpoints.
- **Scaling**: per-app autoscale CPU target; per-app auto-sleep (enable + timeout); manual
  Start restores `replicas_min`; `deploy_app` omits `replicas` when an HPA is active (bug fix).
- **HA (single-node)**: leader election (always-on), Postgres-backed workspace locks +
  rate-limits, graceful shutdown, global build cap.
- **Manifest**: `replicas: 2` + `RollingUpdate`.

---

## 4. Deploy steps (in order)

1. **Back up the DB** (the BaaS migration resets `app_users`).
2. `sudo ./scripts/hermes.sh update` — rebuilds the control-plane image + rolls out;
   migrations run on boot.
3. Validate (section 5). Enable flags only after confirming their prerequisites.

---

## 5. Cluster validation (per feature)

- **Migrations**: `kubectl -n hermes-system logs deploy/hermes-backend | grep -i migrat` → no errors.
- **Backup**: trigger a backup on a Postgres DB → succeeds (no "program not found"); then restore.
- **HA / leader**: `kubectl -n hermes-system get pods` → 2 Ready; exactly one pod logs
  "Acquired control-plane leadership"; crons fire once (not twice).
- **Zero-downtime rollout**: `kubectl -n hermes-system rollout restart deploy/hermes-backend`
  while curling `/health` in a loop → no 5xx.
- **Scaling**: Settings tab → set Replicas Min / Autoscale CPU / Auto-sleep, save, check pods.
- **BaaS**: `register` → `{accessToken, refreshToken}`; `refresh` rotates; with
  `X-Hermes-Auth-Secret` + `additionalClaims` → custom claims appear in the token.
- **reconcile** (after `HERMES_RECONCILE=on`): `kubectl delete deployment <app>` → it
  self-heals, with NO spurious rollouts on healthy apps.

---

## 6. Still open

- **kpack Stage 2/3**: install kpack + ClusterBuilder/Stack/Store + a `hermes-kpack`
  ServiceAccount with git/registry secrets per workspace namespace + `kpack.io` RBAC; then
  build-pod log capture + in-cluster registry addressing; then flip `HERMES_BUILDER=kpack`.
- **reconcile Stage 3** (optional): retire the stuck-reconciler, event-triggered reconcile,
  Service/ingress convergence; or a kube-rs `Controller`/CRDs.
- **Audit findings NOT yet fixed** (separate from the architecture work): command injection
  in the kaniko cloner script (#2), custom backup command running as shell on the control
  plane (#1), JWT accepted via `?token=` query param (#3). kpack removes #2 indirectly;
  moving the custom backup into a Job removes #1.
- **Multi-node HA** (RWX or object storage): out of scope per decision — single-node
  co-mounts the RWO PVCs, which is why MinIO isn't needed here.
