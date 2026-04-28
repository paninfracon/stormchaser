#!/bin/bash
set -e

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
CERT_DIR="${STORMCHASER_CERT_DIR:-$REPO_ROOT/tests/certs}"

mkdir -p "$CERT_DIR"

if [ ! -f "$CERT_DIR/tls.crt" ] || [ ! -f "$CERT_DIR/tls.key" ] || [ ! -f "$CERT_DIR/ca.crt" ]; then
    echo ">>> Generating self-signed TLS certificates for mTLS in $CERT_DIR..."

    # Generate CA
    openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
        -keyout "$CERT_DIR/ca.key" -out "$CERT_DIR/ca.crt" \
        -subj "/CN=Stormchaser CA"

    # Generate Server Key and CSR
    openssl genrsa -out "$CERT_DIR/tls.key" 2048
    openssl req -new -key "$CERT_DIR/tls.key" \
        -subj "/CN=stormchaser" \
        -out "$CERT_DIR/tls.csr"

    # Sign the Server CSR with the CA
    openssl x509 -req -in "$CERT_DIR/tls.csr" \
        -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" \
        -CAcreateserial -out "$CERT_DIR/tls.crt" -days 365

    echo ">>> Certificates generated successfully."
else
    echo ">>> Certificates already exist in $CERT_DIR."
fi
