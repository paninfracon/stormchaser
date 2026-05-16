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

export DATABASE_URL="postgres://stormchaser:${STORMCHASER_DEV_PASSWORD}@127.0.0.1:${PORT_DB}/stormchaser"
export NATS_URL="nats://127.0.0.1:${PORT_NATS}"
export OPA_URL="http://127.0.0.1:${PORT_OPA}/v1/data/stormchaser/allow"
