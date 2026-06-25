#!/bin/bash
# ==============================================================================
#  Hermes OS - Automatic Installation Script (Giga-Simple, One-Command)
#  Compatible systems: Ubuntu 22.04+ / Debian 11+
# ==============================================================================

set -e

# Output colors
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}======================================================================${NC}"
echo -e "${BLUE}        HERMES OS - Starting Automatic Server Installation Process     ${NC}"
echo -e "${BLUE}======================================================================${NC}"

# 1. Verify root permissions
if [ "$EUID" -ne 0 ]; then
  echo -e "${RED}Error: This script must be run as root (sudo).${NC}"
  exit 1
fi

# 2. System update and installation of base packages
echo -e "\n${YELLOW}[1/8] Updating system and installing system dependencies...${NC}"
apt-get update -y
apt-get install -y curl git build-essential libssl-dev pkg-config nginx postgresql postgresql-contrib

# 3. Install Rust Toolchain (if missing)
if ! command -v cargo &> /dev/null; then
  echo -e "\n${YELLOW}[2/8] Installing Rust compiler (Rustup)...${NC}"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
else
  echo -e "\n${GREEN}[*] Rust is already installed.${NC}"
fi

# 4. Install Node.js and npm (if missing)
if ! command -v node &> /dev/null; then
  echo -e "\n${YELLOW}[3/8] Installing Node.js v20 (LTS)...${NC}"
  curl -fsSL https://deb.nodesource.com/setup_20.x | bash -
  apt-get install -y nodejs
else
  echo -e "\n${GREEN}[*] Node.js is already installed (${$(node -v)}).${NC}"
fi

# 5. Configure PostgreSQL database
echo -e "\n${YELLOW}[4/8] Configuring PostgreSQL database...${NC}"
systemctl start postgresql
systemctl enable postgresql

# Set the 'root' password for the postgres user (or replace with a secure password)
DB_PASS="root"
sudo -u postgres psql -c "ALTER USER postgres PASSWORD '$DB_PASS';"
sudo -u postgres psql -c "CREATE DATABASE hermes_db;" || echo -e "${YELLOW}Database hermes_db already exists. Continuing...${NC}"

# 6. Install and configure Kubernetes (K3s)
if ! command -v kubectl &> /dev/null; then
  echo -e "\n${YELLOW}[5/8] Installing K3s (lightweight Kubernetes)...${NC}"
  curl -sfL https://get.k3s.io | sh -
  
  # Wait for cluster initialization
  echo "Waiting for cluster initialization..."
  sleep 15
else
  echo -e "\n${GREEN}[*] Kubernetes (K3s/kubectl) is already installed.${NC}"
fi

# Configure Kubeconfig permissions for the current user and root
mkdir -p ~/.kube
cp /etc/rancher/k3s/k3s.yaml ~/.kube/config
chmod 600 ~/.kube/config

# Install Metrics Server and Prometheus in the cluster
echo -e "\n${YELLOW}[*] Installing Metric-Server and Prometheus in Kubernetes...${NC}"
kubectl apply -f https://github.com/kubernetes-sigs/metrics-server/releases/latest/download/components.yaml || true
# Allow running metrics-server on K3s without signed TLS certificates
kubectl patch deployment metrics-server -n kube-system --type='json' -p='[{"op": "add", "path": "/spec/template/spec/containers/0/args/-", "value": "--kubelet-insecure-tls"}]' || true

# Apply the prometheus manifest
if [ -f "prometheus-deployment.yaml" ]; then
  kubectl apply -f prometheus-deployment.yaml
  elif [ -f "../prometheus-deployment.yaml" ]; then
  kubectl apply -f ../prometheus-deployment.yaml
fi

# 7. Compile and Build Frontend (Angular)
echo -e "\n${YELLOW}[6/8] Installing modules and compiling Frontend Angular...${NC}"
cd "$(dirname "$0")/../frontend"
npm install
npm run build
mkdir -p /var/www/hermes/frontend
cp -r dist/frontend/* /var/www/hermes/frontend/
echo -e "${GREEN}[*] Frontend compiled and placed in /var/www/hermes/frontend.${NC}"

# 8. Compile and start Backend (Rust)
echo -e "\n${YELLOW}[7/8] Compiling Rust Backend (Release)...${NC}"
cd ../backend
cargo build --release

# Initialize .env file for backend
if [ ! -f ".env" ]; then
  cat <<EOF > .env
DATABASE_URL=postgres://postgres:$DB_PASS@127.0.0.1:5432/hermes_db
HERMES_REGISTRY_URL=127.0.0.1:5000
HERMES_PROMETHEUS_URL=http://prometheus-k8s.monitoring.svc:9090
PORT=8000
EOF
fi

# Create systemd service for backend
echo -e "\n${YELLOW}[8/8] Configuring systemd service and Nginx Reverse Proxy...${NC}"
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

# Configure Nginx Reverse Proxy
cat <<EOF > /etc/nginx/sites-available/hermes
server {
    listen 80;
    server_name _; # Accepts any IP/Domain configured on the server

    # Frontend Angular
    location / {
        root /var/www/hermes/frontend;
        index index.html;
        try_files \$uri \$uri/ /index.html;
    }

    # API Proxy to Rust Backend
    location /api/ {
        proxy_pass http://127.0.0.1:8000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade \$http_upgrade;
        proxy_set_header Connection "Upgrade";
        proxy_set_header Host \$host;
        proxy_set_header X-Real-IP \$remote_addr;
        proxy_set_header X-Forwarded-For \$proxy_add_x_forwarded_for;
        
        # Security / optimization for SSE (Live Telemetry Stream)
        proxy_set_header Connection '';
        proxy_buffering off;
        proxy_cache off;
        chunked_transfer_encoding off;
        read_timeout 600s;
    }
}
EOF

# Enable Nginx configuration
ln -sf /etc/nginx/sites-available/hermes /etc/nginx/sites-enabled/
rm -f /etc/nginx/sites-enabled/default
systemctl restart nginx

echo -e "\n${GREEN}======================================================================${NC}"
echo -e "${GREEN}      HERMES OS INSTALLED SUCCESSFULLY!                               ${NC}"
echo -e "${GREEN}      Access the platform at: http://<SERVER_IP>                      ${NC}"
echo -e "${GREEN}======================================================================${NC}"
