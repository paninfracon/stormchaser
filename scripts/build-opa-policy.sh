#!/bin/bash
# Compiles the shipped policy (deploy/opa/policy.rego) to WebAssembly for the services to
# build in. They evaluate it when no OPA_URL or OPA_WASM_PATH is configured.
#
# Run after changing the policy, and commit the result. Needs the opa CLI.
set -euo pipefail

cd "$(dirname "$0")/.."

out=crates/stormchaser-opa/policy/stormchaser.wasm
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

opa test deploy/opa
opa build --target wasm --entrypoint stormchaser/allow --output "$work/bundle.tar.gz" deploy/opa/policy.rego
tar -xzf "$work/bundle.tar.gz" -C "$work" --no-same-permissions /policy.wasm 2>/dev/null
install -m 644 "$work/policy.wasm" "$out"
# A test compares this with the policy, so a change built in by nobody fails it
sha256sum deploy/opa/policy.rego | cut -d' ' -f1 >"$out.source-sha256"
echo "Wrote $out"
