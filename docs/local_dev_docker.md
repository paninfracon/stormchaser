# Local Development with Docker

This document describes how to set up a complete Stormchaser development
environment using only Docker and Docker Compose, independent of Kubernetes.
This is the fastest way to get started and is ideal for rapid iteration on
core engine logic or testing features locally.

## Components

The Docker-only setup includes:

* **Stormchaser API & Engine**: Core orchestration services.
* **PostgreSQL**: Database for state and configuration.
* **NATS JetStream**: Message broker and persistent event store for service
  communication and task queuing.
* **Dex (OIDC)**: Identity provider for authentication.
* **Open Policy Agent (OPA)**: Authorization engine.
* **Stormchaser Docker Runner**: Executes container-based steps locally on
  your host Docker daemon.
* **MinIO (S3)**: S3-compatible storage for artifacts (SFS).
* **Grafana Loki & Alloy**: Centralized log collection and processing.

## Quick Start

### 1. Start the Environment

Run the unified setup script in `docker` mode. This script automatically
handles TLS certificate generation, service startup, and initial storage
backend registration.

```bash
./scripts/setup.sh --mode docker
```

If you ever need to tear down the environment and start fresh, simply run:

```bash
./scripts/setup.sh --cleanup --mode docker
```

### 2. Configure Host Resolution

The local OIDC issuer is configured as `http://dex:5556/dex`. To allow your
local CLI and browser to properly resolve this address during the OAuth2 flow,
add the following entry to your `/etc/hosts` file (this requires `sudo`
privileges):

```text
127.0.0.1 dex
```

### 3. Login (CLI & TUI)

Once the environment is up and `dex` is mapped in your hosts file, you must
authenticate. Both the CLI and TUI support opening a web browser to securely
authenticate via the local Dex Identity Provider.

**Default Local Credentials:**

* **Username:** `stormchaser-admin@paninfracon.net`
* **Password:** `password`

**Using the CLI:**

```bash
stormchaser login --issuer http://dex:5556/dex --client-id stormchaser-cli
```

This command automatically opens your default browser. Enter the credentials
above.

**Using the TUI:**

```bash
cargo run -p stormchaser-tui -- --url http://localhost:3000
```

When the Terminal User Interface opens, press `Enter` to launch the browser
login flow.

## Common Infrastructure Endpoints

| Service | Local URL | Description |
| :--- | :--- | :--- |
| **API** | `http://localhost:3000` | Main entry point for CLI and TUI. |
| **S3 Console** | `http://localhost:9001` | MinIO UI (User: `stormchaser`) |
| **Dex** | `http://localhost:5556/dex` | OIDC Issuer. |
| **Loki** | `http://localhost:3100` | Log aggregation endpoint. |
| **Tempo** | `http://localhost:3200` | Tracing endpoint. |
| **Prometheus**| `http://localhost:9090` | Metrics endpoint. |

## Configuration Details

### mTLS Certificates

The setup script generates self-signed certificates in `.tmp/certs/`. These
are mounted into the API and Engine containers to facilitate secure
communication and mTLS between components.

### OPA Policy

A permissive OPA policy is located at `deploy/opa/policy.rego`. This policy
allows all requests by default in the local environment, making it easy to
test APIs without worrying about complex RBAC rules.

### Log Collection

Grafana Alloy is configured to watch the Docker socket and automatically
scrape logs from all containers managed by Stormchaser. It labels them with
the internal `run_id` and `step_id`, allowing you to seamlessly retrieve logs
via the API and CLI (`stormchaser runs logs <RUN_ID>`).
