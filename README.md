# 🌪️ Stormchaser

[![CI](https://github.com/paninfracon/stormchaser/actions/workflows/ci.yml/badge.svg)](https://github.com/paninfracon/stormchaser/actions/workflows/ci.yml)
[![Coverage](https://codecov.io/gh/paninfracon/stormchaser/branch/trunk/graph/badge.svg)](https://codecov.io/gh/paninfracon/stormchaser)
[![License](https://img.shields.io/badge/license-MIT_OR_Apache--2.0_OR_CDLA--Permissive--2.0-blue.svg)](#-license)
[![prek](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/j178/prek/master/docs/assets/badge-v0.json)](https://github.com/j178/prek)
[![pre-commit](https://img.shields.io/badge/pre--commit-enabled-brightgreen?logo=pre-commit)](https://github.com/pre-commit/pre-commit)

A robust, distributed workflow engine for event-driven and human-triggered
workflows. Built in Rust for performance and reliability, utilizing a
graph-based DSL, NATS JetStream for event messaging, and PostgreSQL for state
management.

## 📖 Overview

Stormchaser is designed to orchestrate complex execution graphs. It moves
beyond simple CI/CD pipelines to support long-running processes,
human-in-the-loop approvals, dynamic parallelism, and distributed step
execution across container runtimes like Kubernetes and Docker.

Instead of relying on YAML, Stormchaser uses a bespoke, graph-based Domain
Specific Language (DSL) providing a clean, HCL-like syntax for defining robust
workflows.

## 📦 Crates

*For a breakdown of compilation features across these crates, see [Cargo Features](docs/cargo-features.md).*

| Crate | Version | Docs |
|---|---|---|
| `stormchaser-cli` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-cli.svg)](https://crates.io/crates/stormchaser-cli) | [![Docs.rs](https://docs.rs/stormchaser-cli/badge.svg)](https://docs.rs/stormchaser-cli) |
| `stormchaser-engine` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-engine.svg)](https://crates.io/crates/stormchaser-engine) | [![Docs.rs](https://docs.rs/stormchaser-engine/badge.svg)](https://docs.rs/stormchaser-engine) |
| `stormchaser-model` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-model.svg)](https://crates.io/crates/stormchaser-model) | [![Docs.rs](https://docs.rs/stormchaser-model/badge.svg)](https://docs.rs/stormchaser-model) |
| `stormchaser-api` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-api.svg)](https://crates.io/crates/stormchaser-api) | [![Docs.rs](https://docs.rs/stormchaser-api/badge.svg)](https://docs.rs/stormchaser-api) |
| `stormchaser-dsl` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-dsl.svg)](https://crates.io/crates/stormchaser-dsl) | [![Docs.rs](https://docs.rs/stormchaser-dsl/badge.svg)](https://docs.rs/stormchaser-dsl) |
| `stormchaser-opa` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-opa.svg)](https://crates.io/crates/stormchaser-opa) | [![Docs.rs](https://docs.rs/stormchaser-opa/badge.svg)](https://docs.rs/stormchaser-opa) |
| `stormchaser-tls` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-tls.svg)](https://crates.io/crates/stormchaser-tls) | [![Docs.rs](https://docs.rs/stormchaser-tls/badge.svg)](https://docs.rs/stormchaser-tls) |
| `stormchaser-runner-docker` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-runner-docker.svg)](https://crates.io/crates/stormchaser-runner-docker) | [![Docs.rs](https://docs.rs/stormchaser-runner-docker/badge.svg)](https://docs.rs/stormchaser-runner-docker) |
| `stormchaser-runner-k8s` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-runner-k8s.svg)](https://crates.io/crates/stormchaser-runner-k8s) | [![Docs.rs](https://docs.rs/stormchaser-runner-k8s/badge.svg)](https://docs.rs/stormchaser-runner-k8s) |
| `stormchaser-agent` | [![Crates.io](https://img.shields.io/crates/v/stormchaser-agent.svg)](https://crates.io/crates/stormchaser-agent) | [![Docs.rs](https://docs.rs/stormchaser-agent/badge.svg)](https://docs.rs/stormchaser-agent) |

## 📝 Example Workflow

```hcl
workflow "example_workflow" {
  description = "A simple deployment workflow"

  storage "workspace" {
    size = "1Gi"
  }

  # First step: Builds the app and stores artifacts in SFS
  step "build_app" "RunContainer" {
    spec {
      image   = "node:18"
      command = ["npm", "run", "build"]
      storage_mounts = [
        { name = "workspace", mount_path = "/app/dist" }
      ]
    }
    next = ["require_approval"]
  }

  # Second step: Waits for a human to approve the deployment
  step "require_approval" "Approval" {
    spec {
      approvers = ["group:admins", "user:alice"]
      timeout   = "24h"
    }
    next = ["deploy_app"]
  }

  # Third step: Mounts the SFS and deploys the built artifacts
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

## ✨ Key Features

- **Graph-Based DSL**: Define workflows using a powerful, typed, and extensible
  configuration language.
- **Distributed Execution**: Dispatch workflow steps to container runtimes
  (K8s, Docker, EKS, ECS) or AWS Lambda.
- **Human-In-The-Loop**: Pause workflows for human interaction, such as
  approvals, form inputs, or email responses.
- **Event-Driven**: Fully event-driven orchestration engine using NATS
  JetStream and a robust state machine.
- **Shared File System (SFS)**: Seamless state and file sharing between steps,
  backed by object storage (S3/GCS) with SHA-256 verification.
- **Dynamic Parallelism**: Map/Reduce capabilities based on runtime inputs.
- **Rich Observability**: Built-in CLI and interactive TUI for real-time
  monitoring. Full OpenTelemetry tracing integration.
- **Policy as Code**: Integrated with Open Policy Agent (OPA) for workflow
  execution validation and authorization.
- **First-Class Security**: First-class SOPS support for secrets, integrated
  with Vault and AWS Secrets Manager.
- **Cron Workflows**: Periodic scheduling via external engines (Kubernetes
  CronJobs, Ofelia).
- **[MCP Server](docs/mcp-server.md)**: Native Model Context Protocol (MCP) server for exposing the API to AI agents (enabled by default but can be disabled via Cargo features, and ships with a restrictive Open Policy Agent authorization policy).

### 📊 Feature Comparison Matrix

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
| **Slack / Teams** | ⚠️ (Planned) | ✅ (ChatOps) | ⚠️ (Plugins) | ✅ | ✅ (Plugins) | ⚠️ (Hooks) | ✅ (Nodes) | ✅ | ❌ |
| **Pipeline UI** | ⚠️ (TUI Only) | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ (FlowForge) | ✅ | ⚠️ (Flower) |
| **Visual Builder** | ❌ (Code-First) | ⚠️ (Workflow Designer) | ❌ | ✅ | ✅ (Blue Ocean) | ⚠️ (UI/3rd Party) | ✅ (Comprehensive) | ✅ | ❌ |

*For a full list of features and planned roadmap, see
[Features](docs/features.md).*

## 🏗️ Architecture

Stormchaser is composed of a control plane (API, Orchestration Engine, DB,
OPA), an event mesh (NATS JetStream), and an execution plane (Runners,
Stormchaser Agent).

- **Control Plane**: Rust-based distributed controller cluster managing state
  and handling API requests.
- **Event Mesh**: NATS JetStream powers the persistent event store and task
  queues.
- **Execution Plane**: Environment-specific runners (e.g., K8s, Docker) execute
  steps utilizing the `stormchaser-agent` to manage shared state and artifacts.

*Read the deep dive into the architecture in
[Architecture Design](docs/architecture.md).*

## 🚀 Quick Start for Docker

Ensure you have Rust, Docker, Docker Compose v2 (`docker compose`) or the
legacy `docker-compose` wrapper, and Python 3 with `passlib[bcrypt]`
installed.

```bash
pip3 install 'passlib[bcrypt]'
```

1. **Clone the repository:**

   ```bash
   git clone https://github.com/paninfracon/stormchaser.git
   cd stormchaser
   ```

2. **Run the setup script:**

   ```bash
   ./scripts/setup.sh
   ```

   This generates TLS certificates, creates Dex identity provider credentials
   (stored in `deploy/dex/credentials.generated`, mode 0600), and starts all
   Docker services (Postgres, NATS, Loki, Dex, MinIO, OPA). The `docker compose`
   step is handled internally by the script, so there is no need to run it
   separately before or after.

3. **Run a test workflow:**

   ```bash
   ./run-cli.sh run tests/hello-world.storm
   ```

4. **Monitor with the TUI:**

   ```bash
   cargo run -p stormchaser-tui
   ```

   *Note: If the `deploy/dex/credentials.generated` file exists, the TUI login
   screen will automatically offer a list of generated personas
   (like `stormchaser-admin@paninfracon.net`) for a seamless 1-click
   auto-login experience. This is a quickstart-only feature that relies on
   the random passwords generated by the local setup script, and it should
   NOT be used in production.*

## 📚 Documentation

Detailed documentation is available in the `docs/` directory:

- [Architecture & Design](docs/architecture.md)
- [Workflow DSL Reference](docs/workflow_dsl.md)
- [Environments & Deployment (inc. K8s)](docs/environments.md)
- [Observability (Metrics, Logs, Tracing)](docs/observability.md)
- [OPA RBAC Policies](docs/opa-rbac.md)
- [State Machines](docs/state_machines.md)
- [Current State & Status](docs/current_state.md)

Explore the [`docs/steps/`](docs/steps/) directory for detailed information on available
workflow steps (e.g., Parallel, WebhookInvoke, RunContainer).

## 🤝 Acknowledgements

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

## 🤝 Contributing

We welcome contributions! Please see our
[Contributing Guidelines](CONTRIBUTING.md) and adhere to our
[Code of Conduct](CODE_OF_CONDUCT.md).

### Development Setup

The project is structured as a Cargo workspace containing multiple crates
(e.g., `stormchaser-api`, `stormchaser-engine`, `stormchaser-model`).

Run tests using the isolated test infrastructure scripts:

```bash
./scripts/test-unit.sh
./scripts/test-integration.sh
```

## 📝 License

This project is licensed under multiple licenses depending on the component.
Please see the following files for details:

- [LICENSE-APACHE](LICENSE-APACHE)
- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-CDLA-2.0.md](LICENSE-CDLA-2.0.md)

## ✨ Contributors

- Maintained by the Stormchaser Core Team.
- **Gemini (AI Assistant)** - Assisting in engineering, documentation, and
  design of the Stormchaser engine.
