# Tool Comparison: Stormchaser vs. The Ecosystem

Stormchaser is a **DevOps process orchestrator**. Data pipeline tools such as
Airflow and Prefect focus on throughput and ETL. Stormchaser focuses on
responding to events, human approvals with identity and policy checks, and
modular steps that run in containers or inside the engine.

This page compares design choices, not measured performance. Where a
Stormchaser capability is planned rather than built, it says so.

## Feature Matrix

| Feature             | Stormchaser           | Argo Workflows       | StackStorm        | Rundeck         | Harness         |
| :------------------ | :-------------------- | :------------------- | :---------------- | :-------------- | :-------------- |
| **Primary Focus**   | DevOps Processes      | Kubernetes Native    | Event-Driven Ops  | Job Scheduling  | CI/CD Platform  |
| **Configuration**   | HCL + HCL Expressions | YAML                 | YAML / Python     | UI / YAML / XML | YAML / UI       |
| **Execution Model** | Runner per platform; built-in steps in the engine | Pod-per-Step         | Local/Remote Exec | SSH / Agent     | SaaS / Delegate |
| **Event Mesh**      | NATS JetStream        | Webhooks / Sensor    | RabbitMQ / Sensor | Polling / API   | Webhooks        |
| **Human-in-Loop**   | Advanced (Multi)      | Basic (Suspend)      | Basic (Inquiry)   | Manual Step     | Built-in        |
| **Security**        | OPA Fail-Closed       | K8s RBAC             | Action Aliases    | ACLs            | RBAC / Secrets  |
| **Linting/Validation**| JSON Schema / Offline | Argo Lint            | Pack Testing      | Basic Syntax    | Built-in        |
| **Language**        | Rust                  | Go                   | Python            | Java            | Java/Go         |

---

## 1. vs. Argo Workflows (Kubernetes-Native)

Argo is the industry standard for K8s workflows, but it treats every step as a
separate Pod.

- **Built-in steps:** HTTP calls, SQL, jq, templates, WASM, email and chat
  messages run inside the engine, with no Pod or container per step.
- **Execution affinity:** Steps in a `shared` affinity group go to the runner
  the group's first step landed on. Running several steps in one container
  (`locked` affinity, and reuse of a warm container for `shared`) is planned;
  see [Affinity](affinity.md).
- **DSL:** Workflows are written in an HCL-based DSL rather than YAML.

## 2. vs. StackStorm (IFTTT for Ops)

StackStorm pioneered event-driven automation, with a large Python-based
ecosystem of packs.

- **Scaling:** Events are persisted in NATS JetStream. Engine replicas can
  split the event stream by shard (`STORMCHASER_ASSIGNED_SHARDS`), and one
  replica, chosen through a Postgres advisory lock, runs the background
  workers.
- **Language:** The services are written in Rust. No latency figures have
  been published.

## 3. vs. Rundeck (Traditional Job Scheduling)

Rundeck excels at "turning scripts into buttons" but lacks modern GitOps and
event-driven primitives.

- **Git as the source:** Runs read their workflow and SOPS-encrypted secrets
  from a Git repository at a given ref. The engine keeps a local clone cache;
  nothing is configured through the UI.
- **Isolation:** Steps run in fresh containers by default (`isolated`); a
  `shared` group pins its steps to one runner.

## 4. vs. Harness (Enterprise CI/CD)

Harness is a comprehensive, often SaaS-based, platform. It is powerful but can
feel like a "black box" with rigid YAML schemas.

- **Extensibility:** Step libraries define reusable steps with parameters,
  and custom step types can be registered as WASM modules. It suits engineers
  building their own automation platform rather than consuming a CI/CD
  service.
- **Resource control:** Per-run quotas (concurrency, CPU, memory, timeout)
  under operator ceilings, per-step CPU and memory limits enforced by the
  runners, and storage provisioning blocks. Storage I/O limits
  (`max_throughput`, `max_iops`) are planned, not enforced.

---

## Why Stormchaser?

### DevOps Process vs. Data Pipeline

Data tools (like Airflow) are built for throughput and large data sets. DevOps
tasks need something else:

- Starting a run from an incident or webhook event.
- Orchestrating a rollout across several Kubernetes clusters and AWS regions.
- Approvals that check who is approving.

Stormchaser is built for these. Workflows are written in an HCL-based DSL, and
operators get an OPA policy that refuses what it does not recognise, plus
resource quotas.

---

## TUI vs. CLI Feature Matrix

While both tools interact with the same underlying API and core models, they are
optimized for different phases of the DevOps lifecycle. The CLI is focused on
scripting, authoring, and point-in-time interactions, while the TUI is designed
for real-time observability, troubleshooting, and interactive management.

| Feature / Capability | CLI (`stormchaser-cli`) | TUI (`stormchaser-tui`) |
| :--- | :---: | :---: |
| **Primary Use Case** | Scripting, CI/CD, DSL Authoring | Live Monitoring, Troubleshooting |
| **Execution Mode** | Headless / Synchronous | Interactive / Real-time (SSE) |
| **Workflow Execution** | `run` (file), `runs enqueue` (git) | Integrated File Browser & Form |
| **Log Viewing** | `runs logs` | Paginated & Streamed UI Viewer |
| **Run Discovery** | `runs list` with flags | Visual List with Interactive Filters |
| **DSL Linting & Validation**| `lint` (CLI-exclusive) | ❌ (Relies on server-side validation) |
| **JSON Schema Export** | `schema` (CLI-exclusive) | ❌ |
| **Human-in-the-Loop (HITL)**| `runs approve`, `runs reject`, `runs approve-link` | Pending Approvals tab and Approval Dialog |
| **Artifacts & Test Reports**| `runs artifacts`, `runs reports`, `runs report` | Artifact list and test results pane in run details |
| **Connections (incl. storage)** | `connections` | Interactive CRUD Forms |
| **Webhooks** | `webhooks` | Interactive CRUD Forms |
| **Event Rules & Cron** | Full CRUD (`rules`, `cron`) | Interactive CRUD Forms |
| **Authentication** | OIDC browser login, profiles (`login`, `auth`) | OIDC browser login |
