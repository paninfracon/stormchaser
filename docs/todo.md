# Stormchaser K8s Runner Improvements

These suggestions focus on improving the reliability, observability, and diagnostic capabilities of the Kubernetes runner implementation.

## 1. Log Capture & Streaming

- [x] **Implementation:** Use the Loki WebSocket `tail` API via the `stormchaser-api` acting as an SSE proxy to stream container logs (stdout/stderr) during execution in real-time.
- [x] **Data Handling:** Ensure logs are properly queried and isolated per step instance using deterministic `job_name` labels in Loki.
- [x] **User Experience:** Enable debugging for user-defined scripts by exposing these logs in the CLI (`stormchaser-cli runs logs`) and preparing the API for Web UI consumption.

## 2. Refine Job Cleanup Strategy

- [x] **Job Persistence:** Stop immediately deleting failed jobs. This allows for manual inspection of the Pod/Job state via `kubectl`.
- [x] **Native TTL:** Instead of imperative `clean_up` calls in the runner, utilize the Kubernetes `ttlSecondsAfterFinished` field in `JobSpec` (introduced in K8s 1.23+).
- [x] **Garbage Collection:** Implement a background "reaper" in the runner to clean up truly orphaned jobs that exceed a certain age (e.g., 24 hours).

## 3. Enhanced Failure Diagnostics

- [x] **Container Status Inspection:** Extract the `reason` and `message` from `ContainerStatus` (e.g., `OOMKilled`, `ImagePullBackOff`, `CrashLoopBackOff`) when a job fails.
- [x] **Rich Error Reporting:** Map these Kubernetes-specific failure reasons to structured error types in the `stormchaser.step.failed` event.
- [ ] **Pre-flight Checks:** Validate image existence or pull secrets before dispatching the job to reduce common configuration errors.

## 4. Watcher & Stream Resilience

- [x] **Robust Reconnection:** Implement a retry/backoff strategy for the Kubernetes watch stream in `watch_job` to handle network blips or API server restarts.
- [x] **Status Polling Fallback:** If the watch stream disconnects and cannot be recovered, fall back to periodic polling (`Api::get`) of the Job status until completion.

## 5. Orchestrator-Runner Synchronization (Zombie Tasks)

- [x] **Adoption Verification:** Before adopting an orphaned job in `scan_for_orphans`, query the orchestrator (via a NATS request/response pattern) to confirm the step is still in a `PENDING` or `RUNNING` state.
- [x] **Conflict Resolution:** If the orchestrator has already marked the step as failed/timed out, clean up the orphaned Job without sending a completion event.

## 6. Shared File System & Artifact Management

- [x] **SFS Phase 1:** Implement S3-compatible parking/unparking with SHA-256 hash verification.
- [x] **Storage CRUD:** Implement API endpoints for managing storage backends.
- [x] **Phase 2 - Artifacts:** Implement pluggable artifact backends (OCI, JFrog).
- [x] **ORAS Integration:** Support pushing artifacts to OCI registries using ORAS protocol.
- [x] **SFS Optimization:** Implement direct mounting of PVCs (ReadWriteMany) for high-performance data sharing in compatible environments.

## 7. Concurrency & Quota Management

- [x] **Optimistic Concurrency Control:** Implement version-based OCC in `workflow_runs` to prevent race conditions during state transitions in a distributed engine.
- [x] **Global Concurrency Limits:** Implement enforcement of `max_concurrency` at the workflow run level, ensuring steps are queued until a slot becomes available.
- [x] **Resource Quota Enforcement:** Implement enforcement of CPU and Memory limits (as defined in `run_quotas`) at the runner level (e.g., K8s resource requests/limits).
- [x] **Timeout Enforcement:** Implement a background "timeout reaper" in the engine to automatically fail workflows that exceed their `timeout` quota.
