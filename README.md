# 🌪️ Stormchaser

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

### 📊 Feature Comparison Matrix

| Feature | Stormchaser | StackStorm | Rundeck | Harness | Jenkins | Argo Workflows | Node-RED |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Pricing** | ✅ (FOSS) | ⚠️ (Open-Core) | ⚠️ (Open-Core) | Commercial | ✅ (FOSS) | ✅ (FOSS) | ✅ (FOSS/Hosted) |
| **Primary Focus** | DevOps Processes | Event-Driven Ops | Job Scheduling | CI/CD Platform | CI/CD Automation | Kubernetes Native | Event Integration |
| **Configuration** | HCL + Expressions | YAML / Python | UI / YAML / XML | YAML / UI | Groovy / UI | YAML | JSON / UI |
| **Execution Model** | Affinity-Aware | Local/Remote Exec | SSH / Agent | SaaS / Delegate | Master / Agent | Pod-per-Step | Node.js Runtime |
| **Git Native** | ✅ | ⚠️ (Packs) | ❌ | ✅ | ⚠️ (Plugins) | ⚠️ (ArgoCD) | ⚠️ (Projects) |
| **K8s / Docker** | ✅ | ⚠️ (Packs) | ⚠️ (Plugins) | ✅ | ⚠️ (Plugins) | ✅ (K8s) | ✅ (Docker) |
| **WASM Steps** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Webhooks** | ✅ (In/Out) | ✅ (In/Out) | ⚠️ (In) | ✅ (In/Out) | ⚠️ (In) | ⚠️ (Argo Events) | ✅ (In/Out) |
| **Event Mesh** | NATS JetStream | RabbitMQ / Sensor | Polling / API | Internal Bus | Polling | Sensor | ⚠️ (MQTT Nodes) |
| **Human-in-Loop** | Advanced (Multi) | Basic (Inquiry) | Manual Step | Built-in | `input` Step | Basic (Suspend) | ⚠️ (UI Nodes) |
| **Variable Passing** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ (`msg.payload`) |
| **Input Forms** | ⚠️ (Basic) | ⚠️ (Inquiry) | ✅ (Comprehensive) | ✅ | ✅ (Parameters) | ⚠️ (Basic) | ✅ (Dashboard) |
| **Security** | OPA Fail-Closed | Action Aliases | ACLs | RBAC / Secrets | Plugin-based RBAC | K8s RBAC | ⚠️ (Basic Auth) |
| **SSO Support** | ✅ (OIDC) | ⚠️ (Enterprise) | ⚠️ (Enterprise) | ✅ | ⚠️ (Plugins) | ✅ (SSO) | ⚠️ (Plugins) |
| **Distributed Exec** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ |
| **Artifact Mgmt** | ✅ (SFS, OCI) | ❌ | ⚠️ (Plugins) | ✅ | ✅ (Plugins) | ✅ | ❌ |
| **JUnit Reports** | ✅ | ❌ | ❌ | ✅ | ✅ (Plugins) | ⚠️ (Artifacts) | ❌ |
| **Observability** | ✅ (OTel, TUI) | ⚠️ (Limited) | ⚠️ (Limited) | ✅ | ⚠️ (Plugins) | ✅ (Prometheus) | ⚠️ (Basic) |
| **Email** | ✅ | ✅ (Packs) | ✅ | ✅ | ✅ (Plugins) | ⚠️ (Hooks) | ✅ (Nodes) |
| **Slack / Teams** | ⚠️ (Planned) | ✅ (ChatOps) | ⚠️ (Plugins) | ✅ | ✅ (Plugins) | ⚠️ (Hooks) | ✅ (Nodes) |
| **Pipeline UI** | ⚠️ (TUI Only) | ✅ | ✅ | ✅ | ✅ | ✅ | ⚠️ (FlowForge) |
| **Visual Builder** | ❌ (Code-First) | ⚠️ (Workflow Designer) | ❌ | ✅ | ✅ (Blue Ocean) | ⚠️ (UI/3rd Party) | ✅ (Comprehensive) |

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

## 🚀 Quick Start

Ensure you have Rust, Docker, Docker Compose v2 (`docker compose`) or the legacy `docker-compose` wrapper, and Python 3 with `passlib[bcrypt]` installed.

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

## 📚 Documentation

Detailed documentation is available in the `docs/` directory:

- [Architecture & Design](docs/architecture.md)
- [Workflow DSL Reference](docs/workflow_dsl.md)
- [Environments & Deployment](docs/environments.md)
- [Observability (Metrics, Logs, Tracing)](docs/observability.md)
- [OPA RBAC Policies](docs/opa-rbac.md)
- [State Machines](docs/state_machines.md)
- [Current State & Status](docs/current_state.md)

Explore the `docs/steps/` directory for detailed information on available
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

Run tests using:

```bash
cargo test
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
