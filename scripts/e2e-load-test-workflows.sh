#!/bin/bash
set -e

GREEN='\033[0;32m'
BLUE='\033[0;34m'
RED='\033[0;31m'
YELLOW='\033[0;33m'
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

echo -e "${BLUE}>>> Checking Service Health...${NC}"

ALL_READY=false
for i in {1..24}; do
    API_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/health" || echo "000")
    QUERY_STATUS=$(curl -s -o /dev/null -w "%{http_code}" "$QUERY_URL/healthz" || echo "000")

    ENGINE_STATUS="000"
    if docker ps --format '{{.Names}}' | grep -q "orchestration-engine"; then
        ENGINE_STATUS="200"
    fi

    if [ "$API_STATUS" = "200" ] && [ "$QUERY_STATUS" = "200" ] && [ "$ENGINE_STATUS" = "200" ]; then
        ALL_READY=true
        echo -e "${GREEN}>>> Core services are healthy!${NC}"
        break
    fi
    echo "Waiting for services to become ready (API: $API_STATUS, Query: $QUERY_STATUS, Engine: $ENGINE_STATUS) (attempt $i/24)..."
    sleep 5
done

if [ "$ALL_READY" != true ]; then
    echo -e "${RED}>>> Service health check failed! The environment may not be fully up.${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Generating load-test.storm...${NC}"
cat << 'EOF' > "$REPO_ROOT/tests/load-test.storm"
workflow "hello_world" {
  description = "A simple hello world workflow."
  steps {
    step "generate" "JQ" {
      program = ".text"
      input = { text = "Hello World!" }
    }
  }
}
EOF

echo ">>> Cleaning up previous workflow runs in local DB..."
POSTGRES_CONTAINER=$(docker ps --format '{{.Names}}' | grep postgres | head -n 1)
if [ "${STORMCHASER_E2E_RESET_ALL_RUNS:-0}" = "1" ]; then
    echo ">>> STORMCHASER_E2E_RESET_ALL_RUNS=1 set; truncating all workflow runs."
    docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -c "TRUNCATE workflow_runs CASCADE" > /dev/null 2>&1
    docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -c "TRUNCATE archived_workflow_runs CASCADE" > /dev/null 2>&1
else
    echo ">>> Deleting only hello_world runs. Set STORMCHASER_E2E_RESET_ALL_RUNS=1 to truncate all runs."
    docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -c "DELETE FROM workflow_runs WHERE workflow_name = 'hello_world'" > /dev/null 2>&1
    docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -c "DELETE FROM archived_workflow_runs WHERE workflow_name = 'hello_world'" > /dev/null 2>&1
fi

NUM_WORKFLOWS=500
echo -e "${BLUE}>>> Launching $NUM_WORKFLOWS workflows concurrently via API...${NC}"

START_TIME=$(date +%s)

# We use raw curl instead of the CLI to avoid the cargo boot overhead (~0.5s per run).
# The cli's `run` command reads the file, parses it, and posts the JSON.
# We will do a multipart post or just construct the payload.
# Since CLI `run` reads the file as a string and puts it in `dsl` field, we'll do the same.

PAYLOAD_FILE=$(mktemp)
cat <<EOF > "$PAYLOAD_FILE"
{
  "dsl": $(jq -Rs . < "$REPO_ROOT/tests/load-test.storm"),
  "inputs": {}
}
EOF

# Array to store Run IDs
RUN_IDS=()

echo "Submitting 500 POST requests in the background..."
# Store responses in a temporary directory
RESP_DIR=$(mktemp -d)

for i in $(seq 1 $NUM_WORKFLOWS); do
    curl -s -X POST "$API_URL/api/v1/runs/direct" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d @"$PAYLOAD_FILE" \
        -o "$RESP_DIR/resp_${i}.json" &
done

# Wait for all background curls to finish
wait

echo -e "${GREEN}>>> All submissions completed.${NC}"

