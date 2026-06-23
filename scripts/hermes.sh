#!/usr/bin/env bash
# ==============================================================================
#  Hermes — cloud-native installer / updater (single-node k3s, Ubuntu/Debian)
#
#  Everything runs IN the cluster: control plane (Rust) + dashboard (static) +
#  Postgres, behind Traefik (k3s default) with automatic TLS via cert-manager.
#  See docs/adr/005-cloud-native-control-plane.md.
#
#  Usage (as root):
#    ./scripts/hermes.sh install         # prompts for email/hosts/IP interactively
#    ./scripts/hermes.sh update          # rebuild + roll out latest code
#    ./scripts/hermes.sh status
#
#  Prompts are pre-filled from env vars when set — pass them to skip the questions
#  (or for non-interactive / CI runs, where prompting is disabled automatically):
#    CERT_EMAIL=you@example.com DASHBOARD_HOST=dashboard.example.com \
#      HERMES_BASE_DOMAIN=example.com ./scripts/hermes.sh install
# ==============================================================================
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
NS="hermes-system"
REGISTRY_HOST="registry.kube-system.svc.cluster.local:80"
KUBECTL="k3s kubectl"

CERT_EMAIL="${CERT_EMAIL:-admin@example.com}"
DASHBOARD_HOST="${DASHBOARD_HOST:-dashboard.hermes.local}"
HERMES_BASE_DOMAIN="${HERMES_BASE_DOMAIN:-hermes.local}"
# IP that custom app/serverless domains resolve to. Empty = auto-detect the node IP.
HERMES_INGRESS_IP="${HERMES_INGRESS_IP:-}"

c()  { printf '\033[1;34m[hermes]\033[0m %s\n' "$*"; }
ok() { printf '\033[1;32m[ ok ]\033[0m %s\n' "$*"; }
die(){ printf '\033[1;31m[fail]\033[0m %s\n' "$*" >&2; exit 1; }

require_root() { [ "$(id -u)" -eq 0 ] || die "Run as root (sudo)."; }

# Prompt for a value, showing a default; Enter keeps the default. Echoes the choice.
ask() {
  local msg="$1" def="$2" ans
  printf '\033[1;34m[hermes]\033[0m %s [\033[1;33m%s\033[0m]: ' "$msg" "${def:-—}" >&2
  read -r ans || ans=""
  printf '%s' "${ans:-$def}"
}

# Interactively collect install config, pre-filled with env vars / defaults. Skipped
# when there's no terminal (CI / piped) — env vars and defaults are used as-is.
collect_config() {
  if [ ! -t 0 ]; then
    c "No terminal detected — using env vars / defaults (CERT_EMAIL, DASHBOARD_HOST, …)."
    return
  fi
  c "Configure this Hermes install — press Enter to accept each default:"
  CERT_EMAIL="$(ask "Email for Let's Encrypt / TLS certificates" "$CERT_EMAIL")"
  DASHBOARD_HOST="$(ask "Dashboard hostname (point its DNS at this node)" "$DASHBOARD_HOST")"
  HERMES_BASE_DOMAIN="$(ask "Base domain for generated app subdomains" "$HERMES_BASE_DOMAIN")"
  local ip_default="$HERMES_INGRESS_IP"
  [ -n "$ip_default" ] || ip_default="$(detect_node_ip)"
  HERMES_INGRESS_IP="$(ask "Public IP custom app/serverless domains resolve to" "$ip_default")"
  echo
  c "Using: CERT_EMAIL=$CERT_EMAIL  DASHBOARD_HOST=$DASHBOARD_HOST"
  c "       HERMES_BASE_DOMAIN=$HERMES_BASE_DOMAIN  HERMES_INGRESS_IP=${HERMES_INGRESS_IP:-auto}"
}

# ── System dependencies (Docker for building images, git, openssl) ────────────
install_system_deps() {
  c "Installing system dependencies (docker, git, curl, openssl)..."
  apt-get update -y
  apt-get install -y --no-install-recommends docker.io git curl openssl ca-certificates
  systemctl enable --now docker
  ok "System deps ready."
}

# ── k3s (ships Traefik on :80/:443 and a bundled kubectl) ─────────────────────
install_k3s() {
  if command -v k3s >/dev/null 2>&1; then ok "k3s already installed."; return; fi
  c "Installing k3s..."
  curl -sfL https://get.k3s.io | sh -
  c "Waiting for the cluster to be ready..."
  until $KUBECTL get nodes >/dev/null 2>&1; do sleep 2; done
  ok "k3s ready."
}

