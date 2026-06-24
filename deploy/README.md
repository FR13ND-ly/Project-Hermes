# Hermes — Cloud-Native Deployment

Everything runs **in the cluster** (single-node k3s): the Rust control plane, the
static dashboard, and Postgres, behind **Traefik** with automatic TLS via
**cert-manager**. This supersedes the old host-Nginx install
(`scripts/deploy.sh`). See [ADR-005](../docs/adr/005-cloud-native-control-plane.md).

## One-command install (fresh Ubuntu/Debian server)

```bash
sudo ./scripts/hermes.sh install
```

The installer **asks for** the email (Let's Encrypt), dashboard hostname, base
domain and public ingress IP at the start (press Enter to accept each default).
To skip the prompts — e.g. for CI / unattended runs — pass them as env vars (when
there's no terminal, prompting is disabled and these are used directly):

```bash
sudo CERT_EMAIL=you@example.com \
     DASHBOARD_HOST=dashboard.example.com \
     HERMES_BASE_DOMAIN=example.com \
     ./scripts/hermes.sh install
```

This installs Docker + k3s, deploys the in-cluster registry (for user app builds)
and configures k3s to trust it, installs cert-manager + Let's Encrypt issuers,
Knative Serving (serverless), **kpack** (the default Buildpacks builder) and
**Prometheus** (telemetry charts), builds & imports the backend/frontend images,
generates platform secrets, and applies the stack. The backend runs DB migrations
automatically on boot.

The installer also injects `HERMES_INGRESS_IP` (the IP custom app/serverless
domains resolve to) — auto-detected from the node, or set it explicitly:

```bash
sudo HERMES_INGRESS_IP=203.0.113.10 ... ./scripts/hermes.sh install
```

Then point `DASHBOARD_HOST`'s DNS at the node's public IP and open
`https://DASHBOARD_HOST`. The first-boot super-admin password is printed during
install (and stored in the `hermes-secrets` Secret).

Re-run a single piece any time: `./scripts/hermes.sh monitoring` (Prometheus),
`./scripts/hermes.sh knative` (serverless) or `./scripts/hermes.sh kpack` (builder).

> **Builder:** apps are built with **kpack / Cloud Native Buildpacks** by default
> (no Dockerfile needed). To fall back to the legacy generated-Dockerfile + Kaniko
> path, set `HERMES_BUILDER=kaniko` on `deploy/hermes-backend`. The in-cluster
> registry is plain HTTP — see the insecure-registry caveat in `90-kpack.yaml` if
> kpack builds fail on the push step.

> **TLS tip:** while testing, set the dashboard/app Ingress issuer to
> `letsencrypt-staging` to avoid Let's Encrypt rate limits, then switch to
> `letsencrypt-prod`.

## Update (deploy latest code)

```bash
sudo ./scripts/hermes.sh update    # git pull + rebuild images + rolling restart
```

Secrets are never regenerated on update (that would invalidate sessions and make
stored encrypted values undecryptable).

## Manifests (applied in order by the script)

| File | Purpose |
|------|---------|
| `00-namespace.yaml` | `hermes-system` namespace |
| `05-registry.yaml` | in-cluster registry for **user** app builds (Kaniko) |
| `10-cert-manager-clusterissuer.yaml` | Let's Encrypt issuers (prod + staging) |
| `20-rbac.yaml` | ServiceAccount + scoped ClusterRole for the orchestrator |
| `30-secret.example.yaml` | template; the script generates the real Secret |
| `40-postgres.yaml` | platform Postgres (StatefulSet + PVC) |
| `50-storage-pvc.yaml` | PVCs: object storage + DB backups |
| `60-backend.yaml` | control-plane Deployment + Service (image carries `kubectl`) |
| `70-frontend.yaml` | static dashboard Deployment + Service |
| `80-ingress.yaml` | platform Ingress (`/api`,`/storage`→backend, `/`→frontend) |
| `90-kpack.yaml` | kpack ClusterStack/ClusterStore/ClusterBuilder (`hermes-builder`) — the default Buildpacks builder |

## Notes
* **Prerequisites for cert-manager / cert issuance:** the dashboard host (and any
  custom app domain) must resolve to this node, and ports 80/443 must be reachable
  (HTTP-01 challenge goes through Traefik).
* **Registry:** Kaniko pushes built app images to
  `registry.kube-system.svc.cluster.local:80`; the node pulls them via the
  `registries.yaml` the installer writes. Platform images are imported directly
  into containerd (`k3s ctr images import`), not via the registry.
* **Multi-node:** these manifests assume single-node (RWO PVCs, image import on one
  node). Multi-node needs shared storage + pushing platform images to the registry.
