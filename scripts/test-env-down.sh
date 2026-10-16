#!/bin/bash
set -e

source ./scripts/test-env.sh

echo "Tearing down isolated test infrastructure..."
# Compose checks the required settings even to tear down; the same placeholders as
# test-env-up.sh stand in for those the test stack does not use.
STORMCHASER_JWT_SECRET="${STORMCHASER_JWT_SECRET:-unused-by-the-test-stack}" \
  STORMCHASER_CLI_SECRET="${STORMCHASER_CLI_SECRET:-unused-by-the-test-stack}" \
  docker compose down -v
rm -f "/tmp/${COMPOSE_PROJECT_NAME}-env.ready" "$TEST_ENV_SECRETS"
