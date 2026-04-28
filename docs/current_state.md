# Stormchaser Project: Current State Analysis

This report analyzes the current implementation status of Stormchaser against the
goals defined in `features.md`.

## Recent Accomplishments & Status

**Key Accomplishments:**

- **CI/CD Workflow Fixes:** Consolidated and fixed the GitHub Actions workflows (`ci.yml`). The test suite now correctly provisions a Postgres database using `sqlx-cli`, generates required runtime TLS certificates, and sets explicit API rate limits to prevent `429 Too Many Requests` errors, ensuring all tests pass in CI.
- **Test Certificate Handling:** Removed hardcoded dummy certificates from Git tracking and updated setup scripts to generate them locally on-the-fly, fixing `gitleaks` pre-commit hook failures and improving security posture.
- **Git History Compression:** Compressed the Git history into a single initial commit on the `trunk` branch, preparing the repository for upstreaming.
- **JinjaRender Step:** Implemented a new native `JinjaRender` step that allows stand-alone MiniJinja template rendering.
- **TestReportEmail Step:** Implemented a specialized `TestReportEmail` step that sends rich HTML test reports (summaries + failures) with an overridable template.
- **AWS SES Backend:** Added support for AWS SES as an alternative email delivery backend, utilizing the AWS SDK and supporting IAM role assumption (EKS Pod Identity). This is available behind the `aws-ses` feature gate.
- **Documentation Alignment:** Updated documentation to accurately reflect that Webhook, Email, Lambda, WASM, and JinjaRender native steps are implemented and integrated into the engine.
- **Intrinsic Step Integration:** Discovered and fixed a gap where `WebhookInvoke` and `EmailSend` handlers were defined but not correctly dispatched. I have now implemented the intrinsic dispatchers for these step types.
- **Archival Verification:** Verified the archival logic in `stormchaser-engine`. Recent fixes for race conditions in the archival process have been confirmed in the code.

**Code Coverage Summary:**

Overall, the project has approximately 45% line coverage. Key backend components like the engine and API have higher coverage in crucial areas, while UI components (like the TUI) and specific runners have lower coverage.

- **Total Regions Covered:** 40.65% (13,599 missed / 22,913 total)
- **Total Functions Covered:** 47.03% (741 missed / 1,399 total)
- **Total Lines Covered:** 45.11% (8,848 missed / 16,120 total)

**Next Steps:**

- **Push to GitHub:** Authenticate the GitHub CLI (`gh auth login`) and push the repository to the new private `stormchaser` repo.
- **Verify Dogfood Run:** Run the updated `dogfood.storm` to confirm the full lifecycle from build to deploy, ensuring data is correctly archived upon completion.
- **Feature Gap Focus:** Future development will focus on high-priority missing features like Workflow Templates, CronWorkflows, and Step Memoization/Caching.

## Implementation Overview

| Category | Status | Details |
| :--- | :--- | :--- |
| **Language** | Implemented | Rust project with workspace structure. |
| **Data Store** | Implemented | PostgreSQL for state and audit; NATS for event-driven orchestration. |
| **DSL** | Implemented | Graph-based DSL with Tree-sitter support and HCL expression evaluation. |
| **Execution** | Implemented | Distributed state machine with support for K8s and Docker runners. Includes Optimistic Concurrency Control (OCC) and Global Concurrency Enforcement. |
| **Auth** | Implemented | SSO (OIDC/Dex) and Policy-as-Code (OPA) integration. |

## Feature Analysis

### 1. Core Workflow & DSL

- **Graph based DSL (NOT YAML!):** [Implemented] DSL is defined in
  `stormchaser-dsl` and `stormchaser-model`. Supports complex graphs via `next`
  lists.
- **Tree-sitter grammar:** [Not Implemented/Delegated] `.storm` files are standard HCL, meaning the generic `tree-sitter-hcl` grammar can be used for editor integration. `stormchaser-dsl` parses the AST directly using `hcl-rs`.
- **Sequential and parallel workflow steps:** [Implemented] Naturally supported
  by the graph execution model in `stormchaser-engine`.
