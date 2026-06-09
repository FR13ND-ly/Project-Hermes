#!/bin/bash
# Enable systemd in WSL if not already enabled
if [ ! -f /etc/wsl.conf ] || ! grep -q "systemd=true" /etc/wsl.conf; then
  echo -e "[boot]\nsystemd=true" | sudo tee -a /etc/wsl.conf
  echo "Systemd enabled in WSL. Please restart WSL on Windows (run 'wsl --shutdown' in PowerShell) and run this script again."
  exit 0
fi

# Install K3s (disabling default Traefik if using Knative Kourier)
curl -sfL https://get.k3s.io | sh -

# Wait for K3s to be ready
echo "Waiting for K3s cluster to start..."
until sudo kubectl get nodes &> /dev/null; do
  sleep 2
done

# Copy kubeconfig to Windows user profile
WINDOWS_USER=$(cmd.exe /c "echo %USERNAME%" 2>/dev/null | tr -d '\r')
if [ -n "$WINDOWS_USER" ]; then
  mkdir -p "/mnt/c/Users/$WINDOWS_USER/.kube"
  sudo cp /etc/rancher/k3s/k3s.yaml "/mnt/c/Users/$WINDOWS_USER/.kube/config"
  sudo chmod 644 "/mnt/c/Users/$WINDOWS_USER/.kube/config"
  echo "Kubeconfig copied to Windows user profile: C:\\Users\\$WINDOWS_USER\\.kube\\config"
fi
