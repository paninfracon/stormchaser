#!/bin/bash
# The token is passed in the environment: an argument would be readable by every user
# on the host, in the process list, for as long as the command runs.
if [ -z "${STORMCHASER_TOKEN:-}" ]; then
  echo "Error: Authentication token required. Set STORMCHASER_TOKEN."
  exit 1
fi
export STORMCHASER_TOKEN
cargo run -p stormchaser-cli -- --url http://localhost:3000 runs list
