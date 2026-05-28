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
PORT_QUERY=${PORT_QUERY:-3001}
PORT_LOKI=${PORT_LOKI:-3100}
PORT_TEMPO=${PORT_TEMPO:-3200}
PORT_S3=${PORT_S3:-9000}
PORT_GRAFANA=${PORT_GRAFANA:-3002}
PORT_PROMETHEUS=${PORT_PROMETHEUS:-9090}

API_URL="http://localhost:${PORT_API}"
QUERY_URL="http://localhost:${PORT_QUERY}"

echo -e "${BLUE}>>> Checking Service Health (API, Query, Engine, Loki, Tempo, MinIO, Grafana, Prometheus)...${NC}"

ALL_READY=false
for i in {1..24}; do
    API_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/health" || echo "000")
    QUERY_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$QUERY_URL/healthz" || echo "000")
    LOKI_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_LOKI}/ready" || echo "000")
    TEMPO_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_TEMPO}/ready" || echo "000")
    MINIO_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_S3}/minio/health/live" || echo "000")
    GRAFANA_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_GRAFANA}/api/health" || echo "000")
    PROM_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "http://localhost:${PORT_PROMETHEUS}/-/ready" || echo "000")

    ENGINE_STATUS="000"
    if docker ps --format '{{.Names}}' | grep -q "orchestration-engine"; then
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
    echo "Please ensure you have run 'docker-compose up -d' before running this script."
    exit 1
fi

echo -e "${BLUE}>>> Generating load-test.storm...${NC}"
# Use python to generate a list of 500 integers as a JSON string
ITERATE_ARRAY=$(python3 -c "import json; print(json.dumps(list(range(500))))")

cat << 'STORM_EOF' > "$REPO_ROOT/tests/load-test.storm"
workflow "e2e_load_test" {
  description = "A full local development e2e load test to verify quotas and concurrency."

  quotas {
    max_concurrency = 50
  }

  steps {
    step "process" "RunContainer" {
      image = "alpine:latest"
      command = ["/bin/sh", "-c"]
      args = ["echo 'Running iteration ${step.iterate.value}'"]
      strategy {
        iterate = "__ITERATE_ARRAY__"
      }
    }
  }
}
STORM_EOF
sed -i.bak "s|__ITERATE_ARRAY__|$ITERATE_ARRAY|" "$REPO_ROOT/tests/load-test.storm"
rm -f "$REPO_ROOT/tests/load-test.storm.bak"
echo -e "${GREEN}>>> Generated tests/load-test.storm${NC}"


echo -e "${BLUE}>>> Running load test workflow...${NC}"
RUN_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" run "$REPO_ROOT/tests/load-test.storm")
RUN_ID=$(echo "$RUN_JSON" | grep -oP '(?<="run_id": ")[^"]*')

if [ -z "$RUN_ID" ]; then
    echo -e "${RED}>>> Failed to extract Run ID from CLI output: $RUN_JSON${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Workflow started with ID: $RUN_ID${NC}"

# Poll for completion. 500 tasks, 50 concurrency = 10 batches.
# Alpine container takes 1-2s. Total should be ~30-60 seconds.
# We'll allow up to 300 seconds (60 attempts * 5s).
STATUS="started"
for i in {1..60}; do
    STATUS_JSON=$(cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID" 2>&1)
    CLI_EXIT=$?
    if [ $CLI_EXIT -ne 0 ]; then
        echo -e "${RED}>>> 'runs get' command failed (exit $CLI_EXIT):${NC}"
        echo "$STATUS_JSON"
        exit 1
    fi

    if echo "$STATUS_JSON" | grep -q '"status": "succeeded"'; then
        STATUS="succeeded"
        break
    elif echo "$STATUS_JSON" | grep -q '"status": "failed"'; then
        STATUS="failed"
        break
    fi
    echo "Waiting for workflow to complete (attempt $i/60)..."
    sleep 5
done

if [ "$STATUS" = "succeeded" ]; then
    echo -e "${GREEN}>>> Workflow execution successful! Load test completed.${NC}"
else
    echo -e "${RED}>>> Workflow execution failed or timed out!${NC}"
    cargo run -q -p stormchaser-cli -- --url "$API_URL" --token "$TOKEN" runs get "$RUN_ID"
    exit 1
fi
