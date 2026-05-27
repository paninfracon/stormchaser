#!/bin/bash
set -e

REPO_ROOT=$(git rev-parse --show-toplevel)
cd "$REPO_ROOT"

GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}>>> Generating test token...${NC}"
TEST_TOKEN=$(python3 "$REPO_ROOT/scripts/generate_dev_token.py")

echo -e "${BLUE}>>> Checking Service Health...${NC}"
until curl -s http://localhost:3000/api/health > /dev/null; do
  sleep 1
done
echo -e "${GREEN}>>> Core services are healthy!${NC}"

NUM_STEPS=500
echo ">>> Generating many-parallel-steps.storm with $NUM_STEPS parallel steps and 1 join step..."

STORM_FILE="$REPO_ROOT/tests/many-parallel-steps.storm"
echo 'workflow "many_parallel_steps" {' > "$STORM_FILE"
echo '  description = "A workflow with many parallel steps joined at the end."' >> "$STORM_FILE"
echo '  steps {' >> "$STORM_FILE"

# Generate parallel steps
DEPENDENCIES=""
for i in $(seq 1 $NUM_STEPS); do
  echo "    step \"step_$i\" \"JQ\" {" >> "$STORM_FILE"
  echo "      program = \".text\"" >> "$STORM_FILE"
  echo "      input = { text = \"Hello World $i!\" }" >> "$STORM_FILE"
  echo "    }" >> "$STORM_FILE"

  if [ -z "$DEPENDENCIES" ]; then
    DEPENDENCIES="\"step_$i\""
  else
    DEPENDENCIES="$DEPENDENCIES, \"step_$i\""
  fi
done

# Generate join step
echo "    step \"join_step\" \"JQ\" {" >> "$STORM_FILE"
echo "      program = \".text\"" >> "$STORM_FILE"
echo "      input = { text = \"All parallel steps completed!\" }" >> "$STORM_FILE"
echo "      depends_on = [$DEPENDENCIES]" >> "$STORM_FILE"
echo "    }" >> "$STORM_FILE"

echo '  }' >> "$STORM_FILE"
echo '}' >> "$STORM_FILE"

echo ">>> Cleaning up previous workflow runs in local DB..."
POSTGRES_CONTAINER=$(docker ps --format '{{.Names}}' | grep postgres | head -n 1)
docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -c "TRUNCATE workflow_runs CASCADE" > /dev/null 2>&1
docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -c "TRUNCATE archived_workflow_runs CASCADE" > /dev/null 2>&1

echo -e "${BLUE}>>> Launching workflow with $NUM_STEPS parallel steps via API...${NC}"

START_TIME=$(date +%s)

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

# Expect NUM_STEPS + 1 completed steps
EXPECTED_STEPS=$((NUM_STEPS + 1))

for attempt in {1..120}; do
    # We check if the workflow reached a terminal state
    STATUS=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "
        SELECT status FROM workflow_runs WHERE id = '$RUN_ID'
        UNION ALL
        SELECT status FROM archived_workflow_runs WHERE id = '$RUN_ID'
    " | xargs)

    echo "Attempt $attempt/120 - Status: $STATUS"

    if [ "$STATUS" == "succeeded" ]; then
        END_TIME=$(date +%s)
        DURATION=$((END_TIME - START_TIME))
        echo -e "${GREEN}>>> Workflow execution successful! Workflow completed in $DURATION seconds.${NC}"

        # Verify that all steps were completed successfully
        COMPLETED_STEPS=$(docker exec "$POSTGRES_CONTAINER" psql -U stormchaser -d stormchaser -t -c "
            SELECT count(*) FROM archived_step_instances WHERE run_id = '$RUN_ID' AND status = 'succeeded'
        " | xargs)

        echo ">>> Completed Steps: $COMPLETED_STEPS / $EXPECTED_STEPS"

        if [ "$COMPLETED_STEPS" -eq "$EXPECTED_STEPS" ]; then
            echo -e "${GREEN}>>> Success! All $EXPECTED_STEPS steps executed correctly.${NC}"
            exit 0
        else
            echo -e "${RED}>>> Failed! Expected $EXPECTED_STEPS completed steps, but got $COMPLETED_STEPS.${NC}"
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
