#!/bin/bash
TOKEN=${STORMCHASER_TOKEN:-$1}
if [ -z "$TOKEN" ]; then
  echo "Error: Authentication token required. Set STORMCHASER_TOKEN or pass it as argument 1."
  exit 1
fi
cargo run -p stormchaser-cli -- --url http://localhost:3000/api/v1 --token "${TOKEN}" runs list
