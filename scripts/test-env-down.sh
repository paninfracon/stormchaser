#!/bin/bash
set -e

source ./scripts/test-env.sh

echo "Tearing down isolated test infrastructure..."
docker compose down -v
