# Tool Comparison: Stormchaser vs. The Ecosystem

Stormchaser is designed as a **DevOps Process Orchestrator**. Unlike data
pipeline tools (Airflow, Prefect) that focus on data gravity and ETL,
Stormchaser focuses on low-latency event response, human-in-the-loop security,
and execution performance for modular automation.

## Feature Matrix

| Feature             | Stormchaser           | Argo Workflows       | StackStorm        | Rundeck         | Harness         |
| :------------------ | :-------------------- | :------------------- | :---------------- | :-------------- | :-------------- |
| **Primary Focus**   | DevOps Processes      | Kubernetes Native    | Event-Driven Ops  | Job Scheduling  | CI/CD Platform  |
| **Configuration**   | HCL + HCL Expressions | YAML                 | YAML / Python     | UI / YAML / XML | YAML / UI       |
| **Execution Model** | Affinity-Aware        | Pod-per-Step         | Local/Remote Exec | SSH / Agent     | SaaS / Delegate |
| **Performance**     | Warm Runner Agent     | High Overhead (Cold) | Medium (Python)   | High (SSH)      | Variable        |
| **Event Mesh**      | NATS JetStream        | Webhooks / Sensor    | RabbitMQ / Sensor | Polling / API   | Webhooks        |
| **Human-in-Loop**   | Advanced (Multi)      | Basic (Suspend)      | Basic (Inquiry)   | Manual Step     | Built-in        |
| **Security**        | OPA Fail-Closed       | K8s RBAC             | Action Aliases    | ACLs            | RBAC / Secrets  |
| **Linting/Validation**| JSON Schema / Offline | Argo Lint            | Pack Testing      | Basic Syntax    | Built-in        |
| **Language**        | Rust                  | Go                   | Python            | Java            | Java/Go         |

---

## 1. vs. Argo Workflows (Kubernetes-Native)

Argo is the industry standard for K8s workflows, but it treats every step as a
separate Pod.

- **Stormchaser Advantage:** **Execution Affinity**. Stormchaser can run
  multiple modular steps (checkout, lint, build) in the same container using the
  "Warm Runner Agent," reducing overhead by 30-50%.
- **DSL:** Stormchaser rejects YAML's whitespace sensitivity in favor of a
  robust native HCL-based DSL, making it far more maintainable for complex
  logic.

## 2. vs. StackStorm (IFTTT for Ops)

StackStorm pioneered event-driven automation, providing a robust Python-based
ecosystem, though its architecture can require additional operational overhead
to scale dynamically.

- **Stormchaser Advantage:** **Native Scalability**. Using NATS JetStream and
  "Sticky Sharding," Stormchaser handles distributed state and event persistence
  natively in its core.
- **Speed:** Being written in Rust with a NATS-backed event mesh, Stormchaser is
  designed for sub-millisecond event-to-workflow latency.

## 3. vs. Rundeck (Traditional Job Scheduling)

Rundeck excels at "turning scripts into buttons" but lacks modern GitOps and
event-driven primitives.

- **Stormchaser Advantage:** **GitOps-First**. Stormchaser treats the Git
  repository as the source of truth for all DSL and secrets (SOPS), syncing them
  to a high-performance DB cache rather than relying on manual UI configuration.
- **Isolation:** Unlike Rundeck's often runner-sticky model, Stormchaser
  provides explicit `isolated` vs `shared` affinity controls.

## 4. vs. Harness (Enterprise CI/CD)

Harness is a comprehensive, often SaaS-based, platform. It is powerful but can
feel like a "black box" with rigid YAML schemas.

- **Stormchaser Advantage:** **Extensibility**. Stormchaser's library system
  allows users to define custom Step types and expand the AST itself. It is
  designed for engineers who need to build custom, highly-integrated automation
  platforms, not just consume a CI/CD service.
- **Resource Control:** Stormchaser provides granular **I/O Speed Governors**
  and provisioning blocks, allowing for fine-tuned multi-tenant stability that
  generic CI/CD tools often lack.

---

## Why Stormchaser?

### DevOps Process vs. Data Pipeline

Data tools (like Airflow) are built for **throughput and data gravity**. They
handle "Big Data" well but are often too slow or heavy for "DevOps" tasks like:

- Responding to a "Production Incident" event in <100ms.
- Orchestrating a complex rollout across multiple K8s clusters and AWS regions.
- Handling nested human approvals with identity validation.

**Stormchaser is built for the latter.** It optimizes for the **Developer
Experience (DX)** through its HCL-based DSL and for **Operator Confidence**
through its Fail-Closed OPA security and rigorous resource quotas.

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
| **Workflow Execution** | `run` (file), `enqueue` (git) | Integrated File Browser & Form |
| **Log Viewing** | Streaming (`logs` cmd) | Paginated & Streamed UI Viewer |
| **Run Discovery** | `list` with flags | Visual List with Interactive Filters |
| **DSL Linting & Validation**| `lint` (CLI-exclusive) | ❌ (Relies on server-side validation) |
| **JSON Schema Export** | `schema` (CLI-exclusive) | ❌ |
| **Human-in-the-Loop (HITL)**| `approve` command | Interactive Approval Dialog |
| **Artifacts & Test Reports**| `artifacts`, `reports` | ❌ (Metadata view only) |
| **Storage Backends** | Full CRUD via JSON payloads | Interactive CRUD Forms |
| **Webhooks** | Full CRUD via JSON payloads | Interactive CRUD Forms |
| **Event Rules & Cron** | Full CRUD (`rules`, `cron`) | Interactive CRUD Forms |
| **Authentication** | OAuth Browser, Token generation | OAuth Browser Flow |
