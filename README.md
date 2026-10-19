# Stormchaser

[![CI](https://github.com/paninfracon/stormchaser/actions/workflows/ci.yml/badge.svg)](https://github.com/paninfracon/stormchaser/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/paninfracon/stormchaser/branch/trunk/graph/badge.svg)](https://codecov.io/gh/paninfracon/stormchaser)
[![License](https://img.shields.io/badge/license-MIT_OR_Apache--2.0_OR_CDLA--Permissive--2.0-blue.svg)](#license)
[![prek](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/j178/prek/master/docs/assets/badge-v0.json)](https://github.com/j178/prek)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit)](https://github.com/pre-commit/pre-commit)

A distributed workflow engine for event-driven and human-triggered workflows,
written in Rust. Workflows are graphs written in an HCL-based DSL. The engine
keeps state in PostgreSQL and passes events over NATS JetStream.

## Overview

Stormchaser runs workflows made of steps connected in a graph. It supports
long-running runs, human approvals, dynamic parallelism, retries, and steps
that run in containers on Docker or Kubernetes, or inside the engine itself
(HTTP calls, SQL, jq, templates, WASM, email and chat messages).

Workflows are written in an HCL-based Domain Specific Language (DSL) rather
than YAML.

## Crates

*For a breakdown of compilation features across these crates, see [Cargo Features](docs/cargo-features.md).*

| Crate | Version | Docs |
|---|---|---|
| `stormchaser-cli` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-cli.svg)](https://crates.io/crates/stormchaser-cli) | [![Docs.rs](https://docs.rs/stormchaser-cli/badge.svg)](https://docs.rs/stormchaser-cli) |
| `stormchaser-core` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-core.svg)](https://crates.io/crates/stormchaser-core) | [![Docs.rs](https://docs.rs/stormchaser-core/badge.svg)](https://docs.rs/stormchaser-core) |
| `stormchaser-engine` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-engine.svg)](https://crates.io/crates/stormchaser-engine) | [![Docs.rs](https://docs.rs/stormchaser-engine/badge.svg)](https://docs.rs/stormchaser-engine) |
| `stormchaser-model` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-model.svg)](https://crates.io/crates/stormchaser-model) | [![Docs.rs](https://docs.rs/stormchaser-model/badge.svg)](https://docs.rs/stormchaser-model) |
| `stormchaser-query` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-query.svg)](https://crates.io/crates/stormchaser-query) | [![Docs.rs](https://docs.rs/stormchaser-query/badge.svg)](https://docs.rs/stormchaser-query) |
| `stormchaser-api` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-api.svg)](https://crates.io/crates/stormchaser-api) | [![Docs.rs](https://docs.rs/stormchaser-api/badge.svg)](https://docs.rs/stormchaser-api) |
| `stormchaser-dsl` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-dsl.svg)](https://crates.io/crates/stormchaser-dsl) | [![Docs.rs](https://docs.rs/stormchaser-dsl/badge.svg)](https://docs.rs/stormchaser-dsl) |
| `stormchaser-opa` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-opa.svg)](https://crates.io/crates/stormchaser-opa) | [![Docs.rs](https://docs.rs/stormchaser-opa/badge.svg)](https://docs.rs/stormchaser-opa) |
| `stormchaser-tls` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-tls.svg)](https://crates.io/crates/stormchaser-tls) | [![Docs.rs](https://docs.rs/stormchaser-tls/badge.svg)](https://docs.rs/stormchaser-tls) |
| `stormchaser-tui` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-tui.svg)](https://crates.io/crates/stormchaser-tui) | [![Docs.rs](https://docs.rs/stormchaser-tui/badge.svg)](https://docs.rs/stormchaser-tui) |
| `stormchaser-web` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-web.svg)](https://crates.io/crates/stormchaser-web) | [![Docs.rs](https://docs.rs/stormchaser-web/badge.svg)](https://docs.rs/stormchaser-web) |
| `stormchaser-runner-docker` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-runner-docker.svg)](https://crates.io/crates/stormchaser-runner-docker) | [![Docs.rs](https://docs.rs/stormchaser-runner-docker/badge.svg)](https://docs.rs/stormchaser-runner-docker) |
| `stormchaser-runner-k8s` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-runner-k8s.svg)](https://crates.io/crates/stormchaser-runner-k8s) | [![Docs.rs](https://docs.rs/stormchaser-runner-k8s/badge.svg)](https://docs.rs/stormchaser-runner-k8s) |
| `stormchaser-agent` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-agent.svg)](https://crates.io/crates/stormchaser-agent) | [![Docs.rs](https://docs.rs/stormchaser-agent/badge.svg)](https://docs.rs/stormchaser-agent) |

## Example Workflow

```hcl
workflow "example_workflow" {
  description = "A simple deployment workflow"

  storage "workspace" {
    size = "1Gi"
  }

  # First step: builds the app into the shared storage, retrying twice on failure
  step "build_app" "RunContainer" {
    spec {
      image   = "node:18"
      command = ["npm", "run", "build"]
      storage_mounts = [
        { name = "workspace", mount_path = "/app/dist" }
      ]
    }
    retry {
      count   = 2
      backoff = "exponential"
    }
    next = ["require_approval"]
  }

  # Second step: waits for a member of the admins group, or alice, to approve
  step "require_approval" "Approval" {
    spec {
      approvers = ["admins", "alice@example.com"]
      timeout   = "24h"
    }
    next = ["deploy_app"]
  }

  # Third step: mounts the same storage and deploys what was built
  step "deploy_app" "RunContainer" {
    spec {
      image   = "node:18"
      command = ["npm", "run", "deploy"]
      storage_mounts = [
        { name = "workspace", mount_path = "/app/dist" }
      ]
    }
  }
}
```

## Key Features

- **HCL-based DSL**: Workflows, step libraries with parameters, includes,
  conditions, iteration and HCL/MiniJinja expressions. `stormchaser lint`
  checks a workflow offline against the DSL's JSON Schema.
- **Distributed execution**: Container steps run on Docker or Kubernetes
  runners. Other steps (`RestApi`, `SqlExecute`, `JQ`, `JinjaRender`,
  `WebhookInvoke`, `Wasm`, `LambdaInvoke`, email, Slack and Teams messages)
  run inside the engine.
- **Human in the loop**: `Approval` steps with approver lists, input forms and
  email notifications carrying sealed, expiring approve and reject links.
- **Event driven**: NATS JetStream carries the engine's events, published
  through a transactional outbox. Several engine replicas can run, with one
  elected through a Postgres advisory lock to run the background workers.
  Webhooks (GitHub signatures, a bearer token, or Standard Webhooks signatures) and
  event rules start runs from outside events.
- **Retries and failure handling**: A step's `retry` block (fixed, linear or
  exponential backoff) and `allow_failure`; run timeouts and quotas.