# ── In-cluster registry for USER app builds (Kaniko push / node pull) ─────────
setup_registry() {
  c "Deploying in-cluster registry + configuring k3s to trust it..."
  $KUBECTL apply -f "$ROOT_DIR/deploy/05-registry.yaml"
  $KUBECTL -n kube-system rollout status deploy/registry --timeout=120s
  local cip
  cip="$($KUBECTL -n kube-system get svc registry -o jsonpath='{.spec.clusterIP}')"
  [ -n "$cip" ] || die "Could not read registry ClusterIP."
  # The node's containerd resolves the in-cluster DNS name to the ClusterIP and
  # treats it as insecure (plain HTTP), so both Kaniko (push) and the node (pull)
  # can use registry.kube-system.svc.cluster.local:80.
  mkdir -p /etc/rancher/k3s
  cat > /etc/rancher/k3s/registries.yaml <<EOF
mirrors:
  "$REGISTRY_HOST":
    endpoint:
      - "http://$cip:80"
configs:
  "$REGISTRY_HOST":
    tls:
      insecure_skip_verify: true
EOF
  c "Restarting k3s to apply registries.yaml..."
  systemctl restart k3s
  until $KUBECTL get nodes >/dev/null 2>&1; do sleep 2; done
  ok "Registry ready ($REGISTRY_HOST -> $cip)."
}

# ── cert-manager + Let's Encrypt issuers ──────────────────────────────────────
install_cert_manager() {
  if $KUBECTL get ns cert-manager >/dev/null 2>&1; then
    ok "cert-manager already installed."
  else
    c "Installing cert-manager..."
    $KUBECTL apply -f https://github.com/cert-manager/cert-manager/releases/latest/download/cert-manager.yaml
    $KUBECTL -n cert-manager rollout status deploy/cert-manager-webhook --timeout=180s
  fi
  c "Applying Let's Encrypt ClusterIssuers (email: $CERT_EMAIL)..."
  sed "s/CHANGE_ME@example.com/$CERT_EMAIL/g" "$ROOT_DIR/deploy/10-cert-manager-clusterissuer.yaml" | $KUBECTL apply -f -
  ok "cert-manager + issuers ready."
}

# ── Knative Serving + Kourier (required for serverless functions) ─────────────
# Heavy-ish (controller + webhook + autoscaler + activator + Kourier); on a small
# node give it headroom. Idempotent: skipped if the Knative Service CRD exists.
install_knative() {
  local kv="knative-v1.13.0"
  if $KUBECTL get crd services.serving.knative.dev >/dev/null 2>&1; then
    ok "Knative Serving already installed."
    return
  fi
  c "Installing Knative Serving ($kv) + Kourier (for serverless functions)..."
  $KUBECTL apply -f "https://github.com/knative/serving/releases/download/$kv/serving-crds.yaml"
  $KUBECTL apply -f "https://github.com/knative/serving/releases/download/$kv/serving-core.yaml"
  $KUBECTL apply -f "https://github.com/knative/net-kourier/releases/download/$kv/kourier.yaml"
  $KUBECTL patch configmap/config-network -n knative-serving --type merge \
    -p '{"data":{"ingress-class":"kourier.ingress.networking.knative.dev"}}'
  $KUBECTL apply -f "https://github.com/knative/serving/releases/download/$kv/serving-default-domain.yaml"
  c "Waiting for Knative controllers to come up..."
  $KUBECTL -n knative-serving rollout status deploy/controller --timeout=300s || true
  $KUBECTL -n knative-serving rollout status deploy/webhook --timeout=300s || true
  ok "Knative Serving ready."
}

# ── Monitoring (Prometheus) — powers the telemetry charts (app/db CPU/RAM, RED) ─
# The backend queries it at HERMES_PROMETHEUS_URL (default prometheus-k8s.monitoring.svc:9090).
# Idempotent: re-apply is a no-op once the Deployment exists.
install_monitoring() {
  c "Deploying Prometheus (monitoring namespace) for telemetry..."
  $KUBECTL apply -f "$ROOT_DIR/prometheus-deployment.yaml"
  $KUBECTL -n monitoring rollout status deploy/prometheus --timeout=180s || true
  ok "Monitoring ready (Prometheus → prometheus-k8s.monitoring.svc:9090)."
}

# Best-effort detection of the node IP that custom app/serverless domains should
# point their A-records at (ExternalIP → InternalIP → default-route source).
detect_node_ip() {
  local ip
  ip="$($KUBECTL get node -o jsonpath='{.items[0].status.addresses[?(@.type=="ExternalIP")].address}' 2>/dev/null || true)"
  [ -n "$ip" ] || ip="$($KUBECTL get node -o jsonpath='{.items[0].status.addresses[?(@.type=="InternalIP")].address}' 2>/dev/null || true)"
  [ -n "$ip" ] || ip="$(ip route get 1.1.1.1 2>/dev/null | awk '{print $7; exit}')"
  printf '%s' "$ip"
}

# ── Build the platform images on the host and import into k3s containerd ──────
build_and_import_images() {
  c "Building backend image (this compiles Rust; takes a few minutes)..."
  docker build -t hermes-control-plane:latest "$ROOT_DIR/backend"
  docker save hermes-control-plane:latest | k3s ctr images import -
  c "Building frontend image..."
  docker build -t hermes-frontend:latest "$ROOT_DIR/frontend"
  docker save hermes-frontend:latest | k3s ctr images import -
  ok "Images built and imported into k3s."
}

