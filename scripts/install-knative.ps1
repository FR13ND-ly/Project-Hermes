# Install Knative Serving CRDs
Write-Host "Installing Knative Serving CRDs..." -ForegroundColor Cyan
kubectl apply -f https://github.com/knative/serving/releases/download/knative-v1.13.0/serving-crds.yaml

# Install Knative Serving Core
Write-Host "Installing Knative Serving Core..." -ForegroundColor Cyan
kubectl apply -f https://github.com/knative/serving/releases/download/knative-v1.13.0/serving-core.yaml

# Install Net-Kourier (Knative networking layer)
Write-Host "Installing Knative Kourier Ingress..." -ForegroundColor Cyan
kubectl apply -f https://github.com/knative/net-kourier/releases/download/knative-v1.13.0/kourier.yaml

# Configure Knative to use Kourier as default ingress
Write-Host "Configuring Knative network class..." -ForegroundColor Cyan
kubectl patch configmap/config-network `
  --namespace knative-serving `
  --type merge `
  --patch '{"data":{"ingress-class":"kourier.ingress.networking.knative.dev"}}'

# Configure Knative DNS (sslip.io magic wildcard domain)
Write-Host "Configuring Knative default domain..." -ForegroundColor Cyan
kubectl apply -f https://github.com/knative/serving/releases/download/knative-v1.13.0/serving-default-domain.yaml

Write-Host "Knative Serving successfully installed on K3s cluster!" -ForegroundColor Green
