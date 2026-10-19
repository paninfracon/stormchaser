# Stormchaser Workflow DSL Specification

This document defines the Stormchaser Workflow DSL. A workflow is an HCL (HashiCorp Configuration Language) file, usually named `*.storm`. Blocks are delimited by braces, so indentation does not matter.

Features that are described but not implemented are marked **(Planned)**. The parser accepts some of them, so a workflow that uses them parses, but nothing acts on them.

## Preamble

A workflow file starts with a DSL version declaration:

```hcl
stormchaser_dsl_version = "1.0"
```

The version is recorded with each run. It is not required and not checked yet.

**(Planned)** Version compatibility: mapping older DSL versions onto the current model, and a deprecation warning in the run's log for a workflow that uses an older version.

## Limits

A workflow file may be at most 1 MiB. A file, or one expression in it, that nests too deeply (by default 16,384 brackets and operators in a file, 8,192 in one expression) is refused. One `iterate` may produce at most 1,000 step instances. `jq` programs have time and memory limits. All but the file size can be raised by the operator; see "Limits on What a Workflow Can Cost" in [security.md](security.md#8-limits-on-what-a-workflow-can-cost).

---

### 1. The HCL Expression Language

Values follow HCL's rules:

* **Literals:** `count = 5`, `enabled = true`, `tags = ["web", "prod"]`, `labels = { team = "infra" }`.
* **Interpolation:** a `${...}` sequence in a string is an expression: `image = "my-app:${inputs.tag}"`.
* **Templates:** `%{ if ... }` and `%{ for ... }` directives work inside strings.
* **Escaping:** write `$${` for a literal `${`.

Most values in a step are kept as written when the workflow is parsed and resolved when the step is scheduled, with the run's inputs, secrets and the outputs of steps that have finished (see [Variables](#51-variables)). A `${...}` that cannot be resolved, such as a shell variable `${HOME}`, is left as it is.

`condition` and `iterate` are different: each is one expression, written as a string without `${...}`, such as `condition = "inputs.environment == \"production\""`. A literal list also works for `iterate`.

---

### 2. Workflow Definition

The root of a `.storm` file is one `workflow` block (or `workflow_template`, see [Template Workflows and Includes](#template-workflows-and-includes)). Its label is the workflow's name.

```hcl
stormchaser_dsl_version = "1.0"

workflow "DeployProduction" {
  description = "Standard production deployment pipeline"

  // Limits for one run; see docs/quotas.md
  quotas {
    max_concurrency = 5
    timeout         = "2h"
  }

  aliases {
    env_name = inputs.environment
  }

  inputs {
    environment = string(enum(["staging", "production"]))
    required    = ["environment"]
  }

  storage "workspace" {
    size = "10Gi"
  }

  step_library "kubectl" {
    type = "RunContainer"
    spec {
      image = "bitnami/kubectl:1.31"
    }
  }

  steps {
    step "deploy" "kubectl" {
      spec {
        args = ["apply", "-f", "/workspace/manifests/${env_name}"]
        storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
      }
    }
  }
}
```

The workflow block takes:

| Attribute or block | Meaning |
| :--- | :--- |
| `description` | Free text. |
| `inputs { ... }` | The JSON schema of the run's inputs. See [Inputs](#3-inputs-and-schemas). |
| `query "<name>" { ... }` | A data source for input options. See [Dynamic Options](#31-dynamic-options-via-queries). |
| `quotas { ... }` | `max_concurrency`, `max_cpu`, `max_memory`, `max_storage` and `timeout` for the run, held to the operator's ceilings. See [quotas.md](quotas.md), which also says when the block is ignored. |
| `aliases { ... }` | Short names for expressions. See [Aliases](#41-aliases). |
| `storage "<name>" { ... }` | A volume steps can share. See [Storage](#42-storage-and-artifacts). |
| `step_library "<name>" { ... }` | A reusable step definition. See [Step Libraries](#step-libraries). |
| `include "<name>" { ... }` | Steps from another workflow file. See [Includes](#template-workflows-and-includes). |
| `steps { ... }` | The workflow's steps. See [Steps](#4-steps). |

These are parsed and stored with the run, but nothing acts on them yet:

* `cron`: **(Planned)** A cron expression in the workflow. Schedules are created with `stormchaser cron` (the API's cron schedules) instead.
* `strategy { ... }` at workflow level: **(Planned)** Defaults for every step. Only a step's own `strategy` is read (see [Iteration](#46-iteration)), and `fail_fast` and `process_allow_list` are not read anywhere.
* `library "<alias>" { source, version, checksum }`: **(Planned)** Importing step libraries from Git or a registry, verified against a SHA-256 checksum. Nothing is fetched; use `step_library` blocks in the workflow instead.
* `handler "<name>" { event_type, condition, action }`: **(Planned)** Event handlers.
* `on_failure { steps... }` and `finally { steps... }`: **(Planned)** Steps to run after the main steps when the run fails, and always. The parser accepts them and applies step libraries to their steps, but the engine never runs them.
* `output "<name>" { value = "..." }`: **(Planned)** Workflow outputs. The values are not evaluated.
* `input "<name>" { type, default, description, validation, options }`: an older way to declare inputs. The TUI's scheduling dialog reads these blocks to build a form, but the engine neither validates inputs against them nor fills in their defaults. Use `inputs { ... }`.

Blocks and attributes the parser does not know are ignored without an error.

---

### 3. Inputs and Schemas

The `inputs` block is a JSON schema, written in HCL. The engine fills in each property's `default` and validates the run's inputs against the schema before the run starts; a run whose inputs do not match fails. The TUI and the web UI build run forms from it.

An attribute whose value is a type function (`string()`, `integer()`, `number()`, `boolean()`, `array()`, `object()`, `map()`) is a property. Functions inside it are JSON schema keywords with one argument (`pattern("...")`, `minimum(1)`, `default(3)`, `enum([...])`, `format("email")`); a type function inside `array()` describes the items. Any other attribute is a keyword at the root of the schema (`required`, `title`, `allOf`, ...). `ui_order` sets the order of fields in forms.

```hcl
inputs {
  ui_order      = ["service_name", "environment", "replica_count"]
  service_name  = string(pattern("^[a-z-]+$"))
  environment   = string(enum(["staging", "production"]))
  replica_count = integer(default(3), minimum(1), maximum(10))
  dry_run       = boolean(default(true))
  required      = ["service_name", "environment"]
}
```

Inputs are read in expressions as `inputs.<name>`.

#### 3.1 Dynamic Options via Queries

A `query` block names a data source. An input refers to its results with `${queries.<name>}` inside `enum(...)`, and each result becomes an option. Query `params` must be strings; other values are dropped.

When the TUI or web UI shows a run form, the query service resolves the queries and puts the options into the schema. The UI tells the service which workflow the form is for, and the service reads the schema and queries from the workflow itself, so the queries that run are the workflow's and not whatever a client sends:

* `POST /api/v1/schema/hydrate` takes `connection`, `workflow_path`, `git_ref` and `inputs`, and reads the workflow from that Git connection's repository. The caller must be allowed to use the connection.
* `POST /api/v1/schema/hydrate-direct` takes `dsl` (the workflow text) and `inputs`, for direct runs. Whoever calls it is choosing the queries, so policy can refuse it to users who may run workflows but not write them (see `author_groups` in [OPA RBAC](opa-rbac.md)).

A user may have 4 hydrations in flight and the service 64 in all (`STORMCHASER_QUERY_MAX_HYDRATIONS_PER_USER`, `STORMCHASER_QUERY_MAX_HYDRATIONS`); one more is refused with `429 Too Many Requests`. Each hydration runs at most 8 of its queries at once.

The engine also runs the queries when a run is queued, and validates the inputs against the schema with their results.

##### Mock Data

Useful for testing or static lists.

```hcl
query "env_options" {
  type = "mock"
  params = {
    items = "dev,staging,prod"
  }
}

inputs {
  environment = string(enum("${queries.env_options}"))
}
```

##### SQL Database

Fetch options from a database connection.

```hcl
query "db_envs" {
  type = "sql"
  params = {
    connection = "my-postgres-db"
    query      = "SELECT name FROM environments WHERE active = true"
  }
}
```

The query must be a single statement that only reads. It runs in a read-only transaction that is rolled back, is stopped after 30 seconds, and may return at most 10,000 rows; the first column of each row becomes an option. Give the connection a database role that can read only what a form needs: a read-only transaction does not stop a function with side effects.

##### REST API

Fetch options from an HTTP endpoint. `jq_filter` can pick a list of strings out of a larger response.

```hcl
query "api_options" {
  type = "api"
  params = {
    url                  = "https://api.example.com/v1/options"
    method               = "GET"
    header_Authorization = "Bearer token"
    jq_filter            = ".data[] | .name"
  }
}
```

##### AWS Cloud Control

List AWS resources.

```hcl
query "aws_vpcs" {
  type = "aws_cloudcontrol"
  params = {
    type_name       = "AWS::EC2::VPC"
    region          = "us-east-1"
    assume_role_arn = "arn:aws:iam::123456789012:role/StormchaserQuery"
  }
}
```

#### 3.2 TUI Integration

The TUI renders run forms from the schema, with drop-down lists for enums. It fetches options from the query service and re-renders dependent fields as inputs change, without blocking its event loop.

#### 3.3 Types

Inputs are JSON values, and the schema's types are JSON schema types: `string`, `integer`, `number`, `boolean`, `array`, `object` and `null`. In expressions they are HCL values: strings, numbers, booleans, lists (tuples), maps (objects) and `null`.

#### 3.4 Secrets

A run of a workflow from a repository gets the secrets of the repository's SOPS file (`secrets.sops.yaml`, `secrets.sops.json`, `.sops.yaml` or `.sops.json`, or the file the run request names), decrypted by the engine. They are read as `secrets.<name>`. The `secret("path#key")` function reads a secret from Vault (see [Standard Library](#53-stormchaser-standard-library-hcl)). Every secret value is masked wherever the API returns run data or logs.

**(Planned)** Dynamic secret masking: a step printing `::add-mask::<value>` to have that value masked in later logs.

---

### 4. Steps

Steps are the units of work. Each step block has two labels, a name and a type:

```hcl
step "build" "RunContainer" {
  // attributes and blocks
}
```

A step name may contain only letters, digits, `_` and `-`. The type is a built-in step type (see [Step Types](#47-step-types)), a registered WASM step type, or the name of a `step_library`.

#### 4.1 Aliases

Aliases give short names to expressions. Workflow aliases are evaluated when a step's expressions are resolved, and a step's own `aliases` block adds to or replaces them for that step.

```hcl
aliases {
  env_name   = inputs.environment
  build_tag  = steps.build.outputs.tag
}
```

An alias's value is an expression. Written without quotes, as above, it is evaluated. A quoted string is evaluated only if it contains `${...}` (`env_name = "${inputs.environment}"`); `"inputs.environment"` would be the text itself. HCL attribute names cannot be quoted, so `"build" = ...` is a syntax error.

An alias that refers to a step's outputs has a value only once that step has finished.

#### 4.2 Storage and Artifacts

A workflow-level `storage` block declares a volume that steps can mount and share. Between steps, a volume's contents are kept by the runner (parked to object storage, or on a shared PVC on Kubernetes).

```hcl
storage "workspace" {
  size    = "10Gi"             // Defaults to 1Gi
  backend = "my-oci-registry"  // Optional: the connection artifacts are published to

  // Fetched into the volume before a step that mounts it starts
  provision {
    download "toolchain" {
      url         = "https://internal.example.com/toolchain.tar.gz"
      destination = "tools"
    }
  }

  // Files to publish to long-term storage
  artifact "binary" {
    path      = "target/release/app"
    retention = "30d"
  }
}
```

Provisioning:

* `download "<name>" { url, destination }` fetches `url` and unpacks it, as a gzipped tar archive, into `destination`, a directory relative to the volume. A `${...}` in `url` can use the run's inputs and secrets.
* `artifact "<name>" { destination }` fetches the artifact of that name published earlier in the same run, from an S3 backend, and writes it to `destination` as a file. `mode` sets the file's permissions (`"0755"`).
* **(Planned)** `config` (a platform config map or parameter), `checksum` verification, `mode` on a download, and `from` (an artifact of another run or storage) are parsed but not acted on.

Artifacts:

* An `artifact "<name>" { path }` block declares a file to publish. `path` is relative to the root of the volume. Only steps that list the artifact in their `artifacts` attribute publish it (see [Mounting Storage](#43-mounting-storage-in-steps)).
* `retention` (default `on_success`) is passed to the runner and stored with the artifact, but nothing deletes artifacts when it runs out. **(Planned)**

**(Planned)** These storage settings are not in the model and are ignored: `limits { max_throughput, max_iops }` (an I/O governor), `preserve` (globs to keep between steps; the storage mount's own `preserve` field is not read either) and `retainment` (when to keep the volume after the run).

#### 4.3 Mounting Storage in Steps

A step mounts declared storage with `storage_mounts` in its spec. `RunContainer`, `RunK8sJob`, `GitCheckout`, `TerraformPlan`, `TerraformApply` and file-mode `JQ` steps take it.

```hcl
step "my_step" "RunContainer" {
  spec {
    image   = "alpine"
    command = ["ls", "-la", "/data"]
    storage_mounts = [
      { name = "workspace", mount_path = "/data", read_only = false }
    ]
  }
  artifacts = ["binary"] // Optional: the artifacts this step publishes
}
```

A step publishes only the artifacts its `artifacts` list names, and only from
storage it mounts; without the list it publishes none. Each artifact is then
published once, by the step that produces it, rather than by every step that
mounts its storage.

`mount_path` must be an absolute path that is not a system location: `/`,
`/usr`, `/usr/local`, `/etc`, and anything at, above or inside `/bin`, `/sbin`,
`/lib*`, `/usr/bin`, `/usr/sbin`, `/usr/lib*`, `/usr/libexec`, `/usr/local/bin`,
`/proc`, `/sys`, `/dev` and `/stormchaser` are refused. Paths such as
`/workspace`, `/app`, `/usr/src/app` or `/etc/myapp` are fine.

An artifact's `path` is relative to the root of the storage volume it is
declared in, wherever a step mounts that volume. Storage is parked and artifacts
are published by a helper outside the step's own container (a separate container
on Docker, a sidecar in the pod on Kubernetes), after the step's command has
exited successfully.

##### OCI registries as artifact backends

A storage block whose `backend` names an `oci` connection publishes its artifacts
to that registry, as `<registry>/<run id>/<storage name>:<artifact name>`. The
connection's `config` takes:

| Key | Meaning |
| :--- | :--- |
| `registry` | Registry host, optionally followed by a path that every repository starts with (`ghcr.io/my-org`). |
| `username`, `password` | Credentials allowed to push there. They never leave the engine: at push time it exchanges them for a token limited to that run's repositories. |
| `plain_http` | `true` for a registry served without TLS. |

The registry must support token authentication (Harbor, GHCR, GitLab, Docker Hub
and the reference registry with a token service do). For one that only does basic
authentication, or issues tokens that are not per repository (ECR), artifacts are
not pushed unless the operator sets `STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS=true`
on the engine, which sends the stored username and password to the runner's helper
instead.

#### 4.4 External Step Libraries (Planned)

Referring to a step from an imported library as `<alias>/<step>` (for example `step "notify" "slack/slack_alert"`) needs `library` imports, which are not implemented. A step whose type is not a built-in type, a registered WASM type or a `step_library` in the workflow is never run, and its run waits until it times out. Define reusable steps with [`step_library`](#step-libraries).

#### 4.5 Base Step Structure

Every step takes the attributes and blocks below. Anything else in the step body, attribute or block, becomes part of the step's spec, so these are equivalent:

```hcl
step "a" "RunContainer" {
  image = "alpine"
}

step "b" "RunContainer" {
  spec {
    image = "alpine"
  }
}

step "c" "RunContainer" {
  spec = {
    image = "alpine"
  }
}
```

```hcl
step "unit_tests" "RunContainer" {
  // Skip the step unless this expression is true
  condition = "inputs.environment != \"production\""

  spec {
    image   = "rust:1.96"
    command = ["cargo", "test"]
  }

  // Values the spec can use as ${params.<name>}; see Step Libraries
  params {
    profile = "dev"
  }

  // Run the step again if it fails; see 4.5.1
  retry {
    count   = 3
    backoff = "exponential"
    delay   = "10s"
  }

  // Longest the step may run; container steps only, see 4.8
  timeout = "30m"

  // Let the run go on if this step fails
  allow_failure = false

  // Steps that start once this one has finished
  next = ["package"]

  // Test reports to collect; see 4.5.5
  reports {
    report "unit-tests" {
      path   = "target/nextest/junit.xml"
      format = "junit"
    }
  }

  // Values to take from the step's log; see 4.5.4
  outputs {
    output "version" {
      regex = "Version: ([0-9.]+)"
    }
  }
}
```

| Attribute or block | Meaning |
| :--- | :--- |
| `spec` | The step type's settings. See the type's page under [steps/](steps/). |
| `condition` | An expression, evaluated when the step would be scheduled. `false` skips the step (status `skipped`), and steps after it go ahead. A value that is not a boolean, or an expression that fails, is an error and the step is not scheduled. |
| `params { ... }` | Named values the spec can use as `${params.<name>}`. See [Step Libraries](#step-libraries). |
| `next` | A list of step names. See [Ordering steps](#452-ordering-steps). |
| `retry { ... }` | See [Retries](#451-retries). |
| `timeout` | The longest the step may run, such as `"30m"`. See [Step limits](#48-step-limits). |
| `allow_failure` | `true` lets the run go on when the step fails. The step is marked `failed_ignored`, and the run continues as if it had succeeded. A step whose runner is lost still fails its run. |
| `aliases { ... }` | Aliases for this step only. See [Aliases](#41-aliases). |
| `artifacts` | The artifacts this step publishes. See [Mounting Storage](#43-mounting-storage-in-steps). |
| `iterate`, `as`, `strategy { ... }` | See [Iteration](#46-iteration). |
| `outputs { ... }` | See [Outputs from logs](#454-outputs-from-logs). |
| `reports { ... }` | See [Test reports](#455-test-reports). |

**(Planned)** Not implemented, though some of them parse:

* `on_failure` on a step (a step to run when this one fails). An `on_failure = ...` attribute ends up in the spec, and nothing reads it.
* Steps inside `next`, with values mapped onto them (the "push" model, `next = [step "x" { map { ... } }]`). This is not valid HCL and does not parse; `next` takes step names only.
* A nested `steps` block (groups); see [Parallel](steps/Parallel.md) and [Sequential](steps/Sequential.md).
* `process_allow_list` (processes allowed to outlive a step).

##### 4.5.1 Retries

A step with a `retry` block that fails is run again, up to `count` more times, before its failure counts. Its run waits meanwhile, and the step shows as `pending` between attempts.

| Attribute | Required | Meaning |
|---|---|---|
| `count` | yes | How many times to run the step again after its first attempt fails, at most 100. |
| `backoff` | no | How the wait grows between attempts: `fixed` (the default) waits `delay` each time, `linear` waits `delay` times the retry's number, and `exponential` doubles the wait each time. |
| `delay` | no | The wait before the first retry. Defaults to `10s`. |
| `max_delay` | no | The longest any wait may be. No wait is longer than 24 hours. |

A `retry` block with no `count`, another attribute, an unknown `backoff`, a duration that does not parse or a `count` over 100 is an error when the workflow is parsed.

With `count = 3`, `backoff = "exponential"` and `delay = "10s"`, a step that keeps failing runs four times, 10, 20 and 40 seconds apart.

Every failure counts: a non-zero exit, a step-level `timeout`, a built-in step's error and a runner that is lost while running the step. A step that also sets `allow_failure` is retried first, and its failure is ignored once its retries are used up. The run's own `timeout` covers all attempts. `Approval`, `TerraformApproval` and `Wait` steps are never retried.

A retry runs the step from the start, as it was dispatched the first time. Only retry a step that is safe to run twice: a step that changes something outside the run, such as an `SqlExecute` or a `RestApi` call, may have done so before it failed. A step's logs hold every attempt, and its outputs and test reports are those of its last attempt.

On the Kubernetes runner, a step's Job gets `backoffLimit` 0, so Kubernetes does not retry the pod itself, unless a `RunK8sJob` sets `backoff_limit`.

##### 4.5.2 Ordering steps

When a run starts, every step that is not in some step's `next` list starts at once. When a step finishes (succeeded, skipped or `failed_ignored`), each step in its `next` list starts if every step that lists it in `next` has finished. A failed step fails the run, unless it sets `allow_failure`, and nothing after it starts.

A workflow in which every step is in some `next` list (a cycle with no way in) fails to start. Steps are matched by name among the workflow's top-level steps.

##### 4.5.3 Conditions

`condition` is evaluated once, with the run's inputs, secrets, workflow aliases and the outputs of steps that have finished. It cannot use the iteration item, `params`, or the step's own aliases.

##### 4.5.4 Outputs from logs

Built-in steps set their own outputs (see each step type's page). A container step's outputs come from its log, through `output` blocks:

```hcl
outputs {
  // Optional: only read the lines between these markers
  start_marker = "--- OUTPUTS ---"
  end_marker   = "--- END OUTPUTS ---"

  output "version" {
    regex = "Version: ([0-9.]+)"
    group = 1                       // The capture group; defaults to 1
  }

  output "build_info" {
    marker = "BUILD_INFO="          // Only lines that contain this
    regex  = "BUILD_INFO=(.*)"
    format = "json"                 // Parse the captured text as JSON
  }

  output "session_token" {
    regex     = "SessionToken: ([a-zA-Z0-9._-]+)"
    sensitive = true                // Store it as sensitive
  }
}
```

When the step succeeds, the engine reads up to 5,000 lines of its log and, for each `output`, finds the last line the `regex` matches (and that contains `marker`, if set). The capture group becomes the output, as a string, or parsed as JSON with `format = "json"`. An output whose regex matches nothing is not set. `source` may be `logs` (the default) or `stdout`, which mean the same.

Outputs are read as `steps.<name>.outputs.<output>`.

**(Planned)** Other sources, such as a file on the shared file system (`source = "file('results.json')"`), a `dotenv` format, and an output without a `regex`.

##### 4.5.5 Test reports

A `reports` block names test report files the runner collects after the step, even when it fails:

```hcl
reports {
  report "unit-tests" {
    path   = "target/surefire-reports/*.xml"
    format = "junit"        // The default
  }
}
```

Only reports declared here are stored. A report's files may total at most 32 MiB compressed, 64 MiB unpacked and 1,000 files; a larger report is dropped and the step's result is unaffected. `junit` reports are parsed into a summary and test cases, which the API, the UIs and the [TestReportEmail](steps/TestReportEmail.md) step read.

#### 4.6 Iteration

A step with `iterate` runs once for each item of a list, as separate step instances:

```hcl
step "build" "RunContainer" {
  iterate = "inputs.services"   // An expression that gives a list
  as      = "service"           // The item's name; defaults to "item"

  strategy {
    max_parallel = 5            // At most this many instances run at once
  }

  spec {
    image   = "builder:latest"
    command = ["build", "${service}"]
  }
}
```

`iterate` is evaluated once, when the step is scheduled; it must give a list of at most 1,000 items (`STORMCHASER_MAX_ITERATIONS`), and an empty list skips the step. Each instance resolves the spec and params with its item. `iterate_as` is another name for `as`. The step counts as finished when all its instances have, so steps in its `next` list wait for all of them.

A step's `strategy` block takes:

* `max_parallel`: instances that may run at once. Without it, all may (within the run's `max_concurrency`).
* `affinity = "shared"`: dispatch the step's container work to the runner that the first step with the same affinity ran on.
* `fail_fast`: **(Planned)** parsed, not read.

**(Planned)** Collecting iteration results. `aggregation` blocks (map/reduce expressions over an `iterations` collection) are not parsed into anything the engine reads, and there is no `steps.<name>.iterations`. Each instance stores its outputs under the step's name, so `steps.<name>.outputs` holds the outputs of one of the instances, and which one is not defined.

#### 4.7 Step Types

| Type | Runs on | Page |
| :--- | :--- | :--- |
| `RunContainer` | Docker or Kubernetes runner | [RunContainer](steps/RunContainer.md) |
| `RunK8sJob` | Kubernetes runner | [RunK8sJob](steps/RunK8sJob.md) |
| `GitCheckout` | A runner, as a `RunContainer` step | [GitCheckout](steps/GitCheckout.md) |
| `TerraformPlan`, `TerraformApply` | A runner, as `RunContainer` steps | [TerraformPlan](steps/TerraformPlan.md), [TerraformApply](steps/TerraformApply.md) |
| `JQ` | The engine, or a runner when it reads or writes files | [JQ](steps/JQ.md) |
| `JinjaRender` | The engine | [JinjaRender](steps/JinjaRender.md) |
| `RestApi` | The engine | [RestApi](steps/RestApi.md) |
| `WebhookInvoke` | The engine | [WebhookInvoke](steps/WebhookInvoke.md) |
| `SqlExecute` | The engine | [SqlExecute](steps/SqlExecute.md) |
| `EmailSend` | The engine | [EmailSend](steps/EmailSend.md) |
| `TestReportEmail` | The engine | [TestReportEmail](steps/TestReportEmail.md) |
| `SlackMessage` | The engine | [SlackMessage](steps/SlackMessage.md) |
| `TeamsMessage` | The engine | [TeamsMessage](steps/TeamsMessage.md) |
| `LambdaInvoke` | The engine | [LambdaInvoke](steps/LambdaInvoke.md) |
| `Wasm`, registered WASM types | The engine | [Wasm](steps/Wasm.md) |
| `Approval` | Waits for a person | [Approval](steps/Approval.md) |
| `TerraformApproval` | Waits for a person, as an `Approval` step | [TerraformApproval](steps/TerraformApproval.md) |
| `Wait` | Waits for a correlated event | [Wait](steps/Wait.md) |
| `Parallel`, `Sequential` | **(Planned)** | [Parallel](steps/Parallel.md), [Sequential](steps/Sequential.md) |

Some step types depend on engine features that are on by default: `EmailSend` and `TestReportEmail` (`email`, and `aws-ses` for SES), `LambdaInvoke` (`aws-lambda`), `SlackMessage` (`chatops-slack`) and `TeamsMessage` (`chatops-teams`). In a build without the feature, such a step fails, saying so.

`Approval` and `Wait` steps suspend the run's progress without holding any runner: the step waits in the `waiting_for_event` state until it is approved, rejected or correlated. Their `timeout` is not enforced; a step left waiting ends only when its run times out.

#### 4.8 Step limits

A runner applies limits to every container step (`RunContainer`, `RunK8sJob`, and the steps that become `RunContainer` steps), whatever its workflow asks for:

| Limit | Default | Runner setting |
| :--- | :--- | :--- |
| CPU for a step that sets none | 1 | `STORMCHASER_STEP_DEFAULT_CPU` |
| Memory for a step that sets none | 2Gi | `STORMCHASER_STEP_DEFAULT_MEMORY` |
| Most CPU a step may ask for | 8 | `STORMCHASER_STEP_MAX_CPU` |
| Most memory a step may ask for | 16Gi | `STORMCHASER_STEP_MAX_MEMORY` |
| Run time of a step that sets no `timeout` | 24h | `STORMCHASER_STEP_DEFAULT_TIMEOUT` |
| Longest `timeout` a step may set | 24h | `STORMCHASER_STEP_MAX_TIMEOUT` |
| Processes in a step's container (Docker) | 4096 | `STORMCHASER_STEP_PIDS_LIMIT` |
| Pods one step may run at once or ask to complete (Kubernetes) | 10 | `STORMCHASER_STEP_MAX_PARALLELISM` |

A step that asks for more than a maximum is refused. A step that runs past its `timeout` is stopped and fails.

Steps the engine runs itself do not use `timeout`; `RestApi` and `WebhookInvoke` have a per-request `timeout` in their spec, `Wasm` and inline `JQ` have their own limits, and the run's `timeout` covers everything.

---

### 5. Expressions (HCL)

#### 5.1 Variables

An expression can use these names:

| Name | Value |
| :--- | :--- |
| `inputs` | The run's inputs, with schema defaults filled in. |
| `secrets` | The run's decrypted SOPS secrets. See [Secrets](#34-secrets). |
| `run` | The run: `run.id`. |
| `steps` | Outputs of steps that have finished: `steps.<name>.outputs.<key>`. A step that has not finished, or set no outputs, is absent. |
| `params` | The step's params (in the spec only). |
| Aliases | Workflow aliases, and the step's own aliases, by name. |
| The iteration item | `item`, or the name `as` gives it (in the spec and params of an iterated step). |
| `queries` | Query results (in the `inputs` schema only). |

There is no fallback lookup: a bare name that is none of these is an error, and a `${...}` that fails is left as written. A step alias replaces a workflow alias of the same name, and the iteration item replaces an alias of the same name.

```hcl
workflow "Example" {
  aliases {
    tag = inputs.global_tag
  }

  steps {
    step "build" "RunContainer" {
      iterate = ["v1", "v2"]
      as      = "tag" // replaces the alias 'tag' in this step

      spec {
        image = "builder:latest"
        // "v1" or "v2", not the input
        args  = ["--version", "${tag}"]
      }
    }
  }
}
```

#### 5.2 Common HCL Patterns

* `inputs.replica_count > 5 && env_name == "prod"` (where `env_name` is an alias for `inputs.environment`)
* `contains(["staging", "production"], inputs.environment)`
* `coalesce(inputs.tag, "latest")`

HCL strings take double quotes only.

#### 5.2.1 `${...}` inside Jinja templates

Some fields are MiniJinja templates, rendered when the step runs: the `query` of
`SqlExecute`, the `template` of `JinjaRender` and `TestReportEmail`, the `body` of
`RestApi`, `WebhookInvoke` and `EmailSend`, and the `notify.body` of `Approval` and
`TerraformApproval`. A `${...}` in one of them is still evaluated first, but its value
goes into the template as text: an input holding `{{ ... }}` or `{% ... %}` is printed
as written, not run as template code. To work with a value in Jinja itself, refer to it
there (`{{ inputs.name }}`) rather than splicing it in.

Splicing a value into a `SqlExecute` query puts it in the SQL. Pass values in `params`
instead; see [SqlExecute](steps/SqlExecute.md).

#### 5.3 Stormchaser Standard Library (HCL)

These functions are available in every expression.

**Strings:**

* `upper(string)`, `lower(string)`: Change case.
* `trim(string, cutset)`: Removes leading and trailing characters that are in `cutset`.
* `trimspace(string)`: Removes leading and trailing whitespace.
* `split(separator, string)`: Splits a string into a list.
* `join(separator, list)`: Joins a list into a string. Items that are not strings are written as JSON.
* `replace(string, old, new)`: Replaces every occurrence of `old` with `new`.
* `slugify(string)`: Lower case, with each run of characters other than `a-z` and `0-9` replaced by `-`, and no `-` at either end.
* `uuid()`: A random version 4 UUID, new on each call.
* `regex_match(pattern, string)`: `true` if `pattern` matches somewhere in `string`. Note the order: the pattern comes first.

**Encoding:**

* `base64encode(string)`, `base64decode(string)`: Base64. Decoding fails if the result is not UTF-8.
* `jsonencode(any)`: A value as a JSON string.
* `jsondecode(string)`: A JSON string as a value.

**Collections:**

* `length(any)`: The length of a list, map or string (in bytes).
* `keys(map)`, `values(map)`: A map's keys or values as a list.
* `contains(list, value)`: Whether the list holds the value.

**Logic:**

* `coalesce(any...)`: The first argument that is neither `null` nor an empty string `""`.
  This matches [Terraform's `coalesce`](https://developer.hashicorp.com/terraform/language/functions/coalesce):
  only `null` and the empty string are skipped. Non-string values such as `0`, `false`, and empty
  collections (`[]`, `{}`) are returned as they are. With no such argument it is an error.

**Paths:**

* `path_join(list)`: Joins path parts with `/` (`path_join(["/", "tmp", "source"])` gives `"/tmp/source"`).

**Environment and secrets:**

* `env(string)`: Reads an environment variable of the engine process. Only names the operator has listed in `STORMCHASER_ENV_ALLOWLIST` (comma-separated) are readable; any other name is an evaluation error. The list is empty by default. A listed variable that is not set reads as `""`.
* `secret(string)`: Reads a secret from Vault, as `secret("path#key")`. Lookups need `VAULT_TOKEN` to be set on the engine (there is no default token), and only paths the operator has listed in `STORMCHASER_VAULT_ALLOWED_PATHS` (comma-separated; a trailing `*` matches a prefix) are readable. The list is empty by default. A value read this way is masked like other secrets.

```hcl
step "deploy" "RunContainer" {
  spec {
    image = "deployer:latest"
    env = [
      { name = "DB_PASSWORD", value = "${secret("kv/data/db#password")}" }
    ]
  }
}
```

**(Planned):** `file_exists`, `file_size`, `url_parse`, `is_valid_ipv4`, `is_valid_ipv6`, `is_retry`, `retry_count` and `is_in_range`.

---

### 6. Validation and Safety

A workflow is checked at these points:

1. **Parsing.** The parser refuses text that is not valid HCL, has no `workflow` or `workflow_template` block, has a step without a name and a type or with an invalid name, has an invalid `retry` block, a library param that is a list or a map, or a spec that uses a param nobody sets. It also enforces the [limits](#limits). `stormchaser lint`, `POST /api/v1/runs/direct` (`stormchaser run <file>`) and the engine all parse a workflow before using it.
2. **Schema.** `stormchaser lint` also validates the parsed workflow against the DSL's JSON schema (see [Validation & Linting](#validation--linting)).
3. **Policy.** When OPA is configured, the engine asks it whether the run may start, with the workflow and its inputs.
4. **Inputs.** The engine validates the run's inputs against the `inputs` schema before the run starts.
5. **At run time.** A workflow whose steps all appear in `next` lists fails to start. Each step type checks its own spec when the step runs.

**(Planned):**

* Checking that every `${inputs.*}`, `${steps.*}` and `${secrets.*}` reference exists, and detecting cycles in `next` that some step does lead into.
* `--verify-external`: checking that images, Git repositories and branches, library sources and OPA policies are reachable.
* `--generate-checksums`: computing the `checksum` of a `library` import.

---

### 7. Full Example

```hcl
stormchaser_dsl_version = "1.0"

workflow "ContainerBuildAndDeploy" {
  description = "Builds an image and deploys it to Kubernetes"

  inputs {
    image_tag = string()
    required  = ["image_tag"]
  }

  storage "workspace" {
    size = "5Gi"
  }

  steps {
    step "checkout" "GitCheckout" {
      spec {
        repo           = "https://github.com/acme/app.git"
        destination    = "/workspace/source"
        storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
      }
      next = ["build"]
    }

    step "build" "RunContainer" {
      spec {
        image = "gcr.io/kaniko-project/executor:latest"
        args = [
          "--destination=myrepo/app:${inputs.image_tag}",
          "--context=dir:///workspace/source"
        ]
        storage_mounts = [{ name = "workspace", mount_path = "/workspace" }]
        cpu    = "2"
        memory = "4Gi"
      }
      next = ["deploy"]
    }

    step "deploy" "RunContainer" {
      spec {
        image = "bitnami/kubectl:1.31"
        args  = ["set", "image", "deployment/myapp", "app=myrepo/app:${inputs.image_tag}"]
      }
      retry {
        count   = 5
        backoff = "linear"
      }
    }
  }
}
```

## Step Libraries

You can define reusable steps inside a workflow using the `step_library` block. A step that names the library as its `type` becomes a step of the library's type, with the library's `spec`, `params`, `timeout`, `allow_failure` and `retry`. Whatever the step sets itself takes precedence: its `params` replace the library's of the same name, and its `spec` is merged into the library's. Objects merge key by key, as deep as they go. Lists of named entries, such as `env`, merge by name: the step's entry replaces the library's of the same name, and its other entries are added. Any other value the step sets, such as `command`, replaces the library's.

```hcl
step_library "ubuntu_base" {
  type = "RunContainer"
  spec {
    image = "ubuntu:22.04"
  }
}

step "my_step" "ubuntu_base" {
  spec {
    command = ["echo", "hello world"]
  }
}
```

### Parameters

A library's `params` are its arguments, with the values it gives them by default. Its `spec` uses them as `${params.<name>}`, and a step that uses the library sets the ones it wants changed:

```hcl
step_library "python_env" {
  type = "RunContainer"
  params {
    python_version = "3.12"
    test_selection = "${inputs.selection}"
  }
  spec {
    image   = "python:${params.python_version}-slim"
    command = ["sh", "-c", "pip install pytest && pytest -k '${params.test_selection}'"]
  }
}

step "unit_tests" "python_env" {}

step "unit_tests_older" "python_env" {
  params {
    python_version = "3.11"
  }
}
```

* A param's value is a string, number or boolean; a list or a map is an error. Numbers and booleans become their text.
* A param's value can use the run's `inputs`, `secrets`, `steps` outputs and the iteration item, as a spec can. Params are resolved first, then the spec; a param cannot use another param.
* A spec that uses a param that neither the step nor its library sets is an error when the workflow is parsed, naming the step and the param.
* A step that uses no library can set `params` too, and its own `spec` can use them the same way.
* A step sets params with a `params { ... }` block. A library takes a block or a `params = { ... }` attribute.

## Template Workflows and Includes

A workflow can be marked as a template with `workflow_template` instead of `workflow`. It is parsed like a workflow; the mark is recorded.

```hcl
workflow_template "standard_build" {
  steps {
    step "build" "RunContainer" {
      spec {
        image   = "builder:latest"
        command = ["build", "${inputs.repo}"]
      }
    }
  }
}
```

Another workflow includes it with an `include` block:

```hcl
workflow "main" {
  include "my_build" {
    workflow = "builds/standard_build.storm"
    inputs = {
      repo = "my-repo"
    }
  }
}
```

When a run is queued from a repository, the engine reads each included file from the same commit (`workflow` is a path relative to the repository root), and adds its steps and step libraries to the workflow; other blocks of the included file are not used. Included steps are named `<include name>.<step name>`, and their `next` lists are renamed to match. Includes may include further files; each file is included once. In each included step's `params`, the text `inputs.<name>` is replaced by the include's value for `<name>`; the spec is not changed.

Direct runs (`stormchaser run <file>`) do not process includes.

## Validation & Linting

`stormchaser lint` parses a workflow and validates it against the DSL's JSON schema, offline.

### Generating the Schema

The JSON Schema (Draft 7) for the DSL can be generated locally:

```bash
stormchaser schema generate > schema.json
```

Or in the HCL form, with `--format`:

```bash
stormchaser schema generate --format hcl > schema.hcl
```

Or fetched from a running API server (`<host>` and `<port>` are your deployment's address):

```bash
curl https://<host>:<port>/api/v1/schema > schema.json
```

### Linting Workflows

```bash
stormchaser lint my-workflow.storm
```

#### Remote & Offline Validation

By default `lint` generates the schema locally. To validate against the live server's schema, use `--remote`:

```bash
stormchaser lint my-workflow.storm --remote
```

To validate offline against a server's schema, download it once:

```bash
# Downloads the live schema to .stormchaser-schema.json
stormchaser lint --prepare
```

Later `lint` runs find `.stormchaser-schema.json` in the current directory and validate against it.

#### Validating Custom Step Plugins

The linter can validate the specs of custom step types against extra schemas, from local JSON files or from local Git repositories (without a checkout):

```bash
stormchaser lint my-workflow.storm \
  --step-schema MyCustomStep=schema.json \
  --step-schema AnotherStep=git-local:///path/to/repo?ref=main&file=schemas/step.json
```

#### Pre-commit Hook Integration

The repository's own `.pre-commit-config.yaml` lints `.storm` files with this hook, which can be copied:

```yaml
repos:
  - repo: local
    hooks:
      - id: stormchaser-lint
        name: Stormchaser Linter
        entry: bash -c 'cargo build -p stormchaser-cli -q && for f in "$@"; do target/debug/stormchaser lint "$f" || exit 1; done' --
        language: system
        files: \.storm$
```
