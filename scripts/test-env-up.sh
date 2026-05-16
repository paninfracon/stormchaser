#!/bin/bash
set -e

source ./scripts/test-env.sh

READY_FILE="/tmp/${COMPOSE_PROJECT_NAME}-env.ready"
if [ -f "$READY_FILE" ]; then
  echo "Isolated test infrastructure already initialized; skipping setup."
  exit 0
fi

echo "Starting isolated test infrastructure..."
docker compose up -d --wait nats postgres

echo "Running migrations for test database..."
sqlx migrate run --database-url "$DATABASE_URL"
touch "$READY_FILE"
