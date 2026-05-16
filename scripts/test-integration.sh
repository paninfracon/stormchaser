#!/bin/bash
set -e

# Source test environment variables and bring up isolated infrastructure
source ./scripts/test-env.sh
./scripts/test-env-up.sh

echo "Running Integration Tests..."
export SQLX_OFFLINE=true
export STORMCHASER_CERT_DIR=${STORMCHASER_CERT_DIR:-./tests/certs}

TEST_ARGS=""
for test_file in crates/*/tests/integration_*.rs; do
  if [ -f "$test_file" ]; then
    test_name=$(basename "$test_file" .rs)
    TEST_ARGS="$TEST_ARGS --test $test_name"
  fi
done

if [ -z "$TEST_ARGS" ]; then
  echo "No integration tests found."
  exit 0
fi

# We use eval to ensure TEST_ARGS expands properly as separate arguments
eval "cargo test --workspace $TEST_ARGS \"\$@\""