# ── Secrets (generated once; never regenerated on update) ─────────────────────
ensure_secret() {
  $KUBECTL get ns "$NS" >/dev/null 2>&1 || $KUBECTL apply -f "$ROOT_DIR/deploy/00-namespace.yaml"
  if $KUBECTL -n "$NS" get secret hermes-secrets >/dev/null 2>&1; then
    ok "Secret hermes-secrets already exists (kept as-is)."
    return
  fi
  c "Generating platform secrets..."
  local jwt enc rootpw dbpw
  jwt="$(openssl rand -hex 32)"        # >= 32 chars
  enc="$(openssl rand -hex 16)"        # exactly 32 bytes
  rootpw="$(openssl rand -hex 16)"
  dbpw="$(openssl rand -hex 16)"
  $KUBECTL -n "$NS" create secret generic hermes-secrets \
    --from-literal=JWT_SECRET="$jwt" \
    --from-literal=HERMES_ENCRYPTION_KEY="$enc" \
    --from-literal=HERMES_ROOT_PASSWORD="$rootpw" \
    --from-literal=POSTGRES_PASSWORD="$dbpw" \
    --from-literal=DATABASE_URL="postgres://postgres:$dbpw@hermes-postgres:5432/hermes_db"
  ok "Secrets created. ROOT password (save it!): $rootpw"
}

# ── Apply the in-cluster stack ────────────────────────────────────────────────
apply_stack() {
  c "Applying Hermes manifests..."
  $KUBECTL apply -f "$ROOT_DIR/deploy/00-namespace.yaml"
  $KUBECTL apply -f "$ROOT_DIR/deploy/20-rbac.yaml"
  $KUBECTL apply -f "$ROOT_DIR/deploy/40-postgres.yaml"
  $KUBECTL apply -f "$ROOT_DIR/deploy/50-storage-pvc.yaml"
  $KUBECTL -n "$NS" rollout status statefulset/hermes-postgres --timeout=180s
  $KUBECTL apply -f "$ROOT_DIR/deploy/60-backend.yaml"
  $KUBECTL apply -f "$ROOT_DIR/deploy/70-frontend.yaml"
  sed "s/dashboard.hermes.local/$DASHBOARD_HOST/g" "$ROOT_DIR/deploy/80-ingress.yaml" | $KUBECTL apply -f -
  # Point the backend at the real base domain + the ingress IP that custom
  # app/serverless domains resolve to (explicit override or auto-detected node IP;
  # without it domains default to 127.0.0.1 and never resolve externally). Both set
  # in one call so the backend rolls out only once.
  local ingress_ip="$HERMES_INGRESS_IP"
  [ -n "$ingress_ip" ] || ingress_ip="$(detect_node_ip)"
  if [ -n "$ingress_ip" ]; then
    $KUBECTL -n "$NS" set env deploy/hermes-backend \
      HERMES_BASE_DOMAIN="$HERMES_BASE_DOMAIN" HERMES_INGRESS_IP="$ingress_ip" >/dev/null
    c "Custom domains will point at $ingress_ip (override with HERMES_INGRESS_IP)."
  else
    $KUBECTL -n "$NS" set env deploy/hermes-backend HERMES_BASE_DOMAIN="$HERMES_BASE_DOMAIN" >/dev/null
    c "Could not auto-detect node IP — set HERMES_INGRESS_IP before adding custom domains."
  fi
  $KUBECTL -n "$NS" rollout status deploy/hermes-backend --timeout=180s
  ok "Stack applied. Backend runs schema migrations automatically on boot."
}

cmd_install() {
  require_root
  collect_config
  install_system_deps
  install_k3s
  setup_registry
  install_cert_manager
  install_knative
  install_monitoring
  build_and_import_images
  ensure_secret
  apply_stack
  ok "Hermes is up. Point DNS for '$DASHBOARD_HOST' at this node, then open https://$DASHBOARD_HOST"
}

cmd_update() {
  require_root
  c "Pulling latest code..."
  git -C "$ROOT_DIR" pull --ff-only
  build_and_import_images
  c "Rolling out new images (migrations run on backend boot)..."
  $KUBECTL -n "$NS" rollout restart deploy/hermes-backend deploy/hermes-frontend
  $KUBECTL -n "$NS" rollout status deploy/hermes-backend --timeout=180s
  ok "Update complete."
}

cmd_status() {
  $KUBECTL -n "$NS" get pods,svc,ingress
  echo "---"
  $KUBECTL get clusterissuer 2>/dev/null || true
}

case "${1:-}" in
  install)    cmd_install ;;
  update)     cmd_update ;;
  knative)    require_root; install_knative ;;
  monitoring) require_root; install_monitoring ;;
  status)     cmd_status ;;
  *) die "Usage: $0 install|update|knative|monitoring|status  (install prompts interactively; env vars CERT_EMAIL, DASHBOARD_HOST, HERMES_BASE_DOMAIN, HERMES_INGRESS_IP pre-fill/skip the prompts)";;
esac
