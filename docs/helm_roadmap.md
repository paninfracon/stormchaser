# Helm & Kubernetes Deployment Roadmap

This document outlines the plan to build out a production-grade and
evaluation-friendly Helm ecosystem for Stormchaser.

**Current Status:** Phase 3 complete. Next up is Phase 4 (Advanced Execution
Planes).

## Phase 1: Umbrella Chart (The "One-Click" Install) ✅

**Goal:** Provide a single `stormchaser` Helm chart that deploys the entire
stack, including optional third-party dependencies, mirroring the convenience
of the `docker-compose` setup.

**Tasks:**

- [x] **Create `deploy/charts/stormchaser` (Umbrella Chart):**
  - Define a new `Chart.yaml` of type `application`.
  - Add local dependencies to `stormchaser-orchestration` and
    `stormchaser-runner-k8s`.
  - Add external dependencies to popular community charts: `postgresql`,
    `nats`, `minio`.
- [x] **Configure `values.yaml` Routing:**
  - Write a comprehensive `values.yaml` that dynamically routes connection
    strings (e.g., `global.database.url`, `config.natsUrl`) to the built-in
    dependencies if enabled.
- [x] **Database Migration Job:**
  - *Decision:* Rely on the `sqlx::migrate!` macro in the `stormchaser-api`
    startup sequence instead of a Helm hook, as it uses Postgres advisory
    locks safely.

## Phase 2: OPA Integration & Authorization

**Goal:** Ensure the system is usable out-of-the-box by providing the Open
Policy Agent (OPA) server and default policies.

**Tasks:**

- [x] **OPA Sidecar / Sub-chart:**
  - Modify the `stormchaser-orchestration` chart to optionally inject an OPA
    sidecar container into the `stormchaser-api` Deployment.
  - *Alternatively:* Create a lightweight `stormchaser-opa` chart that
    deploys a standalone OPA deployment and service.
- [x] **Policy Configuration:**
  - Add a `ConfigMap` template that mounts the default permissive
    `policy.rego` (from `deploy/opa/policy.rego`) into the OPA container.
  - Expose policy definitions in `values.yaml` so users can inject custom
    Rego rules via Helm values.

## Phase 3: Telemetry & Log Collection

**Goal:** Enable the API to stream and serve logs from Kubernetes Jobs back to
the CLI and TUI.

**Tasks:**

- [ ] **Create `deploy/charts/stormchaser-telemetry`:**
  - Deploy Grafana Alloy (or Promtail) as a `DaemonSet` using the official
    Grafana Helm charts as a base or dependency.
- [ ] **Kubernetes Discovery Configuration:**
  - Configure Alloy's `discovery.kubernetes` to specifically watch for Pods
    created by the K8s runner.
  - Write the specific `relabel_configs` required to extract the
    `stormchaser-run-id` and `stormchaser-step-id` annotations/labels from
    the Pod metadata and attach them as indexed labels in Loki.
- [ ] **Loki Dependency:**
  - Optionally add Grafana Loki to the Phase 1 Umbrella chart to complete the
    local evaluation logging stack.

## Phase 4: Advanced Execution Planes (Optional/Future)

**Goal:** Support edge cases like Docker-in-Docker or hybrid bare-metal/K8s
nodes.

**Tasks:**

- [x] **Create `deploy/charts/stormchaser-runner-docker`:**
  - Package the Docker runner as a `DaemonSet`.
  - Configure hostPath mounts for `/var/run/docker.sock` to allow the runner
    to spawn sibling containers on the Kubernetes worker nodes.
  - Add tolerations and nodeSelectors to restrict this runner to specific
    infrastructure nodes.