- **Output/Input passing:** [Implemented] HCL expressions (`${...}`) allow
  passing values between steps. Support for log scraping (stdout/stderr) to
  extract variables via regex.
- **Dynamic Parallelism (Map/Reduce):** [Implemented] `iterate` and `iterate_as`
  are supported. The engine handles fan-out with `max_parallel` batching, global
  concurrency enforcement, and results aggregation.
- **Step Memoization/Caching:** [Not Implemented] Mentioned in docs but no
  implementation found.

### 2. Execution Engine

- **Event-driven state machine (NATS + Postgres):** [Implemented] Core
  orchestration logic in `stormchaser-engine` uses NATS JetStream and Postgres.
- **Distributed execution:** [Implemented] Runners subscribe to NATS subjects
  for task distribution.
- **Step dispatch to container runtimes:** [Implemented] Native runners for
  Kubernetes (`stormchaser-runner-k8s`) and Docker (`stormchaser-runner-docker`).
- **Step optimization:** [Not Implemented] Logic to run multiple steps in one
  container is TBD.
- **Step restart/adoption:** [Implemented] K8s runner includes logic to adopt
  orphaned jobs after a restart.
- **Concurrency Limits:** [Implemented] `max_concurrency` from `run_quotas` is
  enforced at the engine level. Steps are queued until slots become available.
- **Optimistic Concurrency Control:** [Implemented] Version-based OCC in
  `workflow_runs` ensures safe state transitions across distributed engine
  instances.
- **Resource Quota Enforcement:** [Implemented] CPU and Memory limits (as defined
  in `run_quotas`) are enforced at the engine level using a reservation system.
- **Timeout Enforcement:** [Implemented] Background "timeout reaper" in the engine
  automatically fails workflows that exceed their `timeout` quota.

### 3. Integrations & Steps

- **Git Integration:** [Implemented] Engine syncs workflow definitions and
  static secrets from Git repos.
- **Secrets Management:** [Implemented] Dynamic secret lookup via
  `secret("path#key")` in HCL. Pluggable backend architecture with initial Vault
  support. Safe asynchronous lookups integrated into the synchronous HCL
  evaluation engine.
- **Webhook endpoint for receiving events:** [Implemented] Generic and
  GitHub-specific webhook support with signature validation.
- **Event Rules Engine:** [Implemented] Dynamic mapping, filtering (HCL), and
  transforming events to workflow runs.
- **Step Types:**
  - `RunContainer` / `RunK8sJob`: [Implemented]
  - `Invoke Webhook`: [Implemented] Native handler in the engine with template
    rendering for body/headers.
  - `Send Email`: [Implemented] `EmailSend` native handler in the engine with
    MiniJinja templating (guarded by `email` feature). Supports SMTP (with TLS/mTLS)
    and AWS SES (guarded by `aws-ses` feature) backends.
  - `LambdaInvoke`: [Implemented] Native handler in the engine (guarded by
    `aws-lambda` feature).
  - `Request Approval`: [Implemented] Full lifecycle support for human
    interaction steps including triggering an approval, waiting, processing the
    response (with inputs), and resuming. Includes unauthenticated approval
    links via encrypted tokens and external event correlation.
  - `Wasm`: [Implemented] Local WASM execution via Wasmtime.
  - `JinjaRender`: [Implemented] Native Jinja templating for data transformation.
  - `TestReportEmail`: [Implemented] Native HTML test report email with customizable template.
  - `JQ`: [Implemented] Native JQ filtering for data transformation.
- **Shared File System (SFS):** [Implemented] Phase 2 complete. Supports
  S3-compatible parking (Minio/S3) and pluggable artifact backends (Artifact
  Registry) with cryptographic hash verification (SHA-256). K8s runner
  automatically handles unparking (Init Containers) and parking (Post-execution
  agent wrapper). It also includes **SFS Optimization**, allowing direct mounting
  of Persistent Volume Claims (PVCs) for high-performance ReadWriteMany data sharing
  without S3 overhead. Full CRUD API for storage backends and artifact destinations
  is available.
