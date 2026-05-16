#!/bin/bash
set -e

source ./scripts/test-env.sh

echo "Starting isolated test infrastructure..."
docker compose up -d --wait nats postgres

echo "Running migrations for test database..."
sqlx migrate run --database-url "$DATABASE_URL"
