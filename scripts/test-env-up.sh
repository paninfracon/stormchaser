#!/bin/bash
set -e

source ./scripts/test-env.sh

READY_FILE="/tmp/${COMPOSE_PROJECT_NAME}-env.ready"
if [ -f "$READY_FILE" ]; then
  echo "Isolated test infrastructure already initialized; skipping setup."
  exit 0
fi

echo "Starting isolated test infrastructure..."
# Recreates the NATS container if its passwords changed. Compose checks the required
# settings of every service, though only NATS and Postgres start; those of services that
# do not run get placeholders here, which go no further than this command.
STORMCHASER_JWT_SECRET="${STORMCHASER_JWT_SECRET:-unused-by-the-test-stack}" \
  STORMCHASER_CLI_SECRET="${STORMCHASER_CLI_SECRET:-unused-by-the-test-stack}" \
  docker compose up -d --wait nats postgres

echo "Running migrations for test database..."
sqlx migrate run --database-url "$DATABASE_URL"
touch "$READY_FILE"
