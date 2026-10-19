# Resource Quotas in Stormchaser

Quotas limit what one workflow run may use at once: how many steps run, how much CPU and memory they ask for together, and how long the run lasts. They are set per run and enforced by the engine when it dispatches steps.

## Defining Quotas

Quotas are defined within the `quotas` block of a `.storm` workflow definition file:

```hcl
workflow "DataProcessingPipeline" {
    quotas {
        max_concurrency = 10       // Maximum number of steps running simultaneously
        max_cpu         = "4.0"    // Maximum aggregate CPU cores across all running steps
        max_memory      = "8Gi"    // Maximum aggregate memory across all running steps
        max_storage     = "50Gi"   // Recorded, not enforced (see below)
        timeout         = "2h"     // Maximum total duration for the workflow run
    }
    // ...
}
```

A run started from workflow text (a direct run) takes its quotas from this block. A field left out gets the same default as below.
A run queued from a repository, a webhook rule or a cron schedule ignores the
block and gets fixed quotas: concurrency 10, CPU `1`, memory `4Gi`, storage `10Gi`
and timeout `1h` (or the `overrides.timeout` of the `POST /api/v1/runs` request).
A step that asks for more CPU or memory than its run's total fails at once (see
below), so such a run cannot have a step that asks for more than 1 CPU or `4Gi`.

### Operator ceilings

Quotas are chosen by whoever writes or starts the run, so on their own they limit
nothing. The engine (and, for the timeout override, the API) holds them to ceilings
the operator sets. A run that asks for more gets the ceiling, and a warning is logged.

| Ceiling | Default | Environment variable |
| :--- | :--- | :--- |
| `max_concurrency` | 50 | `STORMCHASER_RUN_MAX_CONCURRENCY` |
| `max_cpu` | `64` | `STORMCHASER_RUN_MAX_CPU` |
| `max_memory` | `256Gi` | `STORMCHASER_RUN_MAX_MEMORY` |
| `timeout` | `24h` | `STORMCHASER_RUN_MAX_TIMEOUT` |

The orchestration Helm chart has no values for these variables, so with the chart
the defaults apply.

What one step may use is limited separately, by each runner (see
[What one step may use](security.md#what-one-step-may-use)).

## How Quotas Are Enforced

The engine keeps each run's quotas and current usage in the `run_quotas` table, and locks the row when it changes usage, so several engine replicas stay consistent.

### 1. Concurrency Limits (`max_concurrency`)

Before dispatching, the engine counts the run's steps in the `running` state. If the count meets or exceeds `max_concurrency`, it dispatches nothing; otherwise it dispatches at most the difference. Steps that have been dispatched but have not started running yet (initializing, unpacking storage) are not counted.

### 2. Resource Limits (`max_cpu` and `max_memory`)

The CPU and memory quotas track the total that the run's dispatched steps *ask for*. They do not measure what steps actually use. Only `RunContainer` and `RunK8sJob` steps count; built-in steps such as `JQ` or `RestApi`, and Terraform steps, count as zero.

#### Resource Requirement Parsing

A `RunContainer` or `RunK8sJob` step gives its request in its `spec` (for `RunK8sJob`, `resources.requests.cpu` and `resources.requests.memory` take precedence when set):

```hcl
step "heavy_computation" "RunContainer" {
    spec = {
        image  = "data-processor:latest"
        cpu    = "1500m" // 1.5 cores
        memory = "2Gi"   // 2 Gigabytes
    }
}
```

The Engine parses these values into a normalized format:

* **CPU:** Parsed into a floating-point number of cores. `500m` becomes `0.5`, `2` becomes `2.0`.
* **Memory:** Parsed into an integer number of bytes. `512Mi` becomes `536,870,912`, `1Gi` becomes `1,073,741,824`. `K`, `M`, `G` and `T` are read as `Ki`, `Mi`, `Gi` and `Ti`. A value with a fractional number (`1.5Gi`) or an unknown suffix cannot be parsed and counts as zero; in a run's `max_memory`, zero means no memory limit. Write whole numbers (`1536Mi`).

#### The Claim and Release Cycle

1. **Evaluation:** When the engine picks up a pending step, it reads its CPU and memory request.
2. **Too large:** If the request alone is more than the run's `max_cpu` or `max_memory`, the step can never fit, so the engine fails it with a message saying so instead of leaving it pending.
3. **Claim:** Otherwise the engine adds the request to the run's usage, in one locked database update, if `current_usage + request <= max_quota` for both CPU and memory.
4. **Dispatch:**
    * If the claim succeeds, the step is dispatched to a runner.
    * If it does not, the step goes back to `Pending`, and the engine tries the next pending step, which may be smaller.
    * If dispatching fails, the claim is given back and the step goes back to `Pending`. A dispatch that was lost later is given back in the same transaction that puts the step back.
5. **Release:** When the step completes or fails (including a failed attempt that its retry policy runs again), the engine subtracts its request from the run's usage.

### 3. Timeout Enforcement (`timeout`)

A background worker in the engine (on the one replica that holds the background-worker lock) checks active runs every 30 seconds. The clock starts when the run started, or when it was queued if it has not started, so time spent queued counts. When a run is past its timeout, the engine marks it `Aborted` with the error `Workflow timed out`, fails its unfinished steps, asks the runners to cancel them, publishes `run.aborted`, and archives the run.

Each step also has its own timeout, set by the step's `timeout` or the runner's default; see [What one step may use](security.md#what-one-step-may-use).

### 4. Storage Limits (`max_storage`)

Not enforced. `max_storage` is parsed and stored with the run's quotas, but nothing reads it: neither the engine, the runners nor the agent limit the size of a run's storage volumes. Volume sizes are whatever the runner and storage backend give them (Docker volumes, the size of a shared PVC, the object store's limits).

## Best Practices

* **Define Quotas:** Set quotas in workflows you run directly, especially ones that use `iterate`. Without them a direct run gets concurrency 10, 1 CPU and `4Gi` in all.
* **Step Requests:** Give each container step its CPU and memory. If a step does not declare its requirements, it counts as 0 CPU and 0 Memory against the run's quota, so only `max_concurrency` bounds how many such steps run at once. When it runs, the runner gives it its default CPU and memory (1 core and `2Gi` unless the operator changed them).
* **Balancing Concurrency and Resources:** Use `max_concurrency` to limit API rate limits or connection pools, and use `max_cpu`/`max_memory` to limit compute resource consumption.
