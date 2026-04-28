#!/bin/bash
set -e

# Stormchaser MicroK8s Setup & Deployment Script
# Handles image building, cluster preparation, and full Helm deployment.

GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
NAMESPACE="stormchaser"

# 1. Build Docker Images
echo -e "${BLUE}>>> Building Stormchaser Docker images...${NC}"
declare -A COMPONENTS
COMPONENTS=(
    ["stormchaser-api"]="stormchaser-api"
    ["stormchaser-engine"]="stormchaser-engine"
    ["stormchaser-runner-k8s"]="stormchaser-runner-k8s"
    ["stormchaser-agent"]="stormchaser-agent"
)

for IMAGE_NAME in "${!COMPONENTS[@]}"; do
    BINARY_NAME=${COMPONENTS[$IMAGE_NAME]}
    echo -e "${BLUE}>>> Building $IMAGE_NAME...${NC}"
    docker build -t "$IMAGE_NAME:latest" --build-arg BINARY="$BINARY_NAME" "$REPO_ROOT"
done

# 2. Import to MicroK8s
echo -e "${BLUE}>>> Importing images to MicroK8s...${NC}"
for IMAGE_NAME in "${!COMPONENTS[@]}"; do
    docker save "$IMAGE_NAME:latest" | microk8s images import -
done

# 3. Prepare Cluster
echo -e "${BLUE}>>> Preparing cluster CRDs...${NC}"
microk8s kubectl apply --server-side --force-conflicts -f - <<EOF
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: podlogs.monitoring.grafana.com
spec:
  group: monitoring.grafana.com
  names:
    kind: PodLogs
    listKind: PodLogsList
    plural: podlogs
    singular: podlog
  scope: Namespaced
  versions:
    - name: v1alpha1
      served: true
      storage: false
      schema:
        openAPIV3Schema:
          type: object
          x-kubernetes-preserve-unknown-fields: true
    - name: v1alpha2
      served: true
      storage: true
      schema:
        openAPIV3Schema:
          type: object
          x-kubernetes-preserve-unknown-fields: true
EOF

echo -e "${BLUE}>>> Generating self-signed TLS certificates for mTLS...${NC}"
CERT_DIR="$REPO_ROOT/tests/certs"
mkdir -p "$CERT_DIR"
if [ ! -f "$CERT_DIR/tls.crt" ]; then
    openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
        -keyout "$CERT_DIR/ca.key" -out "$CERT_DIR/ca.crt" \
        -subj "/CN=Stormchaser CA"

    openssl genrsa -out "$CERT_DIR/tls.key" 2048
    openssl req -new -key "$CERT_DIR/tls.key" \
        -subj "/CN=stormchaser" \
        -out "$CERT_DIR/tls.csr"

    openssl x509 -req -in "$CERT_DIR/tls.csr" \
        -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" \
        -CAcreateserial -out "$CERT_DIR/tls.crt" -days 365
    echo -e "${GREEN}>>> Certificates generated in $CERT_DIR${NC}"
fi

echo -e "${BLUE}>>> Creating namespace and TLS secrets...${NC}"
microk8s kubectl create namespace "$NAMESPACE" || true
microk8s kubectl create secret generic stormchaser-tls-certs \
  --namespace "$NAMESPACE" \
  --from-file=ca.crt="$CERT_DIR/ca.crt" \
  --from-file=tls.crt="$CERT_DIR/tls.crt" \
  --from-file=tls.key="$CERT_DIR/tls.key" \
  --dry-run=client -o yaml | microk8s kubectl apply -f -

# 4. Deploy via Helm
echo -e "${BLUE}>>> Deploying via Helm...${NC}"
cd "$REPO_ROOT/deploy/charts/stormchaser"
# Clean up existing subcharts to ensure local modifications are packed
rm -f charts/stormchaser-*.tgz
helm dependency update

helm upgrade --install stormchaser . \
  --namespace "$NAMESPACE" \
  --create-namespace \
  --skip-crds \
  --set "stormchaser-orchestration.api.image.repository=stormchaser-api" \
  --set "stormchaser-orchestration.api.image.tag=latest" \
  --set "stormchaser-orchestration.api.image.pullPolicy=Never" \
  --set "stormchaser-orchestration.engine.image.repository=stormchaser-engine" \
  --set "stormchaser-orchestration.engine.image.tag=latest" \
  --set "stormchaser-orchestration.engine.image.pullPolicy=Never" \
  --set "stormchaser-runner-k8s.image.repository=stormchaser-runner-k8s" \
  --set "stormchaser-runner-k8s.image.tag=latest" \
  --set "stormchaser-runner-k8s.image.pullPolicy=Never" \
  --set "global.agent.image.repository=stormchaser-agent" \
  --set "global.agent.image.tag=latest" \
  --set "global.agent.image.pullPolicy=Never"

echo -e "${BLUE}>>> Deploying Dex Identity Provider...${NC}"
if ! microk8s kubectl get secret dex-admin-secret -n "$NAMESPACE" >/dev/null 2>&1; then
    echo -e "${BLUE}>>> Generating random admin password for Dex...${NC}"
    DEX_PASSWORD=$(python3 -c 'import secrets; print(secrets.token_urlsafe(16))')
    DEX_HASH=$(python3 -c "from passlib.hash import bcrypt; print(bcrypt.hash('$DEX_PASSWORD'))")
    microk8s kubectl create secret generic dex-admin-secret \
        --namespace "$NAMESPACE" \
        --from-literal=password="$DEX_PASSWORD" \
        --from-literal=hash="$DEX_HASH"
    echo -e "${GREEN}>>> Generated Dex admin password and stored in dex-admin-secret.${NC}"
fi

microk8s kubectl apply -f "$REPO_ROOT/deploy/dex-k8s/"

echo -e "${GREEN}>>> Success! Stormchaser is deploying.${NC}"
