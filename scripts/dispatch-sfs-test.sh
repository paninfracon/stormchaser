#!/bin/bash
set -e

# Stormchaser SFS Parking Test Dispatcher
# 1. Ensures local S3 backend is registered
# 2. Dispatches the SFS parking test workflow

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
STORM_CLI="$REPO_ROOT/target/debug/stormchaser"

if [ ! -f "$STORM_CLI" ]; then
    echo "Building CLI..."
    cargo build -p stormchaser-cli
fi

echo ">>> Ensuring local storage is registered..."
TOKEN=${STORMCHASER_TOKEN:-$1}

if [ -z "$TOKEN" ]; then
  echo "Error: Authentication token required."
  echo "Please set STORMCHASER_TOKEN environment variable or pass it as the first argument."
  echo "You can obtain a token by running: stormchaser login"
  exit 1
fi

"$REPO_ROOT/scripts/register-local-s3.sh" "$TOKEN"

echo ">>> Dispatching SFS Parking Test..."
"$STORM_CLI" --token "$TOKEN" run "$REPO_ROOT/tests/sfs-parking.storm"
