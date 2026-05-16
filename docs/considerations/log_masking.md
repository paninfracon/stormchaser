# Real-Time Log Masking Consideration

## The Problem

Implementing dynamic secret masking (`::add-mask::<secret>`) is complex because Stormchaser delegates log shipping to external, user-managed agents (e.g., Promtail for Loki, Fluentd for Elasticsearch) running on the runner nodes.

If the Stormchaser Agent merely masks its own `stdout`, we only protect the logs flowing through the agent. However, external log shippers often tail container or pod logs directly. If a process inside the container bypasses the agent (or if the container orchestrator captures raw streams before the agent processes them), unmasked secrets could leak.

## Proposed Original Plan (For reference)

### 1. Agent-Side Real-Time Redaction (`stormchaser-agent`)

- Update `crates/stormchaser-agent/src/main.rs` in the `Run` command handler.
- Instead of using inherited stdout/stderr via `child.wait()`, set `.stdout(Stdio::piped())` and `.stderr(Stdio::piped())`.
- Spawn two async tasks to read lines from the child's `stdout` and `stderr`.
- Maintain a shared (e.g., `Arc<Mutex<Vec<String>>>`) list of `masks`.
- For each line read:
  - Check if it starts with `::add-mask::`. If so, extract the secret, add it to `masks`, write the updated `masks` array to `/tmp/stormchaser_masks.json`, and **do not print the line**.
  - If it does not start with `::add-mask::`, iterate through the current `masks` and replace any occurrences with `***MASKED***`.
  - Print the redacted line to the agent's actual stdout/stderr.

### 2. Runner Integration (`stormchaser-runner-docker` & `k8s`)

- After the container/job finishes execution, alongside retrieving `/tmp/stormchaser_test_reports.json` and `/tmp/stormchaser_storage_hashes.json`, attempt to read `/tmp/stormchaser_masks.json` (which contains a `Vec<String>`).
- If found and successfully parsed, pass these strings back to the engine.

### 3. Event Model Updates (`stormchaser-model`)

- Add `new_sensitive_values: Option<Vec<String>>` to `StepCompletedEvent` and `StepFailedEvent` in `crates/stormchaser-model/src/events.rs`.

### 4. Engine Database & Event Processing (`stormchaser-engine`)

- Create a new database function `append_sensitive_values(executor, run_id, new_values: Vec<String>)` in `crates/stormchaser-engine/src/db/runs.rs` to append to the existing Postgres array.
- Update `crates/stormchaser-engine/src/handler/step/events/completed/process.rs` and `failed.rs` to call `append_sensitive_values` if `new_sensitive_values` is present in the incoming event.

## Future Considerations

To truly guarantee log masking across any backend, we must either:

1. **Force proxying:** Mandate that all container logs flow through the engine API before reaching the backend (sacrificing performance and the benefits of local log shippers).
2. **Sidecar shippers:** Deploy our own dedicated log shipper sidecar that is aware of the dynamic `sensitive_values` state from the database.
3. **Backend rules:** Automatically provision dynamic log-processing rules in Loki/ES on the fly based on the active run contexts.