- **Shared File System (SFS)**: Storage shared between steps, parked as
  tarballs in S3-compatible object storage with SHA-256 verification, plus
  artifacts (S3 or OCI registries) and JUnit test reports.
- **Dynamic parallelism**: `iterate` over a runtime list, with `max_parallel`.
- **Interfaces and observability**: CLI, TUI and Web UI; OpenTelemetry
  traces and metrics; step logs in Loki or Elasticsearch.
- **Policy as code**: Every API request and every run, connection, webhook,
  rule and schedule is checked by Open Policy Agent. Without an OPA server,
  the services enforce a built-in copy of the shipped policy, which refuses
  what it does not recognise.
- **Secrets**: SOPS-encrypted files in the workflow's repository and
  HashiCorp Vault paths, both behind allow-lists.
- **Secure by default**: Services serve HTTPS and connect to PostgreSQL with
  `verify-full` unless told otherwise; SSRF guards, IAM role and Vault path
  allow-lists, and privileged steps off unless the runner allows them. See
  [Security](docs/security.md).
- **Cron workflows**: Schedules are stored by the API and fired by Kubernetes
  CronJobs or any scheduler that can call the trigger URL.
- **[MCP Server](docs/mcp-server.md)**: A Model Context Protocol server that
  exposes the API to AI agents. It is built by default (Cargo feature `mcp`)
  and goes through the same OPA policy as the rest of the API.

### Feature Comparison Matrix

