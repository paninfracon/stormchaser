#!/bin/bash
set -e

source ./scripts/test-env.sh

echo "Starting isolated test infrastructure..."
docker compose up -d --wait nats postgres opa dex s3

echo "Running migrations for test database..."
sqlx migrate run --database-url "$DATABASE_URL"
