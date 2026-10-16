#!/bin/bash
set -e

# As per GEMINI.md, we use cargo-llvm-cov for coverage and set SQL_OFFLINE=true
# to avoid database connection issues during tests involving sqlx.

if [ -f ".env" ]; then
    set -a
    source .env
    set +a
    # The tests sign their tokens with the API's debug key; the deployment's own key in
    # .env would make the API refuse them
    unset STORMCHASER_JWT_SECRET
fi

source ./scripts/test-env.sh
./scripts/test-env-up.sh

export API_RATE_LIMIT_BURST_SIZE=1000
export API_RATE_LIMIT_PER_SECOND=1000

echo "Running cargo test coverage..."
SQL_OFFLINE=true cargo llvm-cov --summary-only