| Feature | Stormchaser | StackStorm | Rundeck | Harness | Jenkins | Argo Workflows | Node-RED | n8n | Celery |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Pricing** | ✅ (FOSS) | ⚠️ (Open-Core) | ⚠️ (Open-Core) | Commercial | ✅ (FOSS) | ✅ (FOSS) | ✅ (FOSS/Hosted) | ✅ (Fair-Code/Hosted) | ✅ (FOSS) |
| **Primary Focus** | DevOps Processes | Event-Driven Ops | Job Scheduling | CI/CD Platform | CI/CD Automation | Kubernetes Native | Event Integration | API / RPA / ETL | Task Queue |
| **Configuration** | HCL + Expressions | YAML / Python | UI / YAML / XML | YAML / UI | Groovy / UI | YAML | JSON / UI | UI / JSON | Python |
| **Execution Model** | Affinity-Aware | Local/Remote Exec | SSH / Agent | SaaS / Delegate | Master / Agent | Pod-per-Step | Node.js Runtime | SaaS / Worker | Worker Nodes |
| **Git Native** | ✅ | ⚠️ (Packs) | ❌ | ✅ | ⚠️ (Plugins) | ⚠️ (ArgoCD) | ⚠️ (Projects) | ⚠️ (Enterprise) | ❌ |
| **K8s / Docker** | ✅ | ⚠️ (Packs) | ⚠️ (Plugins) | ✅ | ⚠️ (Plugins) | ✅ (K8s) | ✅ (Docker) | ✅ (Docker/K8s) | ✅ |
| **WASM Steps** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Webhooks** | ✅ (In/Out) | ✅ (In/Out) | ⚠️ (In) | ✅ (In/Out) | ⚠️ (In) | ⚠️ (Argo Events) | ✅ (In/Out) | ✅ (In/Out) | ❌ |
| **Event Mesh** | NATS JetStream | RabbitMQ / Sensor | Polling / API | Internal Bus | Polling | Sensor | ⚠️ (MQTT Nodes) | ⚠️ (Polling/Hooks) | Redis / RabbitMQ |
| **Human-in-Loop** | Advanced (Multi) | Basic (Inquiry) | Manual Step | Built-in | `input` Step | Basic (Suspend) | ⚠️ (UI Nodes) | ⚠️ (Wait Node) | ❌ |
| **Variable Passing** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (`msg.payload`) | ✅ | ✅ |
| **Input Forms** | ⚠️ (Basic) | ⚠️ (Inquiry) | ✅ (Comprehensive) | ✅ | ✅ (Parameters) | ⚠️ (Basic) | ✅ (Dashboard) | ⚠️ (Basic) | ❌ |
| **Linting/Validation**| ✅ (JSON Schema/Offline) | ⚠️ (Packs) | ⚠️ (Basic) | ✅ (Built-in) | ⚠️ (Basic) | ✅ (Argo Lint) | ⚠️ (Basic) | ⚠️ (Basic) | ⚠️ (Basic) |
| **Security** | OPA Fail-Closed | Action Aliases | ACLs | RBAC / Secrets | Plugin-based RBAC | K8s RBAC | ⚠️ (Basic Auth) | ⚠️ (Varies) | ⚠️ (Basic) |
| **SSO Support** | ✅ (OIDC) | ⚠️ (Enterprise) | ⚠️ (Enterprise) | ✅ | ⚠️ (Plugins) | ✅ (SSO) | ⚠️ (Plugins) | ✅ | ❌ |
| **Distributed Exec** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | ✅ (Queue Mode) | ✅ |
| **Artifact Mgmt** | ✅ (SFS, OCI) | ❌ | ⚠️ (Plugins) | ✅ | ✅ (Plugins) | ✅ | ❌ | ❌ | ❌ |
| **JUnit Reports** | ✅ | ❌ | ❌ | ✅ | ✅ (Plugins) | ⚠️ (Artifacts) | ❌ | ❌ | ❌ |
| **Observability** | ✅ (OTel, TUI) | ⚠️ (Limited) | ⚠️ (Limited) | ✅ | ⚠️ (Plugins) | ✅ (Prometheus) | ⚠️ (Basic) | ⚠️ (Basic) | ⚠️ (Limited) |
| **Email** | ✅ | ✅ (Packs) | ✅ | ✅ | ✅ (Plugins) | ⚠️ (Hooks) | ✅ (Nodes) | ✅ | ❌ |
| **Slack / Teams** | ✅ (Messages) | ✅ (ChatOps) | ⚠️ (Plugins) | ✅ | ✅ (Plugins) | ⚠️ (Hooks) | ✅ (Nodes) | ✅ | ❌ |
| **Pipeline UI** | ✅ (Web UI & TUI) | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ (FlowForge) | ✅ | ⚠️ (Flower) |
| **Visual Builder** | ❌ (Code-First) | ⚠️ (Workflow Designer) | ❌ | ✅ | ✅ (Blue Ocean) | ⚠️ (UI/3rd Party) | ✅ (Comprehensive) | ✅ | ❌ |

*For a full list of features and what is planned, see
[Features](docs/features.md) and [Project Status](docs/project_status.md).
A longer comparison is in [Comparison](docs/comparison.md).*

## Architecture

Stormchaser is composed of a control plane (API, Orchestration Engine, DB,
OPA), an event mesh (NATS JetStream), and an execution plane (Runners,
Stormchaser Agent).

- **Control Plane**: The API, the query service (which fills in workflow
  input forms), the engine and the web UI. The engine can run as several
  replicas.
- **Event Mesh**: NATS JetStream powers the persistent event store and task
  queues.
- **Execution Plane**: Environment-specific runners (e.g., K8s, Docker) execute
  steps utilizing the `stormchaser-agent` to manage shared state and artifacts.

*Read the deep dive into the architecture in
[Architecture Design](docs/architecture.md).*

## Quick Start for Docker

You need Rust 1.96 or newer, Docker with Docker Compose v2 (`docker compose`),
`curl`, `openssl` and Python 3 with `passlib[bcrypt]`:

```bash
pip3 install 'passlib[bcrypt]'
```

