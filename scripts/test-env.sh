#!/bin/bash

export COMPOSE_PROJECT_NAME=stormchaser-test
export PORT_DB=5433
export PORT_NATS=4223
export PORT_NATS_MGT=8223
export PORT_OPA=8182
export PORT_DEX=5557
export PORT_LOKI=3101
export PORT_PROMETHEUS=9091
export PORT_GRAFANA=3003
export PORT_S3=9002
export PORT_S3_CONS=9003
export PORT_REG=32001
export PORT_API=3004
export PORT_QUERY=3005

export STORMCHASER_TEST_DB_PASSWORD=${STORMCHASER_TEST_DB_PASSWORD:-stormchaser_test_password}
export STORMCHASER_DEV_PASSWORD=${STORMCHASER_TEST_DB_PASSWORD}

# Integration tests point HTTP steps at mock servers on loopback, which the SSRF guard
# blocks unless the host is allow-listed.
export STORMCHASER_SSRF_ALLOWED_HOSTS="127.0.0.1,localhost"

# NATS requires authentication; tests connect as the engine user, which is unrestricted,
# because they play every role (queueing runs, dispatching tasks, reporting on steps).
# The users' passwords are random, generated the first time and kept, readable by you
# alone, for as long as the test stack lives (test-env-down.sh removes them). A fixed
# password would let anyone who can reach the published port use the broker. Values
# already in the environment, as CI sets them, take precedence.
TEST_ENV_SECRETS_DIR="${XDG_RUNTIME_DIR:-$HOME/.cache}/stormchaser"
TEST_ENV_SECRETS="$TEST_ENV_SECRETS_DIR/${COMPOSE_PROJECT_NAME}.env"
if [ ! -f "$TEST_ENV_SECRETS" ]; then
  (
    umask 077
    mkdir -p "$TEST_ENV_SECRETS_DIR"
    for user in CONTROL RUNNER API QUERY; do
      # Starting with a letter: NATS reads the password as an unquoted config value
      echo "TEST_NATS_${user}_PASSWORD=n$(head -c 24 /dev/urandom | od -An -tx1 | tr -d ' \n')"
    done > "$TEST_ENV_SECRETS"
  )
  # A stack started with other passwords is set up again with these
  rm -f "/tmp/${COMPOSE_PROJECT_NAME}-env.ready"
fi
# shellcheck source=/dev/null
. "$TEST_ENV_SECRETS"
export STORMCHASER_NATS_PASSWORD=${STORMCHASER_NATS_PASSWORD:-$TEST_NATS_CONTROL_PASSWORD}
export STORMCHASER_NATS_RUNNER_PASSWORD=${STORMCHASER_NATS_RUNNER_PASSWORD:-$TEST_NATS_RUNNER_PASSWORD}
export STORMCHASER_NATS_API_PASSWORD=${STORMCHASER_NATS_API_PASSWORD:-$TEST_NATS_API_PASSWORD}
export STORMCHASER_NATS_QUERY_PASSWORD=${STORMCHASER_NATS_QUERY_PASSWORD:-$TEST_NATS_QUERY_PASSWORD}

export DATABASE_URL="postgres://stormchaser:${STORMCHASER_DEV_PASSWORD}@127.0.0.1:${PORT_DB}/stormchaser"
export NATS_URL="nats://127.0.0.1:${PORT_NATS}"
export NATS_USER=stormchaser
export NATS_PASSWORD="${STORMCHASER_NATS_PASSWORD}"
export OPA_URL="http://127.0.0.1:${PORT_OPA}/v1/data/stormchaser/allow"
