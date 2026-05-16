#!/bin/bash
set -e

# Source test environment variables and bring up isolated infrastructure
source ./scripts/test-env.sh
./scripts/test-env-up.sh

echo "Running Unit Tests..."
export SQLX_OFFLINE=true

TEST_ARGS=""
# Collect all tests that are NOT integration tests
for test_file in crates/*/tests/*.rs; do
  if [ -f "$test_file" ]; then
    test_name=$(basename "$test_file" .rs)
    if [[ ! "$test_name" == integration_* ]]; then
      TEST_ARGS="$TEST_ARGS --test $test_name"
    fi
  fi
done

# Run library tests, binary tests, and non-integration tests
eval "cargo test --workspace --lib --bins $TEST_ARGS \"\$@\""
