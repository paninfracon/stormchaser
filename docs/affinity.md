# Execution Affinity in Stormchaser

Stormchaser introduces a unique scheduling concept called **Execution Affinity**. This allows workflow authors to define how steps within a group (`Sequential` or `Parallel`) should be scheduled relative to their execution environments (containers, VMs, etc.).

## Affinity Levels

Unlike many workflow engines that enforce a single model, Stormchaser provides three distinct levels of affinity:

| Level | Intent | Scheduling Behavior | Implementation Strategy |
| :--- | :--- | :--- | :--- |
| `isolated` | **Must not share** | Every step is guaranteed a fresh, clean execution environment. | **Standard Runner** |
| `shared` | **May share** | The engine attempts to reuse the same environment (e.g., container). | **Warm Sidecar/Agent** |
| `locked` | **Must share** | All steps are guaranteed to run in the same execution environment. | **Static Composition** |

---

## Competitive Comparison

Stormchaser's approach is designed to provide the flexibility that standard engines often lack.

### 1. vs. Argo Workflows (Isolation-First)

* **Argo:** Every step is a separate Kubernetes Pod. To "lock" steps together, you must combine them into a single, complex `script` template.
* **Stormchaser Advantage:** You can keep your steps modular and reusable (e.g., `GitCheckout`, `RunLinter`), then use the `affinity` toggle to run them in the same Pod without rewriting the templates.

### 2. vs. Tekton (Static Grouping)

* **Tekton:** A `Task` is locked to a Pod; a `Pipeline` is isolated across Pods. This is a hard architectural boundary.
* **Stormchaser Advantage:** `affinity` is a **policy toggle**. You can move from `isolated` to `locked` for a group of steps without changing the DSL structure or the step definitions themselves.

### 3. vs. GitHub Actions (Runner-Sticky)

* **GHA:** All steps in a `job` are locked to the same runner. There is no easy way to isolate steps within a job to prevent cross-contamination of the file system or environment variables.
* **Stormchaser Advantage:** Provides `isolated` steps within a single logical group, ensuring a clean slate for each task while still allowing them to share a common `storage` volume.

---

## The "Shared" (MAY) Optimization

The most unique capability in Stormchaser is the `shared` affinity. It treats the execution environment as a **reusable cache**.

If Step A and Step B both use the `python:3.11` image:

1. **Standard Engine:** Stop Container A -> Upload Logs -> Start Container B (Pull image, start networking, etc.).
2. **Stormchaser (Shared):** Keep Container A open -> Flush Logs -> Run Step B command in the same namespace.

### Performance Impact

For workflows with many small, sequential steps (e.g., a CI pipeline with 10+ linting/formatting checks), `shared` affinity can reduce total duration by **30-50%** by eliminating repeated container pull and startup overhead.

---

## Security & Multi-Tenancy Constraints

To ensure data integrity and prevent cross-contamination in a multi-tenant environment, Stormchaser enforces a strict **Workflow Execution Boundary**:

1. **Run Isolation:** The `shared` and `locked` affinity levels are scoped strictly to a **single workflow execution (Run ID)**.
2. **No Cross-Workflow Sharing:** An execution environment (e.g., a specific container or VM) MUST NEVER be reused across different workflows, even if they share the same image or originate from the same user.
3. **Post-Execution Cleanup:** Once a workflow completes (Success, Failure, or Abort), all execution environments associated with its `shared` or `locked` steps must be immediately terminated and their local storage wiped.
4. **Implicit Context Purge:** When moving between steps in a `shared` environment, the runner is responsible for an automated "Soft Reset" to prevent lateral movement or accidental data leakage:
    * **Process Reaping:** The Runner Agent MUST send a `SIGKILL` to all processes in the execution environment's process group (excluding the Agent itself and any processes explicitly matched by the **Process Allow List**).
    * **Process Allow List:** Users may define a list of process names or PID file paths that are permitted to survive the reap phase. This is intended for local sidecars (e.g., `postgres`, `redis`, `localstack`) that must persist across multiple steps in a `shared` or `locked` context.
    * **Environment Reset:** All environment variables from the previous step MUST be cleared. Only the system-standard variables and the specific inputs/secrets for the *new* step are injected.
    * **Temp Cleanup:** Common temporary directories (e.g., `/tmp`, `/var/tmp`) MUST be wiped between steps.
    * **Filesystem Trust:** Users are warned that `shared` affinity does not provide a filesystem sandbox. Modifications to the root filesystem (outside of `/tmp`) will persist. For absolute filesystem isolation, `isolated` affinity MUST be used.

---

## DSL Usage

Affinity is defined within the `strategy` block of a group step:

```hcl
step "ci_pipeline" "Sequential" {
    strategy {
        // Options: "isolated", "shared", "locked"
        affinity = "shared"
    }

    steps {
        step "lint" "RunContainer" { ... }
        step "typecheck" "RunContainer" { ... }
        step "test" "RunContainer" { ... }
    }
}
```
