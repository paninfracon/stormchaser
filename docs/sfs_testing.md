# Testing Shared File System (SFS)

Stormchaser's Shared File System (SFS) lets steps share data across execution boundaries. This document explains the architecture and how to verify the implementation.

## Architecture Overview

The SFS works on a **Park/Unpark** model:

1. **Unpark (Pre-execution):** Before a step starts, a helper (an init container on Kubernetes, an `unpark-<id>` container on Docker) downloads a `.tar.gz` bundle from S3-compatible storage using a pre-signed GET URL and extracts it to a local volume. It verifies the **SHA-256 hash** against the last known state.
2. **Execution:** The main container runs with the local volume mounted, performing its tasks.
3. **Park (Post-execution):** A helper outside the step's own container (the `stormchaser-agent` sidecar in the pod on Kubernetes, a `park-<id>` container on Docker) tars the volume, calculates a new **SHA-256 hash**, and uploads the bundle back to storage using a pre-signed PUT URL. The step's container never holds the URL.
   It skips parking and artifacts when the step's command failed.
4. **Persistence:** The new hash is reported back to the Orchestration Engine and stored in the `run_storage_states` table for the next step to verify.

## Local Test Environment

### Prerequisites

- **Docker Compose:** Running Postgres, NATS, and Minio.
- **MicroK8s:** Running the K8s Runner.
- **Stormchaser CLI:** Built with `cargo build -p stormchaser-cli`; the binary is `target/debug/stormchaser`.

### Automatic Setup

Use the setup script in hybrid mode (Compose for the control plane, the
Kubernetes runner in MicroK8s):

```bash
./scripts/setup.sh --mode hybrid
```

It:

1. Starts the infrastructure with Docker Compose.
2. Builds the `stormchaser-agent` and `stormchaser-runner-k8s` images and imports them into MicroK8s.
3. Deploys the Kubernetes runner with Helm.
4. Runs `scripts/register-local-s3.sh`, which creates the `stormchaser-sfs` bucket in MinIO and registers it as the default SFS backend through the API, using the host's address so pods can reach it.

## Running the End-to-End Test

A dedicated test workflow and dispatcher are provided to verify the full lifecycle.

### 1. The Test Workflow (`tests/sfs-parking.storm`)

This workflow consists of two steps:

- **`create_file`**: Writes a string to `/data/test.txt` inside the `workspace` storage.
- **`verify_file`**: Reads `/data/test.txt` from the same `workspace` and greps for the string.

### 2. Dispatching the Test

Run the dispatcher script:

```bash
export STORMCHASER_TOKEN="<your-cli-token>"
./scripts/dispatch-sfs-test.sh
```

It registers the local S3 backend again (harmless if it exists) and runs
`tests/sfs-parking.storm` with the CLI.

### 3. Verifying Results

Use the CLI to monitor the run:

```bash
# The token is read from the environment, not the command line, where other users can read it
export STORMCHASER_TOKEN="<your-cli-token>"

# List runs and pick the test's run ID from the JSON output
./target/debug/stormchaser runs list

# Inspect the run details
./target/debug/stormchaser runs get "$RUN_ID"
```

## Observability & Debugging

### Checking Agent Logs

The agent logs its SFS operations (hashing, uploads, downloads); look for
`Parking storage`. On Kubernetes the agent is the `stormchaser-agent` container
in the step's pod:

```bash
microk8s kubectl logs -n <step namespace> <pod> -c stormchaser-agent
```

With the bundled Alloy configuration from the telemetry chart, the same logs
reach Loki with a `container` label:

```bash
curl -G -s "http://localhost:3100/loki/api/v1/query_range" \
  --data-urlencode 'query={container="stormchaser-agent"} |= "Parking storage"'
```

On Docker the helper containers are named `park-<id>` and `unpark-<id>`.

### Manual Hash Verification

You can manually check the hashes stored in the database:

```bash
docker compose -p stormchaser-hybrid exec postgres psql -U stormchaser -d stormchaser \
  -c "SELECT storage_name, last_hash FROM run_storage_states WHERE run_id = '$RUN_ID';"
```

### Common Failure Modes

- **411 Length Required:** An old `stormchaser-agent` image that did not send `Content-Length`. Rebuild and re-import the agent image.
- **404 Not Found (S3):** Usually means the bucket `stormchaser-sfs` doesn't exist. Run `scripts/register-local-s3.sh` to fix.
- **Bad Address / DNS Error:** Ensure the storage backend is registered using the host's Docker IP (`172.17.0.1`) instead of `localhost` or `s3`, so the K8s pod can reach it.
