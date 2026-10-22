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
MINIO_USER=$(microk8s kubectl get secret -n stormchaser stormchaser-minio -o jsonpath='{.data.root-user}' | base64 -d)
MINIO_PASSWORD=$(microk8s kubectl get secret -n stormchaser stormchaser-minio -o jsonpath='{.data.root-password}' | base64 -d)
export AWS_ACCESS_KEY_ID="$MINIO_USER"
export AWS_SECRET_ACCESS_KEY="$MINIO_PASSWORD"
export AWS_DEFAULT_REGION="us-east-1"

# Ensure bucket exists
aws --endpoint-url "http://${MINIO_IP}:9000" s3 mb "s3://stormchaser-sfs" 2>/dev/null || true
aws --endpoint-url "http://${MINIO_IP}:9000" s3api put-bucket-policy --bucket stormchaser-sfs --policy '{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":"*","Action":["s3:GetObject"],"Resource":["arn:aws:s3:::stormchaser-sfs/*"]}]}'
aws --endpoint-url "http://${MINIO_IP}:9000" s3 cp "$TEMP_TAR" "s3://stormchaser-sfs/dogfood/repo.tar.gz"

REPO_URL="http://stormchaser-minio.stormchaser.svc.cluster.local:9000/stormchaser-sfs/dogfood/repo.tar.gz"

echo -e "${BLUE}>>> Launching dogfood workflow...${NC}"
# shellcheck source=scripts/k8s-api-access.sh
source "$REPO_ROOT/scripts/k8s-api-access.sh"

echo -e "${BLUE}>>> Registering SFS backend...${NC}"
# The MinIO credentials reach jq through its environment and curl through stdin
MINIO_USER="$MINIO_USER" MINIO_PASSWORD="$MINIO_PASSWORD" jq -n '{
    "name": "local-minio",
    "description": "Local Minio S3-compatible storage for SFS parking",
    "connection_type": "s3",
    "is_default_sfs": true,
    "config": {
      "endpoint": "http://stormchaser-minio.stormchaser.svc.cluster.local:9000",
      "bucket": "stormchaser-sfs",
      "region": "us-east-1",
      "access_key": $ENV.MINIO_USER,
      "secret_key": $ENV.MINIO_PASSWORD,
      "force_path_style": true
    }
  }' |
  curl -s -X POST "$API_URL/api/v1/connections" \
    -H @<(printf 'Authorization: Bearer %s\n' "$TOKEN") \
    -H "Content-Type: application/json" \
    --data-binary @- > /dev/null

# The database password: an argument, or STORMCHASER_DEV_PASSWORD, or else the one in
# the cluster
DB_PASSWORD="${1:-$STORMCHASER_DEV_PASSWORD}"
if [ -z "$DB_PASSWORD" ]; then
    DB_PASSWORD=$(microk8s kubectl get secret -n stormchaser stormchaser-postgresql -o jsonpath='{.data.password}' 2>/dev/null | base64 -d)
fi
if [ -z "$DB_PASSWORD" ]; then
    echo -e "${RED}Error: no database password: pass one, set STORMCHASER_DEV_PASSWORD, or install the umbrella chart.${NC}" >&2
    exit 1
fi
# The bundled NATS requires authentication; the umbrella chart generates this Secret.
NATS_PASSWORD=$(microk8s kubectl get secret -n stormchaser stormchaser-nats-control -o jsonpath='{.data.password}' | base64 -d)

KUBECONFIG_DATA=$(microk8s config | sed 's|https://127.0.0.1:16443|https://kubernetes.default.svc:443|g' | sed 's|certificate-authority-data:.*|insecure-skip-tls-verify: true|g')

echo -e "${BLUE}>>> Triggering CLI to launch workflow...${NC}"
# Use the CLI to run the workflow and parse the run ID
# The token goes through the environment and the secret inputs through file descriptors,
# so that none of them appears in the process list.
export STORMCHASER_TOKEN="$TOKEN"
RUN_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" run tests/dogfood-k8s.storm \
    --input repo_url="$REPO_URL" \
    --input kubeconfig=@<(printf '%s' "$KUBECONFIG_DATA") \
    --input db_password=@<(printf '%s' "$DB_PASSWORD") \
    --input nats_password=@<(printf '%s' "$NATS_PASSWORD"))
RUN_ID=$(echo "$RUN_JSON" | grep -oP '(?<="run_id": ")[^"]*')

if [ -z "$RUN_ID" ]; then
    echo -e "${RED}>>> Failed to extract Run ID from CLI output: $RUN_JSON${NC}"
    exit 1
fi

echo -e "${BLUE}Run ID:${NC} $RUN_ID"

# Poll for completion
STATUS="started"
# A release build and the tests from scratch take well over ten minutes
for i in {1..180}; do
    # A poll that fails is a run the engine has not created yet
    STATUS_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" runs get "$RUN_ID" 2>&1) || true
    RUN_STATUS=$(echo "$STATUS_JSON" | jq -r '.detail.status' 2>/dev/null || true)

    if [ "$RUN_STATUS" = "succeeded" ]; then
        STATUS="succeeded"
        break
    elif [ "$RUN_STATUS" = "failed" ] || [ "$RUN_STATUS" = "error" ]; then
        STATUS="failed"
        break
    fi
    echo "Waiting for workflow to complete (attempt $i/180)..."
    sleep 10
done

if [ "$STATUS" = "succeeded" ]; then
    echo -e "${GREEN}>>> Dogfood workflow execution successful!${NC}"
else
    echo -e "${RED}>>> Dogfood workflow execution failed or timed out!${NC}"
    cargo run -q -p stormchaser-cli -- --url "$API_URL" runs get "$RUN_ID"
    exit 1
fi
