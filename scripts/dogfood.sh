#!/bin/bash
set -e

# Stormchaser Dogfooding Script
# Generates a repository tarball, uploads it to SFS, and launches the dogfood workflow.

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)

# 1. Determine Host IP
HOST_IP=$(ip -4 addr show docker0 | grep -Po 'inet \K[\d.]+' || hostname -I | awk '{print $1}')
echo -e "${BLUE}>>> Detected Host IP: $HOST_IP${NC}"

# 2. Create Repository Tarball
echo -e "${BLUE}>>> Creating repository tarball (respecting .gitignore)...${NC}"
TEMP_TAR="/tmp/stormchaser-dogfood.tar.gz"
(cd "$REPO_ROOT" && {
    git ls-files -z
    if [ -d .tmp/ratatui-form ]; then
        find .tmp/ratatui-form -type f -print0
    fi
    if [ -d tests/certs ]; then
        find tests/certs -type f -print0
    fi
} | sort -z -u | tar -czf "$TEMP_TAR" --null -T -)

# 3. Upload to Local S3 (MinIO)
echo -e "${BLUE}>>> Uploading tarball to local S3...${NC}"
export AWS_ACCESS_KEY_ID="stormchaser"
if [ -z "$STORMCHASER_MINIO_PASSWORD" ]; then
    echo -e "${RED}Error: STORMCHASER_MINIO_PASSWORD is not set.${NC}" >&2
    exit 1
fi
export AWS_SECRET_ACCESS_KEY="$STORMCHASER_MINIO_PASSWORD"
export AWS_DEFAULT_REGION="us-east-1"

# Ensure bucket exists
aws --endpoint-url "http://localhost:9000" s3 mb "s3://stormchaser-sfs" 2>/dev/null || true

# Make bucket public for the download provisioner test
aws --endpoint-url "http://localhost:9000" s3api put-bucket-policy --bucket stormchaser-sfs --policy '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::stormchaser-sfs/*"]}]}'

# Upload the file
aws --endpoint-url "http://localhost:9000" s3 cp "$TEMP_TAR" "s3://stormchaser-sfs/dogfood/repo.tar.gz"

# Internal URL for the runner
REPO_URL="http://$HOST_IP:9000/stormchaser-sfs/dogfood/repo.tar.gz"

# 4. Launch Workflow via API
echo -e "${BLUE}>>> Launching dogfood workflow...${NC}"

wait_for_api() {
    local url="http://localhost:3000/healthz"
    local timeout=60
    local count=0
    until curl -s "$url" > /dev/null; do
        echo "Waiting for API to be ready..."
        sleep 2
        count=$((count + 2))
        if [ $count -ge $timeout ]; then
            echo -e "${RED}Error: Timeout waiting for API${NC}"
            exit 1
        fi
    done
}

wait_for_api

# Get OIDC token
TOKEN=${STORMCHASER_TOKEN:-$1}

if [ -z "$TOKEN" ]; then
  echo -e "${RED}Error: Authentication token required.${NC}"
  echo "Please set STORMCHASER_TOKEN environment variable or pass it as the first argument."
  echo "You can obtain a token by running: stormchaser login"
  exit 1
fi

KUBECONFIG="$REPO_ROOT/.tmp/kubeconfig"
KUBECONFIG_DATA=$(cat "$KUBECONFIG" 2>/dev/null | sed "s/127.0.0.1/$HOST_IP/g" || echo "")

DSL_CONTENT=$(cat "$REPO_ROOT/tests/dogfood.storm")
INPUTS=$(jq -n --arg url "$REPO_URL" --arg ip "$HOST_IP" --arg kubeconfig "$KUBECONFIG_DATA" \
  --arg port_api "${PORT_API:-3000}" \
  --arg port_db "${PORT_DB:-5432}" \
  --arg port_nats "${PORT_NATS:-4222}" \
  --arg port_loki "${PORT_LOKI:-3100}" \
  --arg port_dex "${PORT_DEX:-5556}" \
  --arg port_s3 "${PORT_S3:-9000}" \
  --arg port_reg "${PORT_REG:-32000}" \
  --arg port_opa "${PORT_OPA:-8181}" \
  --arg db_pwd "$STORMCHASER_DEV_PASSWORD" '{
  "repo_url": $url,
  "host_ip": $ip,
  "db_password": $db_pwd,
  "kubeconfig": $kubeconfig,
  "port_api": $port_api,
  "port_db": $port_db,
  "port_nats": $port_nats,
  "port_loki": $port_loki,
  "port_dex": $port_dex,
  "port_s3": $port_s3,
  "port_reg": $port_reg,
  "port_opa": $port_opa
}')

LAUNCH_RESP=$(curl -v -X POST "http://localhost:3000/api/v1/runs/direct" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "$(jq -n --arg dsl "$DSL_CONTENT" --argjson inputs "$INPUTS" \
    '{"dsl": $dsl, "inputs": $inputs, "initiating_user": "stormchaser-admin@paninfracon.net"}')")

RUN_ID=$(echo "$LAUNCH_RESP" | jq -r '.run_id' 2>/dev/null)

if [ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ]; then
  echo -e "${GREEN}>>> Workflow launched successfully!${NC}"
  echo -e "${BLUE}Run ID:${NC} $RUN_ID"
else
  echo -e "${RED}Error: Failed to launch workflow.${NC}"
  echo "Response: $LAUNCH_RESP"
  exit 1
fi
