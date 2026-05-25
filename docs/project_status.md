# Stormchaser Project Status & Future Work

## 1. Executive Summary

Stormchaser has a solid foundational architecture with a powerful graph-based DSL and a scalable, event-driven orchestration engine. The core container execution on Kubernetes is mature, and the security model (SSO/OPA) is well-integrated. The engine supports a rich set of native steps including Webhooks, Email, Lambda, and WASM.

Recent focus has been on improving reliability, observability, and advanced data handling (SFS/Artifacts) alongside significant code quality refactoring. The documentation and testing standards have been rigorously verified and brought up to a high standard.

## 2. Recent Accomplishments & Completed Epics

* **Quality Pass & Refactoring:** Refactored multiple crates to adhere to the "500-line rule". Monolithic functions and classes were decomposed into smaller, focused modules (e.g., splitting TUI `app.rs` into specialized sub-modules).
* **Testing Remediation:** Successfully executed a full test remediation plan. Flakiness in API rate limit tests was resolved. "Fake" assertion-less tests and tests with bypassed logic (`SQL_OFFLINE`) were identified and fully remediated with meaningful assertions.
* **TUI Enhancements:** Replaced the unmaintained `ratatui-form` with a static JSON Schema approach (`schemaui`) for simple forms, and a Native Reactive Dependency Graph for complex workflow inputs handling SSE streams.
* **Web UI Introduction:** A new Web UI has been developed that includes an Admin interface, Reporting functionality, and Import/Export features. (Reporting logic is centralized in the API for cross-UI consumption).
* **NATS Schema Modernization:** All NATS messaging infrastructure has been modernized. Messages conform to CloudEvents, use an OCI Schema Registry, implement subject-based versioning, and use local caching to eliminate network bottlenecks.
* **Integrations:** Added native `JinjaRender`, `TestReportEmail`, and AWS SES backend steps.
* **Documentation Quality:** Documentation was reviewed and confirmed to accurately reflect Cargo feature flags, security models (OPA/Fail-Closed), state machines, and DSL grammar.

## 3. Implementation Overview

| Category | Status | Details |
| :--- | :--- | :--- |
| **Language** | Implemented | Rust project with workspace structure. |
| **Data Store** | Implemented | PostgreSQL for state and audit; NATS for event-driven orchestration. |
| **DSL** | Implemented | Graph-based DSL with Tree-sitter support and HCL expression evaluation. |
| **Execution** | Implemented | Distributed state machine (K8s and Docker runners). Includes Optimistic Concurrency Control (OCC), Global Concurrency Enforcement, and Resource Quota/Timeout enforcement. |
| **Auth** | Implemented | SSO (OIDC/Dex), Policy-as-Code (OPA) integration, and mTLS. |
| **File System** | Implemented | Shared File System (SFS) with S3-compatible parking, Artifact Registry, and PVC optimizations. |
| **Steps** | Implemented | Native support for K8sJob, Docker, Webhooks, Email (SMTP/SES), Lambda, WASM, JinjaRender, Test Reports, JQ, and Human-In-The-Loop approvals. |

## 4. Planned Features & Gap Analysis

The following features are planned for future development:

### Core Workflow & Engine Capabilities

* **Workflow Templates & CronWorkflows:** Reusability of sub-workflows and periodic scheduling.
* **Step Optimization:** Logic to run multiple small steps (e.g., Python scripts) within a single container to reduce overhead.
* **Step Memoization/Caching:** The ability to skip execution if inputs/code haven't changed.
* **Input Validation & Dynamic Schemas:** Complete implementation of workspace input schemas using the existing HCL embedded schema format. This includes expanding the AST, intercepting workflow executions for JSON schema payload validation, and hydrating forms via dynamic queries (e.g., SQL, AWS) to populate UI dropdowns dynamically.

### Advanced Step Types & Integrations

* **Slack/ChatOps Integration:** Approvals and status updates directly via Slack or MS Teams.
* **Continuous Verification:** Steps with health metric monitoring and automatic rollbacks.
* **Sensors (Polling):** Implementation of background polling mechanisms for external systems (Jira, GitHub, DB) to emit NATS events automatically.
* **Environment and Service abstractions:** For cross-context workflow reuse.

### Execution Runner Enhancements

* **Native WASM on Kubernetes:** Integrate `https://kwasm.sh/` for executing Wasm steps natively within the Kubernetes runner environment.

### Reliability & Developer Experience

* **Crash Recovery for Resolvers:** Graceful handover of workflows stuck in the `resolving` state if an engine instance dies.
* **Dry-run and Linting Mode:** Validating workflows without triggering side effects.
* **Built-in Mermaid Rendering:** Currently requires an external CLI container; integration directly into the application is planned.
* **Dynamic Schema Resolution via OCI Event Streams:** Consume event streams from the OCI schema registry to trigger dynamic schema cache updates, allowing instant validation rule changes without waiting for the background sync interval.

## 5. Blocked / Pending Upstream

* **TUI HCL Editor:** The goal to implement `ratatui-code-editor` for syntax-highlighted editing of `.storm` files in the TUI is currently **blocked**. The `ratatui-code-editor` crate (v0.0.3) hardcodes supported languages and does not expose a public API to inject a custom `tree_sitter::Language` instance (like `tree_sitter_hcl::language()`). We must wait for upstream support for dynamic language injection.

## 6. Actionable Maintenance Tasks

* **Coverage Syncing:** Manual test coverage statistics in documentation quickly drift. Automate the injection of coverage stats into the README/documentation via CI/CD.
* **Planned Features Tracking:** As features transition from "Planned" to "Implemented", ensure this status document and corresponding DSL documentation are updated simultaneously.
* **Upstream Blocks:** Monitor the `ratatui-code-editor` crate for the required `tree_sitter` language injection feature.
