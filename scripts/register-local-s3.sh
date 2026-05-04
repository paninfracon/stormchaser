#!/bin/bash
set -e

# Stormchaser Local Storage Registration Script
# Registers the local Minio container as the default SFS backend.

API_URL=${STORMCHASER_API_URL:-"http://localhost:3000"}
TOKEN=${STORMCHASER_TOKEN:-$1}

if [ -z "$TOKEN" ]; then
  echo "Error: Authentication token required."
  echo "Please set STORMCHASER_TOKEN environment variable or pass it as the first argument."
  echo "You can obtain a token by running: stormchaser login"
  exit 1
fi

echo ">>> Cleaning up existing 'local-minio' if present..."
# Get the ID of the existing backend
OLD_ID=$(curl -s -H "Authorization: Bearer $TOKEN" "$API_URL/api/v1/storage-backends" | jq -r '.[] | select(.name=="local-minio") | .id')
if [ -n "$OLD_ID" ] && [ "$OLD_ID" != "null" ]; then
  curl -s -X DELETE -H "Authorization: Bearer $TOKEN" "$API_URL/api/v1/storage-backends/$OLD_ID"
  echo "Deleted old backend $OLD_ID"
fi

echo ">>> Ensuring Minio bucket 'stormchaser-sfs' exists..."
export AWS_ACCESS_KEY_ID="stormchaser"
if [ -z "$STORMCHASER_MINIO_PASSWORD" ]; then
    echo -e "\033[0;31mError: STORMCHASER_MINIO_PASSWORD is not set.\033[0m" >&2
    exit 1
fi
export AWS_SECRET_ACCESS_KEY="$STORMCHASER_MINIO_PASSWORD"
export AWS_DEFAULT_REGION="us-east-1"
AWS_ENDPOINT=${AWS_ENDPOINT:-"http://localhost:9000"}
aws --endpoint-url "$AWS_ENDPOINT" s3 mb "s3://stormchaser-sfs" 2>/dev/null || true

echo ">>> Registering local Minio as default SFS backend..."
S3_ENDPOINT=${S3_ENDPOINT:-"http://s3:9000"}
curl -s -X POST "$API_URL/api/v1/storage-backends" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d "$(jq -n \
    --arg secret "$STORMCHASER_MINIO_PASSWORD" \
    --arg endpoint "$S3_ENDPOINT" \
    '{
      "name": "local-minio",
      "description": "Local Minio S3-compatible storage for SFS parking",
      "backend_type": "s3",
      "is_default_sfs": true,
      "config": {
        "endpoint": $endpoint,
        "bucket": "stormchaser-sfs",
        "region": "us-east-1",
        "access_key": "stormchaser",
        "secret_key": $secret,
        "force_path_style": true
      }
    }')" | jq .

echo -e "\n>>> Storage backend registered successfully."
