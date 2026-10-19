# Execution Affinity in Stormchaser

Execution affinity says whether steps of a run should share an execution
environment (a container) or each get a fresh one. A step sets it in its
`strategy` block.

## Status

Only part of this design is implemented. What works today:

| Level | Intent | What happens today |
| :--- | :--- | :--- |
| `isolated` | **Must not share** | The default. Every step gets its own container or Job. |
| `shared` | **May share** | The engine sends all of a run's `shared` steps to the same runner. The Docker runner runs them in one long-lived container; the Kubernetes runner still starts a separate Job for each. |
| `locked` | **Must share** | Not implemented. Accepted by the parser, but handled like `isolated`. |

Any other value is also handled like `isolated`. The cleanup and reset rules
under [Security & Multi-Tenancy Constraints](#security--multi-tenancy-constraints)
are the target design; most are not implemented yet.

## DSL Usage

Affinity is a field of a step's own `strategy` block. It is not inherited: a
`strategy` on a group step (`Sequential`, `Parallel`) does not apply to the
steps inside it, so set it on each step that should share.

```hcl
step "lint" "RunContainer" {
    strategy {
        // "isolated" (default), "shared"; "locked" is not implemented
        affinity = "shared"
    }
    image   = "python:3.11"
    command = ["sh", "-c", "make lint"]
    next    = ["test"]
}

step "test" "RunContainer" {
    strategy {
        affinity = "shared"
    }
    image   = "python:3.11"
    command = ["sh", "-c", "make test"]
}
```

`tests/docker-affinity-scalability.storm` is a working example.

All of a run's `shared` steps form one group. There is no way to name separate
groups within a run.

## How `shared` works

### Routing (engine)

1. The first `shared` step of a run is dispatched like any other step, on
   `stormchaser.v1.<shard>.step.scheduled.<type>`, and whichever runner takes it
   becomes the group's runner.
2. If several `shared` steps are dispatched before any has a runner, the
   earliest one goes first and the others wait up to 30 seconds for its runner
   to be known.
3. Later `shared` steps are sent to that runner only, on
   `stormchaser.v1.runner.<kind>.<runner id>`, where `<kind>` is the kind the
   runner registered with (`docker` or `k8s`). A container step can land on a
   Kubernetes runner, so the kind comes from the runner's registration, not
   from the step type.
4. If the group's runner is not a known Docker or Kubernetes runner, the engine
   logs a warning and dispatches the step to any runner.

### Execution (Docker runner)

* The first `shared` step creates a container named `storm-shared-<run id>`
  from its image, with its storage mounts. The container runs `tail -f /dev/null`
  and each `shared` step, including the first, runs as a `docker exec` in it,
  with the step's `command`, `args` and `env`. The exec's output goes to the
  runner's own log (prefixed `[exec]`), not to Loki, so the API cannot show it
  as the step's logs and outputs cannot be extracted from it.
* Later `shared` steps run in that container whatever `image` they name, and
  their storage mounts are not added, since a running container cannot gain
  mounts. Give all of a run's `shared` steps the same image and mounts.
* The container is not removed when a step finishes. It is removed when the run
  is cancelled or aborted, or by the runner's hourly reaper once it is 24 hours
  old. A run that finishes normally leaves it running until then.

### Execution (Kubernetes runner)

Each step is still its own Job. `shared` only pins the run's steps to one
runner; nothing is reused.

## Security & Multi-Tenancy Constraints

The design below is the target. The status of each rule is noted.

1. **Run Isolation:** `shared` and `locked` are scoped to a single run.
   *Implemented for `shared`:* the shared container is named after the run id and
   the routing key is per run.
2. **No Cross-Workflow Sharing:** an execution environment must never be reused
   across runs, even with the same image or user. *Implemented,* as above.
3. **Post-Execution Cleanup:** when a run ends, its shared environments must be
   terminated and their local storage wiped. *Partly implemented:* only on
   cancel or abort, and otherwise after 24 hours (see above).
4. **Implicit Context Purge:** between steps in a shared environment the runner
   should reset it. *Not implemented.* Today:
    * **Process Reaping:** no processes are killed between steps. Background
      processes started by one step keep running into the next.
    * **Process Allow List:** `strategy.process_allow_list` is parsed but
      nothing uses it.
    * **Environment Reset:** each `exec` gets only the new step's `env`, but the
      container keeps the environment of the step that created it.
    * **Temp Cleanup:** `/tmp` and `/var/tmp` are not wiped.
    * **Filesystem Trust:** `shared` gives no filesystem sandbox. Everything a
      step writes stays for the next. Use `isolated` for steps that must not see
      each other's files.

## Competitive Comparison

How the design compares with other engines. It describes the intent; see
[Status](#status) for what exists.

### 1. vs. Argo Workflows (Isolation-First)

* **Argo:** Every step is a separate Kubernetes Pod. To lock steps together,
  you combine them into one `script` template.
* **Stormchaser:** Steps stay separate (e.g., `GitCheckout`, `RunLinter`), and
  `affinity` decides whether they share a container. On Kubernetes this is not
  implemented yet.

### 2. vs. Tekton (Static Grouping)

* **Tekton:** A `Task` is locked to a Pod; a `Pipeline` is isolated across Pods.
* **Stormchaser:** `affinity` is a per-step setting, so steps can move between
  `isolated` and `shared` without restructuring the workflow.

### 3. vs. GitHub Actions (Runner-Sticky)

* **GHA:** All steps in a `job` run on the same runner, with no way to isolate
  steps within a job.
* **Stormchaser:** Steps are `isolated` by default and can still share data
  through a `storage` volume.

## Why `shared`

`shared` treats the container as a cache. With `isolated`, each step pulls its
image (if needed), creates and starts a container, and removes it afterwards.
With `shared` on Docker, later steps only `exec` into a running container, which
saves container start-up time for runs with many short steps. The project has no
published measurement of the saving.
