# Local Development with Docker

This document describes how to set up a complete Stormchaser development
environment using only Docker and Docker Compose, independent of Kubernetes.
This is the fastest way to get started and is ideal for rapid iteration on
core engine logic or testing features locally.

## Components

The Docker-only setup includes:

* **Stormchaser API, query service, engine and web UI**: The control plane.
  They run without TLS here (`STORMCHASER_LISTEN_TLS=false`,
  `STORMCHASER_DB_SSL=false`), which a real deployment must not copy.
* **PostgreSQL**: Database for state and configuration. It is not encrypted
  at rest; do not store real credentials in this environment.
* **NATS JetStream**: Message broker and persistent event store for service
  communication and task queuing. It requires passwords, which `setup.sh`
  generates into `.env` (`STORMCHASER_NATS_PASSWORD` for the control plane,
  `STORMCHASER_NATS_RUNNER_PASSWORD` for runners), but has no TLS; see
  [NATS: Quickstart and Production](nats.md).
* **Dex (OIDC)**: Identity provider for authentication.
* **Open Policy Agent (OPA)**: Authorization engine.
* **Stormchaser Docker Runner**: Executes container-based steps locally on
  your host Docker daemon.
* **MinIO (S3)**: S3-compatible storage for artifacts (SFS).
* **Grafana Loki & Alloy**: Centralized log collection and processing.
* **Tempo, Prometheus and Grafana**: Traces, metrics and dashboards. These are
  for evaluation only; see [Observability](observability.md).
* **Registry**: An unauthenticated local OCI registry for artifacts.

## Quick Start

### 1. Start the Environment

Run the unified setup script in `docker` mode (the default). It generates
certificates in `tests/certs`, random passwords and keys in `.env`, and the
Dex configuration and persona passwords; builds and starts the compose
project `stormchaser-docker`; and registers MinIO as the default SFS storage.
You need Docker with Compose v2, `curl`, `openssl` and Python 3 with
`passlib[bcrypt]`.

```bash
./scripts/setup.sh --mode docker
```

If you ever need to tear down the environment and start fresh, run the
following. It removes the stack's volumes and regenerates `.env`:

```bash
./scripts/setup.sh --cleanup --mode docker
```

The script opts this environment in to two things a production install should
not do, because the dogfood workflows need them:

* `STORMCHASER_ALLOW_PRIVILEGED=true` on the Docker runner. Set it to `false`
  in your shell or `.env` to refuse privileged steps.
* Control-plane ports (PostgreSQL, NATS, OPA, Loki, MinIO, ...) published on
  all interfaces. Set `STORMCHASER_ISOLATE_STEPS=true` to publish them on
  loopback only, or `STORMCHASER_CONTROL_PLANE_BIND` to choose an address.

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

**Local Credentials:**

The setup script generates a random password for each Dex persona (`admin`,
`dev`, `ops` and `sec`, as `stormchaser-<persona>@paninfracon.net`) and writes
them to `deploy/dex/credentials.generated`, readable only by you:

```bash
cat deploy/dex/credentials.generated
```

**Using the CLI:**

```bash
stormchaser --url http://localhost:3000 login
```

This command opens your default browser. Enter the credentials above. The
token is stored in a profile for that URL (readable only by you) and used by
later commands; pass `--print-token` to print it instead. The CLI listens on
`http://localhost:8080/callback` for the browser to return; `--callback-port`
changes the port, which must match a redirect URL registered with the
identity provider.

**Using the TUI:**

```bash
cargo run -p stormchaser-tui -- --url http://localhost:3000
```

When the Terminal User Interface opens, press `Enter` to launch the browser
login flow.

## Common Infrastructure Endpoints

| Service | Local URL | Description |
| :--- | :--- | :--- |
| **API** | `http://localhost:3000` | Main entry point for CLI and TUI. Swagger UI at `/swagger-ui`. |
| **Query service** | `http://localhost:3001` | Fills in workflow input forms for the TUI and web UI. |
| **Web UI** | `http://localhost:3003` | Browser interface. |
| **Grafana** | `http://localhost:3002` | Dashboards; anonymous visitors are Viewers. |
| **S3 Console** | `http://localhost:9001` | MinIO UI (User: `stormchaser`) |
| **Dex** | `http://localhost:5556/dex` | OIDC Issuer. |
| **Loki** | `http://localhost:3100` | Log aggregation endpoint. |
| **Tempo** | `http://localhost:3200` | Tracing endpoint. |
| **Prometheus**| `http://localhost:9090` | Metrics endpoint. |
| **OPA** | `http://localhost:8181` | Policy server; its API only evaluates the policy. |
| **PostgreSQL** | `localhost:5432` | User `stormchaser`, password `STORMCHASER_DEV_PASSWORD` in `.env`. |
| **NATS** | `localhost:4222` | Users and passwords in `.env` (see [NATS](nats.md)). |

Each host port can be changed with the `PORT_*` variables in `docker-compose.yml`
(`PORT_API`, `PORT_QUERY`, `PORT_WEB`, `PORT_GRAFANA`, ...).

## Configuration Details

### TLS Certificates

The setup script generates a self-signed CA and a certificate for `localhost`
in `tests/certs/` (`scripts/generate-certs.sh`; `STORMCHASER_CERT_DIR` moves
it). The `certs` service copies them into a volume the services can read, and
the engine, API and query service mount it at `/etc/engine/certs`. The
listeners and the database connection run without TLS in this stack.

### OPA Policy

The OPA container serves the shipped policy, `deploy/opa/policy.rego`, with
the settings in `deploy/opa/settings.json`. It is the production policy: it
refuses anything it does not recognise, and users see and change only the
runs, webhooks, rules and schedules they created. To act on everything, add
your user to `admins` or a group to `admin_groups` in `settings.json` and
restart the `opa` service. See [OPA RBAC](opa-rbac.md).

### Log Collection

The Docker runner sends step containers' output to Loki through the Docker
`loki` logging driver, which the setup script installs as a Docker plugin.
Grafana Alloy also watches the Docker socket and ships container logs to Loki.
Logs are labelled with the `run_id` and `step_id`, so the API and CLI can read
them back (`stormchaser runs logs <RUN_ID>`).

### Performance Optimization (Host Bind Mounts)

For heavily data-intensive workflows running in this local environment, the Docker runner can be configured to bypass S3 upload/download operations using the `STORMCHASER_SFS_HOST_PATH` environment variable. By default, this is not enabled to ensure test parity with remote runners, but it can be activated in the `docker-compose.yml` to significantly speed up local execution times. See [SFS Docker Optimization](sfs_docker_optimization.md) for details.
