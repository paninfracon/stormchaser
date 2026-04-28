#!/bin/bash
set -e

# Dispatches a one-off workflow to Stormchaser via NATS with parameters

GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

STORM_FILE="tests/hello-parameter.storm"
RUN_ID=$(uuidgen | tr '[:upper:]' '[:lower:]')
NATS_URL=${NATS_URL:-"nats://localhost:4222"}

echo -e "${BLUE}>>> Dispatching workflow from $STORM_FILE...${NC}"
echo -e "${BLUE}>>> Run ID: $RUN_ID${NC}"

# Read the DSL content and escape for JSON
DSL_CONTENT=$(cat "$STORM_FILE")

# Construct the payload with inputs
PAYLOAD=$(jq -n \
  --arg run_id "$RUN_ID" \
  --arg dsl "$DSL_CONTENT" \
  --arg user "$(whoami)" \
  '{run_id: $run_id, dsl: $dsl, initiating_user: $user, inputs: {target: "Gemini CLI"}}')

# Dispatch via NATS (prefer local CLI, fallback to Docker)
if command -v nats &> /dev/null; then
    echo "$PAYLOAD" | nats pub -s "$NATS_URL" stormchaser.run.direct
else
    echo -e "${BLUE}>>> NATS CLI not found locally, using Docker...${NC}"
    # Use argument instead of stdin for reliability in docker
    docker run --rm --network host natsio/nats-box nats pub -s "$NATS_URL" stormchaser.run.direct "$PAYLOAD"
fi

echo -e "${GREEN}>>> Workflow dispatched!${NC}"
echo -e "${BLUE}Monitor logs with:${NC}"
echo "  docker compose logs -f orchestration-engine"
echo "  kubectl --kubeconfig .tmp/kubeconfig logs -l app.kubernetes.io/instance=stormchaser-runner"
