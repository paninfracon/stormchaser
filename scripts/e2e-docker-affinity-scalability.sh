#!/bin/bash
set -e

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m'

echo -e "${BLUE}>>> Generating test token...${NC}"
TEST_TOKEN=$(python3 "$REPO_ROOT/scripts/generate_dev_token.py")

echo -e "${BLUE}>>> Checking Service Health...${NC}"
until curl -s http://localhost:3000/api/health > /dev/null; do
  sleep 1
done
echo -e "${GREEN}>>> Core services are healthy!${NC}"

echo -e "${BLUE}>>> Scaling Docker Runners to 2 instances...${NC}"
# Scale up docker-runner to test scalability
export COMPOSE_PROJECT_NAME=stormchaser-docker
docker compose up -d --scale docker-runner=2 docker-runner

# Give runners a few seconds to register
sleep 5

echo -e "${BLUE}>>> Launching affinity & scalability workflow...${NC}"
STORM_FILE="$REPO_ROOT/tests/docker-affinity-scalability.storm"

PAYLOAD=$(jq -n --arg dsl "$(cat "$STORM_FILE")" '{dsl: $dsl, inputs: {}}')
RESPONSE=$(curl -s -X POST http://localhost:3000/api/v1/runs/direct \
  -H "Authorization: Bearer $TEST_TOKEN" \
  -H "Content-Type: application/json" \
  -d "$PAYLOAD")

RUN_ID=$(echo "$RESPONSE" | jq -r '.run_id')
if [ "$RUN_ID" == "null" ] || [ -z "$RUN_ID" ]; then
    echo -e "${RED}>>> Failed to spawn workflow!${NC}"
    echo "$RESPONSE"
    exit 1
fi

echo -e "${GREEN}>>> Successfully spawned workflow $RUN_ID.${NC}"

echo ">>> Polling for completion of workflow..."
POSTGRES_CONTAINER=$(docker ps --format '{{.Names}}' | grep postgres | head -n 1)

for attempt in {1..60}; do
    STATUS=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "
        SELECT status FROM workflow_runs WHERE id = '$RUN_ID'
        UNION ALL
        SELECT status FROM archived_workflow_runs WHERE id = '$RUN_ID'
    " | xargs)

    echo "Attempt $attempt/60 - Status: $STATUS"

    if [ "$STATUS" == "succeeded" ]; then
        echo -e "${GREEN}>>> Workflow execution successful!${NC}"

        # Verify step count
        COMPLETED_STEPS=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "
            SELECT count(*) FROM archived_step_instances WHERE run_id = '$RUN_ID' AND status = 'succeeded'
        " | xargs)

        if [ "$COMPLETED_STEPS" -eq 5 ]; then
             echo -e "${GREEN}>>> Success! Found $COMPLETED_STEPS completed steps.${NC}"
             export COMPOSE_PROJECT_NAME=stormchaser-docker
             docker compose up -d --scale docker-runner=1 docker-runner
             exit 0
        else
             echo -e "${RED}>>> Failed! Expected 5 completed steps, got $COMPLETED_STEPS.${NC}"
             export COMPOSE_PROJECT_NAME=stormchaser-docker
             docker compose up -d --scale docker-runner=1 docker-runner
             exit 1
        fi
    elif [ "$STATUS" == "failed" ]; then
        echo -e "${RED}>>> Workflow failed!${NC}"
        exit 1
    fi

    sleep 2
done

echo -e "${RED}>>> Workflow timed out!${NC}"
exit 1
