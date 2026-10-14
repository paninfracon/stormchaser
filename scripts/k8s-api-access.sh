#!/bin/bash
# Sourced by the microk8s scripts to reach the API and the query service, which serve
# HTTPS with the certificate the chart generated. That certificate names the Services
# and localhost, not their cluster IPs, so they are reached through port-forwards on
# localhost, trusting the chart's CA. Sets API_URL and QUERY_URL, exports
# STORMCHASER_CA_CERT for the CLI, and stops the port-forwards on exit.

K8S_NAMESPACE="${K8S_NAMESPACE:-stormchaser}"
K8S_RELEASE_PREFIX="${K8S_RELEASE_PREFIX:-stormchaser-stormchaser-orchestration}"

STORMCHASER_CA_CERT=$(mktemp)
export STORMCHASER_CA_CERT
microk8s kubectl get secret -n "$K8S_NAMESPACE" stormchaser-tls-certs \
    -o jsonpath='{.data.ca\.crt}' | base64 -d > "$STORMCHASER_CA_CERT"

microk8s kubectl port-forward -n "$K8S_NAMESPACE" "svc/${K8S_RELEASE_PREFIX}-api" \
    "${PORT_FORWARD_API:-13000}:3000" > /dev/null 2>&1 &
API_FORWARD_PID=$!
microk8s kubectl port-forward -n "$K8S_NAMESPACE" "svc/${K8S_RELEASE_PREFIX}-query" \
    "${PORT_FORWARD_QUERY:-13001}:3001" > /dev/null 2>&1 &
QUERY_FORWARD_PID=$!
trap 'kill "$API_FORWARD_PID" "$QUERY_FORWARD_PID" 2> /dev/null; rm -f "$STORMCHASER_CA_CERT"' EXIT

API_URL="https://localhost:${PORT_FORWARD_API:-13000}"
QUERY_URL="https://localhost:${PORT_FORWARD_QUERY:-13001}"
export CURL_CA_BUNDLE="$STORMCHASER_CA_CERT"

# Wait for the forwards to accept connections
for _ in {1..20}; do
    if curl -s -o /dev/null "$API_URL/healthz" && curl -s -o /dev/null "$QUERY_URL/healthz"; then
        break
    fi
    sleep 0.5
done
