#!/bin/bash
set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)

echo -e "${BLUE}>>> Generating test token...${NC}"
TOKEN=$(python3 "$REPO_ROOT/scripts/generate_dev_token.py")

echo -e "${BLUE}>>> Creating repository tarball...${NC}"
TEMP_TAR="/tmp/stormchaser-dogfood.tar.gz"
(cd "$REPO_ROOT" && { git ls-files -z; } | tar -czf "$TEMP_TAR" --null -T -)

echo -e "${BLUE}>>> Uploading tarball to MinIO...${NC}"
MINIO_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-minio -o jsonpath='{.spec.clusterIP}')
MINIO_PASSWORD=$(microk8s kubectl get secret -n stormchaser stormchaser-minio -o jsonpath='{.data.root-password}' | base64 -d)
export AWS_ACCESS_KEY_ID="stormchaser"
export AWS_SECRET_ACCESS_KEY="$MINIO_PASSWORD"
export AWS_DEFAULT_REGION="us-east-1"

# Ensure bucket exists
aws --endpoint-url "http://${MINIO_IP}:9000" s3 mb "s3://stormchaser-sfs" 2>/dev/null || true
aws --endpoint-url "http://${MINIO_IP}:9000" s3api put-bucket-policy --bucket stormchaser-sfs --policy '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::stormchaser-sfs/*"]}]}'
aws --endpoint-url "http://${MINIO_IP}:9000" s3 cp "$TEMP_TAR" "s3://stormchaser-sfs/dogfood/repo.tar.gz"

REPO_URL="http://stormchaser-minio.stormchaser.svc.cluster.local:9000/stormchaser-sfs/dogfood/repo.tar.gz"

echo -e "${BLUE}>>> Launching dogfood workflow...${NC}"
API_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-stormchaser-orchestration-api -o jsonpath='{.spec.clusterIP}')
API_URL="http://${API_IP}:3000"

echo -e "${BLUE}>>> Registering SFS backend...${NC}"
curl -s -X POST "$API_URL/api/v1/storage-backends" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "local-minio",
    "description": "Local Minio S3-compatible storage for SFS parking",
    "connection_type": "s3",
    "is_default_sfs": true,
    "config": {
      "endpoint": "http://stormchaser-minio.stormchaser.svc.cluster.local:9000",
      "bucket": "stormchaser-sfs",
      "region": "us-east-1",
      "access_key": "stormchaser",
      "secret_key": "'"$MINIO_PASSWORD"'",
      "force_path_style": true
    }
  }' > /dev/null

if [ -z "$STORMCHASER_DEV_PASSWORD" ] && [ -z "$1" ]; then
    echo -e "${RED}Error: STORMCHASER_DEV_PASSWORD is not set and no password was passed as argument.${NC}" >&2
    exit 1
fi
DB_PASSWORD="${1:-$STORMCHASER_DEV_PASSWORD}"

KUBECONFIG_DATA=$(microk8s config | sed 's|https://127.0.0.1:16443|https://kubernetes.default.svc:443|g' | sed 's|certificate-authority-data:.*|insecure-skip-tls-verify: true|g')

echo -e "${BLUE}>>> Triggering CLI to launch workflow...${NC}"
# Use the CLI to run the workflow and parse the run ID
RUN_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" run tests/dogfood-k8s.storm --input repo_url="$REPO_URL" --input kubeconfig="$KUBECONFIG_DATA" --input db_password="$DB_PASSWORD")
RUN_ID=$(echo "$RUN_JSON" | grep -oP '(?<="run_id": ")[^"]*')

if [ -z "$RUN_ID" ]; then
    echo -e "${RED}>>> Failed to extract Run ID from CLI output: $RUN_JSON${NC}"
    exit 1
fi

echo -e "${BLUE}Run ID:${NC} $RUN_ID"

# Poll for completion
STATUS="started"
for i in {1..60}; do
    STATUS_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID")
    RUN_STATUS=$(echo "$STATUS_JSON" | jq -r '.detail.status')

    if [ "$RUN_STATUS" = "succeeded" ]; then
        STATUS="succeeded"
        break
    elif [ "$RUN_STATUS" = "failed" ] || [ "$RUN_STATUS" = "error" ]; then
        STATUS="failed"
        break
    fi
    echo "Waiting for workflow to complete (attempt $i/60)..."
    sleep 10
done

if [ "$STATUS" = "succeeded" ]; then
    echo -e "${GREEN}>>> Dogfood workflow execution successful!${NC}"
else
    echo -e "${RED}>>> Dogfood workflow execution failed or timed out!${NC}"
    cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID"
    exit 1
fi
