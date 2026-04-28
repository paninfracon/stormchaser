# Testing Shared File System (SFS)

Stormchaser implements a robust Shared File System (SFS) allowing steps to share data across execution boundaries. This document explains the architecture and how to verify the implementation.

## Architecture Overview

The SFS works on a **Park/Unpark** model:

1. **Unpark (Pre-execution):** Before a step starts, an Init Container downloads a `.tar.gz` bundle from S3-compatible storage using a pre-signed GET URL and extracts it to a local volume. It verifies the **SHA-256 hash** against the last known state.
2. **Execution:** The main container runs with the local volume mounted, performing its tasks.
3. **Park (Post-execution):** A command wrapper tars the volume, calculates a new **SHA-256 hash**, and uploads the bundle back to storage using a pre-signed PUT URL.
4. **Persistence:** The new hash is reported back to the Orchestration Engine and stored in the `run_storage_states` table for the next step to verify.

## Local Test Environment

### Prerequisites

- **Docker Compose:** Running Postgres, NATS, and Minio.
- **MicroK8s:** Running the K8s Runner.
- **Stormchaser CLI:** Built and available in `target/debug/stormchaser-cli`.

### Automatic Setup

The easiest way to set up the environment is using the local dev script:

```bash
./scripts/local-dev-up.sh
```

This script now automatically:

1. Starts all infrastructure.
2. Builds and imports the `stormchaser-agent` image into MicroK8s.
3. Ensures the `stormchaser-sfs` bucket exists in Minio.
4. Registers Minio as the default SFS backend via the API.

## Running the End-to-End Test

A dedicated test workflow and dispatcher are provided to verify the full lifecycle.

### 1. The Test Workflow (`tests/sfs-parking.storm`)

This workflow consists of two steps:

- **`create_file`**: Writes a string to `/data/test.txt` inside the `workspace` storage.
- **`verify_file`**: Reads `/data/test.txt` from the same `workspace` and greps for the string.

### 2. Dispatching the Test

Run the dispatcher script:

```bash
./scripts/dispatch-sfs-test.sh
```

### 3. Verifying Results

Use the CLI to monitor the run:

```bash
# Get a token
TOKEN="<your-cli-token>"

# List runs to find the latest ID
RUN_ID=$(./target/debug/stormchaser-cli --token "$TOKEN" runs list | jq -r '.[0].id')

# Inspect the run details
./target/debug/stormchaser-cli --token "$TOKEN" runs get "$RUN_ID"
```

## Observability & Debugging

### Checking Logs in Loki

All SFS operations (hashing, uploads, downloads) are logged by the `stormchaser-agent`. You can query these in Loki:

```bash
# Search for agent logs related to a specific run
curl -G -s "http://localhost:3100/loki/api/v1/query_range" \
  --data-urlencode 'query={container="worker"} |= "Parking storage"'
```

### Manual Hash Verification

You can manually check the hashes stored in the database:

```bash
docker compose exec postgres psql -U stormchaser -d stormchaser \
  -c "SELECT storage_name, last_hash FROM run_storage_states WHERE run_id = '$RUN_ID';"
```

### Common Failure Modes

- **411 Length Required:** Fixed. Ensure `stormchaser-agent` is using the latest build that includes `Content-Length` headers.
- **404 Not Found (S3):** Usually means the bucket `stormchaser-sfs` doesn't exist. Run `register-local-s3.sh` to fix.
- **Bad Address / DNS Error:** Ensure the storage backend is registered using the host's Docker IP (`172.17.0.1`) instead of `localhost` or `s3`, so the K8s pod can reach it.
