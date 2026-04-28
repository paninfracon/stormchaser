#!/bin/bash
set -e

REPO_ROOT=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &> /dev/null && pwd)
CERT_DIR="${STORMCHASER_CERT_DIR:-$REPO_ROOT/tests/certs}"

mkdir -p "$CERT_DIR"

if [ ! -f "$CERT_DIR/tls.crt" ] || [ ! -f "$CERT_DIR/tls.key" ] || [ ! -f "$CERT_DIR/ca.crt" ]; then
    echo ">>> Generating self-signed TLS certificates (v3) for mTLS in $CERT_DIR..."

    # Create OpenSSL config for v3 extensions
    cat > "$CERT_DIR/openssl.cnf" <<EOF
[req]
distinguished_name = req_distinguished_name
x509_extensions = v3_ca
prompt = no

[req_distinguished_name]
CN = Stormchaser CA

[v3_ca]
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always,issuer
basicConstraints = critical, CA:true
keyUsage = critical, digitalSignature, cRLSign, keyCertSign

[v3_req]
basicConstraints = CA:FALSE
keyUsage = nonRepudiation, digitalSignature, keyEncipherment
subjectAltName = @alt_names

[alt_names]
DNS.1 = localhost
DNS.2 = stormchaser
IP.1 = 127.0.0.1
EOF

    # Generate CA
    openssl req -x509 -nodes -days 365 -newkey rsa:2048 \
        -config "$CERT_DIR/openssl.cnf" \
        -keyout "$CERT_DIR/ca.key" -out "$CERT_DIR/ca.crt"

    # Generate Server Key and CSR
    openssl genrsa -out "$CERT_DIR/tls.key" 2048
    openssl req -new -key "$CERT_DIR/tls.key" \
        -subj "/CN=stormchaser" \
        -out "$CERT_DIR/tls.csr"

    # Sign the Server CSR with the CA using v3 extensions
    openssl x509 -req -in "$CERT_DIR/tls.csr" \
        -CA "$CERT_DIR/ca.crt" -CAkey "$CERT_DIR/ca.key" \
        -CAcreateserial -out "$CERT_DIR/tls.crt" -days 365 \
        -extfile "$CERT_DIR/openssl.cnf" -extensions v3_req

    rm "$CERT_DIR/openssl.cnf"
    echo ">>> X.509 v3 Certificates generated successfully."
else
    echo ">>> Certificates already exist in $CERT_DIR."
fi