- **Native Junit ingest:** [Implemented] Support for collecting and persisting
  test reports (e.g., `junit.xml`) via the `reports` block in the DSL. Runner
  Agent automatically collects matching files, calculates SHA-256 checksums, and
  the engine persists them in the `step_test_reports` table.

### 4. Operations & Security

- **Authn and Authz:** [Implemented] OIDC/Dex support for SSO. OPA integration
  for fine-grained policy-as-code validation.
- **Transport Security (mTLS):** [Implemented] Mutual TLS support for all
  internal (NATS, Postgres, OPA) and external (S3, Webhooks) connections.
  Includes dynamic certificate reloading via filesystem watching for
  zero-downtime rotation.
- **CLI:** [Implemented] `stormchaser-cli` supports scheduling and viewing runs.
- **Log Management:** [Implemented] Integration with Loki and Elasticsearch for
  log retrieval and output scraping.
- **OpenTelemetry:** [Implemented] Tracing and metrics (e.g., `runs_enqueued`)
  are integrated into the API and Engine.
- **Step Status History:** [Implemented] Every granular state transition for a
  step is recorded in the `step_status_history` table and exposed via the API
  and TUI.
- **Mermaid rendering:** [Partial] Mentioned in docs as an external CLI
  integration; not built into the application code.

## Gap Analysis

The following high-priority features from `features.md` are currently missing or
require significant work:

1. **Workflow Templates & CronWorkflows:** Reusability of sub-workflows and
   periodic scheduling (via external systems like Kubernetes CronJobs or
   Ofelia).
2. **Step Memoization/Caching:** The ability to skip execution if inputs/code
   haven't changed.
3. **Step Optimization:** Logic to run multiple small steps (e.g., Python
   scripts) within a single container to reduce overhead.

## Conclusion

Stormchaser has a solid foundational architecture with a powerful DSL and a
scalable, event-driven orchestration engine. The core container execution on
Kubernetes is mature, and the security model (SSO/OPA) is well-integrated.
The engine supports a rich set of native steps including Webhooks, Email, Lambda,
and WASM. Recent focus has been on improving reliability, observability, and
advanced data handling (SFS/Artifacts).

---

### 🚀 Missing Features (Planned but not implemented)

The documentation identifies several gaps and planned features that are
currently missing from the implementation. These can be broken down into a few
main categories:

#### 1. Core Workflow & Engine Capabilities

- **Workflow Templates & CronWorkflows:** Reusability of sub-workflows and
  periodic scheduling.
- **Step Memoization/Caching:** The ability to skip execution if inputs/code
  haven't changed.
- **Advanced Retry Policies & Error Handling:** Exponential backoff/jitter for
  retries, and dedicated Error Handling Hooks (`On-Failure`/`Finally` blocks)
  for resource cleanup.
- **Step Optimization:** Logic to run multiple small steps within a single
  container to reduce overhead.
- **Input Validation:** Formal workflow input specification schemas and
  validation/querying abilities.

#### 2. Advanced Step Types & Integrations

- **Slack/ChatOps Integration:** Approvals and status updates directly via Slack or
  MS Teams.
- **Continuous Verification:** Steps with health metric monitoring and
  automatic rollbacks.
- **Sensors (Polling):** [Planned] Implementation of background polling mechanisms
  for external systems (Jira, GitHub, DB) to emit NATS events automatically.

#### 3. Execution Runner Enhancements

- **SFS Optimization:** Direct mounting of Persistent Volume Claims (PVCs) for
  high-performance ReadWriteMany data sharing.
- **Native WASM on Kubernetes:** Integrate `https://kwasm.sh/` for executing Wasm
  steps natively within the Kubernetes runner environment.

#### 4. Reliability & DX (Developer Experience)

- **Crash Recovery for Resolvers:** If an engine instance dies, another
  instance needs to gracefully take over workflows stuck in the `resolving`
  state on startup.
- **Dry-run and Linting Mode:** Validating workflows without triggering side
  effects.
- **Built-in Mermaid Rendering:** Currently requires an external CLI container;
  integration directly into the application is planned.