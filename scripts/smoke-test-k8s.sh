#!/bin/bash
set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
NC='\033[0m' # No Color

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
cd "$REPO_ROOT"

echo -e "${BLUE}>>> Generating test token...${NC}"
TOKEN=$(python3 "$REPO_ROOT/scripts/generate_dev_token.py")

echo -e "${BLUE}>>> Discovering API and Query endpoints...${NC}"
API_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-stormchaser-orchestration-api -o jsonpath='{.spec.clusterIP}')
API_URL="http://${API_IP}:3000"

QUERY_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-stormchaser-orchestration-query -o jsonpath='{.spec.clusterIP}')
QUERY_URL="http://${QUERY_IP}:3001"

echo -e "${BLUE}>>> Checking Service Health (API, Query, Engine, Loki, Tempo, MinIO, Grafana, Prometheus)...${NC}"
LOKI_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-loki -o jsonpath='{.spec.clusterIP}')
TEMPO_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-tempo -o jsonpath='{.spec.clusterIP}')
MINIO_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-minio -o jsonpath='{.spec.clusterIP}')
GRAFANA_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-grafana -o jsonpath='{.spec.clusterIP}')
PROM_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-prometheus-server -o jsonpath='{.spec.clusterIP}')

ALL_READY=false
for i in {1..24}; do
    API_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/health" || echo "000")
    QUERY_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$QUERY_URL/healthz" || echo "000")
    LOKI_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://$LOKI_IP:3100/ready" || echo "000")
    TEMPO_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://$TEMPO_IP:3100/ready" || echo "000")
    MINIO_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://$MINIO_IP:9000/minio/health/live" || echo "000")
    GRAFANA_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://$GRAFANA_IP:80/api/health" || echo "000")
    PROM_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://$PROM_IP:80/-/ready" || echo "000")

    ENGINE_STATUS="000"
    if microk8s kubectl get pods -n stormchaser -l app.kubernetes.io/component=engine -o jsonpath='{.items[*].status.phase}' | grep -q "Running"; then
        ENGINE_STATUS="200"
    fi

    if [ "$API_STATUS" = "200" ] && [ "$QUERY_STATUS" = "200" ] && [ "$LOKI_STATUS" = "200" ] && [ "$TEMPO_STATUS" = "200" ] && [ "$MINIO_STATUS" = "200" ] && [ "$GRAFANA_STATUS" = "200" ] && [ "$PROM_STATUS" = "200" ] && [ "$ENGINE_STATUS" = "200" ]; then
        ALL_READY=true
        echo -e "${GREEN}>>> All core services are healthy!${NC}"
        break
    fi
    echo "Waiting for services to become ready (API: $API_STATUS, Query: $QUERY_STATUS, Engine: $ENGINE_STATUS, Loki: $LOKI_STATUS, Tempo: $TEMPO_STATUS, MinIO: $MINIO_STATUS, Grafana: $GRAFANA_STATUS, Prom: $PROM_STATUS) (attempt $i/24)..."
    sleep 5
done

if [ "$ALL_READY" != true ]; then
    echo -e "${RED}>>> Service health check failed! The environment may not be fully up.${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Verifying OIDC Login Flow...${NC}"
DEX_IP=$(microk8s kubectl get svc -n stormchaser dex -o jsonpath='{.spec.clusterIP}')
OIDC_TOKEN=$(HOST_DEX="${DEX_IP}" PORT_DEX="5556" HOST_API="${API_IP}" PORT_API="3000" python3 "$REPO_ROOT/scripts/get_token.py")
if [ -z "$OIDC_TOKEN" ]; then
    echo -e "${RED}>>> OIDC token acquisition failed!${NC}"
    exit 1
fi
echo -e "${GREEN}>>> OIDC token acquired successfully.${NC}"

echo -e "${BLUE}>>> Verifying OIDC token authorizes API calls...${NC}"
OIDC_AUTH_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "Authorization: Bearer $OIDC_TOKEN" \
    "$API_URL/api/v1/connections")
if [ "$OIDC_AUTH_STATUS" = "200" ]; then
    echo -e "${GREEN}>>> OIDC auth flow verified successfully!${NC}"
else
    echo -e "${RED}>>> OIDC token rejected by API (HTTP $OIDC_AUTH_STATUS)!${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Verifying Connection Registration...${NC}"
CONNECTIONS_CHECK=$(curl -s -H "Authorization: Bearer $TOKEN" "$API_URL/api/v1/connections")
if echo "$CONNECTIONS_CHECK" | grep -q "local-minio"; then
    echo -e "${GREEN}>>> Connection successfully registered!${NC}"
else
    echo -e "${RED}>>> Connection registration missing!${NC}"
    echo "$CONNECTIONS_CHECK"
    exit 1
fi

echo -e "${BLUE}>>> Running test workflow (tests/hello-world.storm)...${NC}"
RUN_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" run "$REPO_ROOT/tests/hello-world.storm" --input target="smoke-test-k8s")
RUN_ID=$(echo "$RUN_JSON" | grep -oP '(?<="run_id": ")[^"]*')

if [ -z "$RUN_ID" ]; then
    echo -e "${RED}>>> Failed to extract Run ID from CLI output: $RUN_JSON${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Workflow started with ID: $RUN_ID${NC}"

# Poll for completion
STATUS="started"
for i in {1..30}; do
    STATUS_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID")
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
TEMPO_IP=$(microk8s kubectl get svc -n stormchaser stormchaser-tempo -o jsonpath='{.spec.clusterIP}')
TEMPO_URL="http://${TEMPO_IP}:3100"

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
