#!/bin/bash
# ==============================================================================
#  Hermes OS - Script de Instalare Automată (Giga-Simplu, One-Command)
#  Sisteme compatibile: Ubuntu 22.04+ / Debian 11+
# ==============================================================================

set -e

# Culori pentru output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}======================================================================${NC}"
echo -e "${BLUE}        HERMES OS - Pornire Proces de Instalare Automată pe Server     ${NC}"
echo -e "${BLUE}======================================================================${NC}"

# 1. Verificare permisiuni root
if [ "$EUID" -ne 0 ]; then
  echo -e "${RED}Eroare: Acest script trebuie rulat ca root (sudo).${NC}"
  exit 1
fi

# 2. Actualizare sistem și instalare pachete de bază
echo -e "\n${YELLOW}[1/8] Actualizare sistem și instalare dependințe de sistem...${NC}"
apt-get update -y
apt-get install -y curl git build-essential libssl-dev pkg-config nginx postgresql postgresql-contrib

# 3. Instalare Rust Toolchain (dacă lipsește)
if ! command -v cargo &> /dev/null; then
  echo -e "\n${YELLOW}[2/8] Instalare Rust compiler (Rustup)...${NC}"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
else
  echo -e "\n${GREEN}[*] Rust este deja instalat.${NC}"
fi

# 4. Instalare Node.js și npm (dacă lipsesc)
if ! command -v node &> /dev/null; then
  echo -e "\n${YELLOW}[3/8] Instalare Node.js v20 (LTS)...${NC}"
  curl -fsSL https://deb.nodesource.com/setup_20.x | bash -
  apt-get install -y nodejs
else
  echo -e "\n${GREEN}[*] Node.js este deja instalat (${$(node -v)}).${NC}"
fi

# 5. Configurare bază de date PostgreSQL
echo -e "\n${YELLOW}[4/8] Configurare bază de date PostgreSQL...${NC}"
systemctl start postgresql
systemctl enable postgresql

# Setăm parola 'root' pentru utilizatorul postgres (sau poți înlocui cu o parolă securizată)
DB_PASS="root"
sudo -u postgres psql -c "ALTER USER postgres PASSWORD '$DB_PASS';"
sudo -u postgres psql -c "CREATE DATABASE hermes_db;" || echo -e "${YELLOW}Baza de date hermes_db există deja. Se continuă...${NC}"

# 6. Instalare și configurare Kubernetes (K3s)
if ! command -v kubectl &> /dev/null; then
  echo -e "\n${YELLOW}[5/8] Instalare K3s (Kubernetes lightweight)...${NC}"
  curl -sfL https://get.k3s.io | sh -
  
  # Așteaptă ca nodul să fie gata
  echo "Se așteaptă inițializarea clusterului..."
  sleep 15
else
  echo -e "\n${GREEN}[*] Kubernetes (K3s/kubectl) este deja instalat.${NC}"
fi

# Configurare permisiuni Kubeconfig pentru utilizatorul curent și root
mkdir -p ~/.kube
cp /etc/rancher/k3s/k3s.yaml ~/.kube/config
chmod 600 ~/.kube/config

# Instalează Metrics Server și Prometheus în cluster
echo -e "\n${YELLOW}[*] Instalare Metric-Server și Prometheus în Kubernetes...${NC}"
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml || true
# Permite rularea metrics-server pe K3s fără certificate TLS semnate
kubectl patch deployment metrics-server -n kube-system --type='json' -p='[{"op": "add", "path": "/spec/template/spec/containers/0/args/-", "value": "--kubelet-insecure-tls"}]' || true

# Aplica manifestul prometheus
if [ -f "prometheus-deployment.yaml" ]; then
  kubectl apply -f prometheus-deployment.yaml
elif [ -f "../prometheus-deployment.yaml" ]; then
  kubectl apply -f ../prometheus-deployment.yaml
fi

# 7. Compilare și Build Frontend (Angular)
echo -e "\n${YELLOW}[6/8] Instalare module și compilare Frontend Angular...${NC}"
cd "$(dirname "$0")/../frontend"
npm install
npm run build
mkdir -p /var/www/hermes/frontend
cp -r dist/frontend/* /var/www/hermes/frontend/
echo -e "${GREEN}[*] Frontend compilat și plasat în /var/www/hermes/frontend.${NC}"

# 8. Compilare și pornire Backend (Rust)
echo -e "\n${YELLOW}[7/8] Compilare Backend Rust (Release)...${NC}"
cd ../backend
cargo build --release

# Inițializare fișier .env pentru backend
if [ ! -f ".env" ]; then
  cat <<EOF > .env
DATABASE_URL=postgres://postgres:$DB_PASS@127.0.0.1:5432/hermes_db
HERMES_REGISTRY_URL=127.0.0.1:5000
HERMES_PROMETHEUS_URL=http://prometheus-k8s.monitoring.svc:9090
PORT=8000
EOF
fi

# Creare serviciu systemd pentru backend
echo -e "\n${YELLOW}[8/8] Configurare serviciu systemd și Nginx Reverse Proxy...${NC}"
cat <<EOF > /etc/systemd/system/hermes-backend.service
[Unit]
Description=Hermes OS Backend Engine
After=network.target postgresql.service

[Service]
Type=simple
User=root
WorkingDirectory=$(pwd)
ExecStart=$(pwd)/target/release/backend
Restart=on-failure
EnvironmentFile=$(pwd)/.env

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable hermes-backend
systemctl restart hermes-backend

# Configurare Nginx Reverse Proxy
cat <<EOF > /etc/nginx/sites-available/hermes
server {
    listen 80;
    server_name _; # Acceptă orice IP/Domeniu configurat pe server

    # Frontend Angular
    location / {
        root /var/www/hermes/frontend;
        index index.html;
        try_files \$uri \$uri/ /index.html;
    }

    # API Proxy către Backend Rust
    location /api/ {
        proxy_pass http://127.0.0.1:8000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection "Upgrade";
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        
        # Securitate / optimizare pentru SSE (Live Telemetry Stream)
        proxy_set_header Connection '';
        proxy_buffering off;
        proxy_cache off;
        chunked_transfer_encoding off;
        read_timeout 600s;
    }
}
EOF

# Activare configurare Nginx
ln -sf /etc/nginx/sites-available/hermes /etc/nginx/sites-enabled/
rm -f /etc/nginx/sites-enabled/default
systemctl restart nginx

echo -e "\n${GREEN}======================================================================${NC}"
echo -e "${GREEN}      HERMES OS INSTALAT CU SUCCES!                                   ${NC}"
echo -e "${GREEN}      Accesează platforma la: http://<IP_SERVER>                      ${NC}"
echo -e "${GREEN}======================================================================${NC}"