# Extract Run IDs from responses
for i in $(seq 1 $NUM_WORKFLOWS); do
    if [ -f "$RESP_DIR/resp_${i}.json" ]; then
        RID=$(jq -r '.run_id' "$RESP_DIR/resp_${i}.json")
        if [ "$RID" != "null" ] && [ -n "$RID" ]; then
            RUN_IDS+=("$RID")
        else
            echo -e "${YELLOW}Warning: Failed to extract Run ID from response $i: $(cat "$RESP_DIR/resp_${i}.json")${NC}"
        fi
    fi
done

SUCCESSFUL_SUBMISSIONS=${#RUN_IDS[@]}
echo -e "${BLUE}>>> Successfully extracted $SUCCESSFUL_SUBMISSIONS Run IDs.${NC}"

if [ "$SUCCESSFUL_SUBMISSIONS" -lt "$NUM_WORKFLOWS" ]; then
    echo -e "${RED}>>> Failed to submit all workflows. Only submitted $SUCCESSFUL_SUBMISSIONS.${NC}"
    exit 1
fi

echo -e "${BLUE}>>> Polling for completion of $SUCCESSFUL_SUBMISSIONS workflows...${NC}"

# Polling loop. We'll poll up to 90 times (5 mins).
# It's faster to list all runs and count statuses.
STATUS="started"
for attempt in {1..90}; do
    # Fast polling using database count to bypass 100 limit on API
    POSTGRES_CONTAINER=$(docker ps --format '{{.Names}}' | grep postgres | head -n 1)

    SUCCEEDED=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT (SELECT count(*) FROM workflow_runs WHERE status = 'succeeded' AND workflow_name = 'hello_world') + (SELECT count(*) FROM archived_workflow_runs WHERE status = 'succeeded' AND workflow_name = 'hello_world')" | xargs)
    FAILED=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT (SELECT count(*) FROM workflow_runs WHERE status = 'failed' AND workflow_name = 'hello_world') + (SELECT count(*) FROM archived_workflow_runs WHERE status = 'failed' AND workflow_name = 'hello_world')" | xargs)
    RUNNING=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT count(*) FROM workflow_runs WHERE status = 'running' AND workflow_name = 'hello_world'" | xargs)
    QUEUED=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT count(*) FROM workflow_runs WHERE status = 'queued' AND workflow_name = 'hello_world'" | xargs)

    echo "Attempt $attempt/90 - Succeeded: $SUCCEEDED, Failed: $FAILED, Running: $RUNNING, Queued: $QUEUED"

    if [ "$FAILED" -gt 0 ]; then
        echo -e "${RED}>>> One or more workflows failed!${NC}"
        STATUS="failed"
        break
    fi

    # We only care about the ones we submitted. But if there are 500 succeeded, we are good.
    if [ "$SUCCEEDED" -ge "$SUCCESSFUL_SUBMISSIONS" ]; then
        STATUS="succeeded"
        break
    fi

    sleep 5
done

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))

rm -f "$PAYLOAD_FILE"
rm -rf "$RESP_DIR"

if [ "$STATUS" = "succeeded" ]; then
    echo -e "${GREEN}>>> Workflow execution successful! All $SUCCESSFUL_SUBMISSIONS workflows completed in $DURATION seconds.${NC}"
else
    FINAL_RUNNING=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT count(*) FROM workflow_runs WHERE status = 'running' AND workflow_name = 'hello_world'" | xargs)
    FINAL_QUEUED=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT count(*) FROM workflow_runs WHERE status = 'queued' AND workflow_name = 'hello_world'" | xargs)
    STUCK_RUN_IDS=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "SELECT id FROM workflow_runs WHERE workflow_name = 'hello_world' AND status IN ('queued', 'running') ORDER BY created_at ASC LIMIT 10" | xargs)
    echo -e "${RED}>>> Workflow execution failed or timed out!${NC}"
    echo "Remaining runs - Running: $FINAL_RUNNING, Queued: $FINAL_QUEUED"
    if [ -n "$STUCK_RUN_IDS" ]; then
        echo "Sample stuck run IDs: $STUCK_RUN_IDS"
    fi
    exit 1
fi
