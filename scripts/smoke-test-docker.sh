#!/bin/bash
set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
cd "$REPO_ROOT"

# Load environment variables if present
if [ -f "$REPO_ROOT/.env" ]; then
    set -a
    source "$REPO_ROOT/.env"
    set +a
fi

echo -e "${BLUE}>>> Generating test token...${NC}"
TOKEN=$(python3 "$REPO_ROOT/scripts/generate_dev_token.py")

PORT_API=${PORT_API:-3000}
PORT_LOKI=${PORT_LOKI:-3100}
PORT_TEMPO=${PORT_TEMPO:-3200}
PORT_S3=${PORT_S3:-9000}
PORT_GRAFANA=${PORT_GRAFANA:-3002}
PORT_PROMETHEUS=${PORT_PROMETHEUS:-9090}

API_URL="http://localhost:${PORT_API}"

echo -e "${BLUE}>>> Checking Service Health (API, Engine, Loki, Tempo, MinIO, Grafana, Prometheus)...${NC}"

ALL_READY=false
for i in {1..24}; do
    API_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/health" || echo "000")
    LOKI_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_LOKI}/ready" || echo "000")
    TEMPO_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_TEMPO}/ready" || echo "000")
    MINIO_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_S3}/minio/health/live" || echo "000")
    GRAFANA_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_GRAFANA}/api/health" || echo "000")
    PROM_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_PROMETHEUS}/-/ready" || echo "000")

    ENGINE_STATUS="000"
    if docker ps --format '{{.Names}}' | grep -q "orchestration-engine"; then
        ENGINE_STATUS="200"
    fi

    if [ "$API_STATUS" = "200" ] && [ "$LOKI_STATUS" = "200" ] && [ "$TEMPO_STATUS" = "200" ] && [ "$MINIO_STATUS" = "200" ] && [ "$GRAFANA_STATUS" = "200" ] && [ "$PROM_STATUS" = "200" ] && [ "$ENGINE_STATUS" = "200" ]; then
        ALL_READY=true
        echo -e "${GREEN}>>> All core services are healthy!${NC}"
        break
    fi
    echo "Waiting for services to become ready (API: $API_STATUS, Engine: $ENGINE_STATUS, Loki: $LOKI_STATUS, Tempo: $TEMPO_STATUS, MinIO: $MINIO_STATUS, Grafana: $GRAFANA_STATUS, Prom: $PROM_STATUS) (attempt $i/24)..."
    sleep 5
done

if [ "$ALL_READY" != true ]; then
    echo -e "${RED}>>> Service health check failed! The environment may not be fully up.${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Verifying OIDC Login Flow...${NC}"
OIDC_TOKEN=$(PORT_DEX="${PORT_DEX:-5556}" PORT_API="${PORT_API:-3000}" python3 "$REPO_ROOT/scripts/get_token.py")
if [ -z "$OIDC_TOKEN" ]; then
    echo -e "${RED}>>> OIDC token acquisition failed!${NC}"
    exit 1
fi
echo -e "${GREEN}>>> OIDC token acquired successfully.${NC}"

echo -e "${BLUE}>>> Verifying OIDC token authorizes API calls...${NC}"
OIDC_AUTH_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer $OIDC_TOKEN" \
    "$API_URL/api/v1/storage-backends")
if [ "$OIDC_AUTH_STATUS" = "200" ]; then
    echo -e "${GREEN}>>> OIDC auth flow verified successfully!${NC}"
else
    echo -e "${RED}>>> OIDC token rejected by API (HTTP $OIDC_AUTH_STATUS)!${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Verifying Storage Backend Registration...${NC}"
STORAGE_CHECK=$(curl -s -H "Authorization: Bearer $TOKEN" "$API_URL/api/v1/storage-backends")
if echo "$STORAGE_CHECK" | grep -q "local-minio"; then
    echo -e "${GREEN}>>> Storage backend successfully registered!${NC}"
else
    echo -e "${RED}>>> Storage backend registration missing!${NC}"
    echo "$STORAGE_CHECK"
    exit 1
fi

echo -e "${BLUE}>>> Running test workflow (tests/hello-world.storm)...${NC}"
RUN_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" run "$REPO_ROOT/tests/hello-world.storm")
RUN_ID=$(echo "$RUN_JSON" | grep -oP '(?<="run_id": ")[^"]*')

if [ -z "$RUN_ID" ]; then
    echo -e "${RED}>>> Failed to extract Run ID from CLI output: $RUN_JSON${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Workflow started with ID: $RUN_ID${NC}"

# Poll for completion
STATUS="started"
for i in {1..30}; do
    STATUS_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID" || echo "{}")
    # Using grep instead of jq to avoid depending on jq
    if echo "$STATUS_JSON" | grep -q '"status": "succeeded"'; then
        STATUS="succeeded"
        break
    elif echo "$STATUS_JSON" | grep -q '"status": "failed"'; then
        STATUS="failed"
        break
    fi
    echo "Waiting for workflow to complete (attempt $i/30)..."
    sleep 5
done

if [ "$STATUS" = "succeeded" ]; then
    echo -e "${GREEN}>>> Workflow execution successful!${NC}"
else
    echo -e "${RED}>>> Workflow execution failed or timed out!${NC}"
    cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID"
    exit 1
fi

# Verify traces are in tempo
echo -e "${BLUE}>>> Verifying Tempo Traces...${NC}"
TEMPO_URL="http://localhost:${PORT_TEMPO}"

TRACE_FOUND=false
for i in {1..10}; do
    TRACE_RESPONSE=$(curl -s "$TEMPO_URL/api/search?tags=service.name=stormchaser-engine&limit=1")
    if echo "$TRACE_RESPONSE" | grep -q "traceID"; then
        TRACE_FOUND=true
        break
    fi
    echo "Waiting for traces to be flushed (attempt $i/10)..."
    sleep 3
done

if [ "$TRACE_FOUND" = true ]; then
    echo -e "${GREEN}>>> Traces successfully recorded in Tempo!${NC}"
else
    echo -e "${RED}>>> Warning: No traces found in Tempo!${NC}"
    echo "$TRACE_RESPONSE"
fi

echo -e "${GREEN}>>> Full Stack Smoke Test Completed Successfully!${NC}"