The local stack is for evaluation and development. It runs without TLS and
with the bundled Loki, Grafana and MinIO; see
[Local Development with Docker](docs/local_dev_docker.md) for details.

1. **Clone the repository:**

   ```bash
   git clone https://github.com/paninfracon/stormchaser.git
   cd stormchaser
   ```

2. **Run the setup script:**

   ```bash
   ./scripts/setup.sh
   ```

   The default mode is `docker`. The script generates TLS certificates in
   `tests/certs`, random passwords in `.env`, and the Dex configuration and
   persona credentials (`deploy/dex/credentials.generated`, mode 0600). It
   then builds and starts the compose project `stormchaser-docker`:
   PostgreSQL, NATS, Dex, OPA, MinIO, a registry, Loki, Tempo, Prometheus,
   Grafana, Alloy, the API (port 3000), the query service (3001), the web UI
   (3003), the engine and the Docker runner. Finally it registers MinIO as the
   default SFS storage.

3. **Log in and list runs:**

   Add `127.0.0.1 dex` to `/etc/hosts` so the browser can reach the local
   identity provider, then:

   ```bash
   cargo run -p stormchaser-cli -- --url http://localhost:3000 login
   cargo run -p stormchaser-cli -- --url http://localhost:3000 runs list
   ```

   Log in as one of the personas in `deploy/dex/credentials.generated`, such
   as `stormchaser-admin@paninfracon.net`. `run-cli.sh` lists runs with a
   token you have already exported as `STORMCHASER_TOKEN`. Pass tokens in the
   environment or a profile, never as an argument, where other users on the
   host can read them.

4. **Run a workflow and monitor it:**

   ```bash
   cargo run -p stormchaser-cli -- --url http://localhost:3000 run path/to/workflow.storm
   cargo run -p stormchaser-tui
   ```

   The web UI is at `http://localhost:3003`.

   *Note: If the `deploy/dex/credentials.generated` file exists, the TUI login
   screen offers a list of the generated personas for one-key login. This is
   a quickstart-only feature that relies on the random passwords generated by
   the local setup script, and it should NOT be used in production.*

## Documentation

Detailed documentation is available in the `docs/` directory:

- [Architecture & Design](docs/architecture.md)
- [Workflow DSL Reference](docs/workflow_dsl.md)
- [Environments & Deployment (inc. K8s), with the configuration reference](docs/environments.md)
- [Local Development with Docker](docs/local_dev_docker.md)
- [Security](docs/security.md)
- [NATS](docs/nats.md)
- [Observability (Metrics, Logs, Tracing)](docs/observability.md)
- [OPA RBAC Policies](docs/opa-rbac.md)
- [Resource Quotas](docs/quotas.md)
- [State Machines](docs/state_machines.md)
- [Cargo Features](docs/cargo-features.md)
- [Features](docs/features.md) and [Project Status](docs/project_status.md)
- [Release How-To](docs/release-howto.md)

Explore the [`docs/steps/`](docs/steps/) directory for detailed information on available
workflow steps (e.g., RunContainer, WebhookInvoke, Approval).

## Acknowledgements

As always we stand on the shoulders of giants. Among the many awesome
software engineers who have lead the way for a project like this I would
like to explicitly call out:

- [Stackstorm](https://stackstorm.com/)
- [Rundeck](https://www.rundeck.com/)
- [Node-RED](https://nodered.org/)
- [Terraform](https://www.hashicorp.com/) (and the awesome team at Hashicorp)
- The rust development team

As well as everyone else from the FOSS Community - together we are stronger!

This project is published under the MIT and Apache licenses to give something back.

## Contributing

We welcome contributions! Please see our
[Contributing Guidelines](CONTRIBUTING.md) and adhere to our
[Code of Conduct](CODE_OF_CONDUCT.md).

### Development Setup

The project is structured as a Cargo workspace containing multiple crates
(e.g., `stormchaser-api`, `stormchaser-engine`, `stormchaser-model`).

Run tests with the scripts below. The integration tests start their own
PostgreSQL and NATS in Docker (`scripts/test-env-up.sh`, stopped with
`scripts/test-env-down.sh`); the unit tests do too with
`STORMCHASER_UNIT_TEST_WITH_ENV=1`. Both pass extra arguments to `cargo test`,
such as `--all-features`:

```bash
./scripts/test-unit.sh
./scripts/test-integration.sh
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the tools they need.

## License

This project is licensed under multiple licenses depending on the component.
Please see the following files for details:

- [LICENSE-APACHE](LICENSE-APACHE)
- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-CDLA-2.0.md](LICENSE-CDLA-2.0.md)

## Contributors

- Maintained by the Stormchaser Core Team.
- **Gemini (AI Assistant)** - Assisting in engineering, documentation, and
  design of the Stormchaser engine.
