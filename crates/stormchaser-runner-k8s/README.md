# Stormchaser K8s Runner

The Stormchaser K8s Runner is a specialized agent responsible for executing `RunContainer` workflow steps as Kubernetes Jobs. It operates as a distributed, stateless worker that communicates exclusively via NATS with the Stormchaser Orchestration Engine.

## Orchestrator Interaction

The runner acts as a consumer of work dispatched by the orchestration engine. It does not require direct access to the Stormchaser PostgreSQL database.

### 1. Registration and Discovery

On startup, the runner generates a unique `RUNNER_ID` (if not provided via environment) and registers itself with the orchestrator by publishing a message to `stormchaser.runner.register`. This payload includes:

- `runner_id`: Unique identifier for the instance.
- `runner_type`: Always "k8s".
- `nats_subject`: The specific subject this runner listens to for targeted tasks.
- `step_types`: A list of supported step types (e.g., `RunContainer`) along with their JSON schemas generated from the `K8sJobSpec` model. This allows the orchestrator's DSL parser to validate K8s-specific parameters.

### 2. Liveness and Heartbeats

- **Heartbeats:** The runner publishes periodic heartbeats to `stormchaser.runner.heartbeat` every 10 seconds. The engine uses these to maintain the live runner registry.
- **Graceful Shutdown:** On `SIGTERM` (SIG15) or `SIGINT`, the runner publishes a `runner_offline` event to `stormchaser.runner.offline`. It initiates a graceful shutdown, stopping new task consumption and allowing time for final messages to be flushed.

### 3. Task Subscription

The runner subscribes to two NATS subjects:

- **Generic:** `stormchaser.step.scheduled.runcontainer` - Used for distributed load balancing. Multiple runners can subscribe to this subject to consume tasks in a work-queue fashion.
- **Specific:** `stormchaser.runner.k8s.<id>` - Used for tasks specifically routed to this runner instance (e.g., for session or cache affinity).

### 4. Reporting Results

Upon completion or failure of a task, the runner reports the outcome back to NATS:

- **Success:** `stormchaser.step.completed` with `run_id`, `step_id`, `status: "succeeded"`, and an `outputs` map containing execution metrics.
- **Failure:** `stormchaser.step.failed` with `run_id`, `step_id`, `status: "failed"`, the error reason, and the same `outputs` map.

## Implicit Step Variables

For every `RunContainer` step executed on Kubernetes, the runner automatically captures and exports the following variables in the `outputs` map:

| Variable Name | Description | Example |
|---------------|-------------|---------|
| `k8s exit code` | The final exit code of the container. | `0` |
| `Number of attempts` | Total container starts (1 + restarts). | `2` |
| `run duration` | Time from Job creation to completion. | `450ms` |
| `run latency` | Time from NATS message receipt to Job creation. | `12ms` |

These variables can be accessed in subsequent workflow steps using standard interpolation, e.g., `${steps.my_step.outputs["run latency"]}`.

## Kubernetes Runtime Interaction

The runner manages the lifecycle of Kubernetes Jobs using the `kube-rs` library.

### 1. Cluster Connection Pool

The runner maintains a `ClusterPool` of Kubernetes connections. While it defaults to the local cluster (using service account credentials or a kubeconfig), it is architected to support multi-cluster execution by dynamically acquiring clients for different target clusters.

### 2. K8sJobMachine

The core execution logic is encapsulated in the `K8sJobMachine` state machine:

#### Job Spec Building

The machine translates the Stormchaser `Step` metadata and its structured `spec` (of type `K8sJobSpec`) into a native Kubernetes `batch/v1.Job` object:

- **Container:** Image, command, args, and environment variables.
- **Resources:** CPU and Memory requests and limits.
- **Mounts:** Structured mounting of Kubernetes Secrets and ConfigMaps via the `secret_mounts` and `config_map_mounts` fields.
- **Labels:** Automatic tagging with `managed-by=stormchaser`, `stormchaser-run-id`, and `stormchaser-step-id` for tracking and reconciliation.
- **Annotations:** `stormchaser.io/received-at` and `stormchaser.io/step-dsl` are used to persist the original task context and timing for accurate reporting even after runner restarts.
- **Retry Policy:** Maps the DSL `retry` count directly to the K8s `backoffLimit`, allowing Kubernetes to handle container-level retries natively.
- **Minimum Version Spec:** The optional `minimum_version` field (defaults to `1.21.0`) allows steps to require specific Kubernetes API levels. The runner validates this against the cluster version before job creation.

#### Execution and Watching

Once the Job is created, the runner uses the Kubernetes `watcher` API to monitor the Job's status in real-time. It detects:

- **Success:** When the Job's `succeeded` count is greater than zero.
- **Failure:** When the Job's `failed` count indicates the `backoffLimit` has been reached, or a terminal error condition is met.

#### Cleanup

To maintain cluster health, the runner automatically deletes the Kubernetes Job object (using `Background` propagation) once the terminal state is reached and reported to the orchestrator.

### 3. Orphaned Job Adoption

One of the runner's most critical features is **Orphan Adoption**. On startup (after registration), the runner scans all known clusters for existing Jobs labeled with `managed-by=stormchaser`.

- If it finds Jobs that are still in progress, it "adopts" them by spawning a new `K8sJobMachine` instance to watch them.
- **State Restoration:** It uses the `stormchaser.io/step-dsl` and `stormchaser.io/received-at` annotations to perfectly restore the original step context and timing.
- This ensures that if a runner crashes or is restarted, the tasks it was managing are not left in a "zombie" state; the new runner instance will eventually pick them up and report their final status with accurate metrics.

### 4. Kubernetes Probes

The runner hosts a small HTTP server (Axum) on port `8080` for standard Kubernetes probes:

- `/healthz`: Liveness probe (returns 200 OK).
- `/readyz`: Readiness probe (returns 200 OK only after successful NATS connection and registration).

## Configuration

| Environment Variable | Description | Default |
|----------------------|-------------|---------|
| `NATS_URL` | URL of the NATS server | `nats://localhost:4222` |
| `RUNNER_ID` | Unique ID for this runner instance | Generated UUID v4 |
| `RUST_LOG` | Logging level | `stormchaser_runner_k8s=info` |
