# Resource Quotas in Stormchaser

Stormchaser provides a robust resource quota management system to prevent individual workflow runs from monopolizing cluster resources or exceeding organizational limits. Quotas are defined at the workflow level and enforced by the Orchestration Engine during execution.

## Defining Quotas

Quotas are defined within the `quotas` block of a `.storm` workflow definition file:

```hcl
workflow "DataProcessingPipeline" {
    quotas {
        max_concurrency = 10       // Maximum number of steps running simultaneously
        max_cpu         = "4.0"    // Maximum aggregate CPU cores across all running steps
        max_memory      = "8Gi"    // Maximum aggregate memory across all running steps
        max_storage     = "50Gi"   // Maximum shared storage space (SFS) allowed
        timeout         = "2h"     // Maximum total duration for the workflow run
    }
    // ...
}
```

## How Quotas Are Enforced

The Stormchaser Engine enforces quotas using a central, database-backed tracking system to ensure consistency even in a distributed, multi-controller deployment.

### 1. Concurrency Limits (`max_concurrency`)

The `max_concurrency` limit is the simplest form of quota. It restricts the absolute number of `StepInstance`s that can be in the `Running` state at any given time for a specific workflow run.

Before dispatching new steps, the Engine counts the active steps. If the count meets or exceeds `max_concurrency`, no further steps are dispatched until an active step completes, fails, or aborts.

### 2. Resource Limits (`max_cpu` and `max_memory`)

Unlike concurrency, which just counts steps, the CPU and Memory quotas track the *aggregate requested resources* of all currently running steps. This allows you to run many small steps concurrently, or fewer large steps, safely within a defined boundary.

#### Resource Requirement Parsing

When a step is defined (typically a `RunContainer` or `RunK8sJob`), it can specify its resource requirements in its `spec`:

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
* **Memory:** Parsed into an integer number of bytes. `512Mi` becomes `536,870,912`, `1Gi` becomes `1,073,741,824`.

#### The Claim and Release Cycle

1. **Evaluation:** When the Engine identifies a pending step that is ready to run, it extracts its CPU and Memory requirements.
2. **Claim:** The Engine attempts to atomically "claim" this quota against the run's current usage in the database.
3. **Dispatch:**
    * If the claim is **successful** (i.e., `current_usage + step_requirement <= max_quota`), the step is dispatched to a Runner.
    * If the claim is **unsuccessful** (i.e., dispatching this step would exceed the `max_cpu` or `max_memory`), the step remains in the `Pending` state. The Engine will try to dispatch other, smaller pending steps if they fit within the remaining quota.
4. **Release:** When the step finishes (either successfully or with an error), the Engine hooks into the completion event and subtracts the step's requested resources from the run's current usage, freeing up space for pending steps.

### 3. Timeout Enforcement (`timeout`)

The `timeout` quota is enforced by a background process in the Orchestration Engine. It periodically scans active runs. If a run's duration exceeds its specified timeout, the Engine automatically transitions the run to the `Failed` (or `Aborted`) state and attempts to gracefully terminate any active steps associated with it.

### 4. Storage Limits (`max_storage`)

The `max_storage` limit applies to the Shared File System (SFS). Enforcement of this limit is typically handled by the underlying storage provisioner or the Runner Agent, which monitors the size of the mounted volumes during execution. If a step attempts to write data that pushes the volume size beyond this limit, the write operation will fail, resulting in a step failure.

## Best Practices

* **Always Define Quotas:** It is highly recommended to define baseline quotas for all production workflows to prevent accidental runaway processes, especially when using dynamic features like `iterate` (Map/Reduce).
* **Step Granularity:** Be explicit about the CPU and Memory requirements of individual steps. If a step does not declare its requirements, it is assumed to require 0 CPU and 0 Memory for quota calculation purposes, which bypasses the aggregate limits and could lead to cluster resource exhaustion.
* **Balancing Concurrency and Resources:** Use `max_concurrency` to limit API rate limits or connection pools, and use `max_cpu`/`max_memory` to limit compute resource consumption.
