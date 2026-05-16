#!/bin/bash
set -e

source ./scripts/test-env.sh

echo "Tearing down isolated test infrastructure..."
docker compose down -v
rm -f "/tmp/${COMPOSE_PROJECT_NAME}-env.ready"
