# Stormchaser Workflow DSL Specification

This document defines the Stormchaser Workflow DSL (SWD). The DSL is designed for human readability, machine parsability, and robust version control (git) compatibility. It explicitly rejects whitespace-based scoping (like YAML) in favor of explicit delimiters.

## Preamble

Every workflow file MUST start with a DSL version declaration. This version determines the schema validation rules and available features for the workflow.

```hcl
stormchaser_dsl_version = "1.0"
```

### Version Compatibility & Migration

The Stormchaser engine is designed with **Backwards Compatibility** as a core principle:

* **Version Mapping:** The engine can parse and execute older DSL versions by internally mapping them to the current internal representation (AST).
* **Automatic Upgrades:** When an older version is detected, the engine applies internal transformations to ensure the workflow behaves as expected on the modern runtime.
* **Deprecation Warnings:** If a workflow uses an older DSL version, the engine will emit a `DEPRECATION` warning in the workflow's execution log, encouraging authors to upgrade to the latest schema to access new features.

## Grammar Design Principles

1. **Unified HCL:** The DSL uses HCL (HashiCorp Configuration Language) for both structural components (blocks, labels, and keys) and logic evaluation.
2. **Explicit Scope:** Use curly braces `{ }` for blocks.
3. **No Indentation Sensitivity:** Indentation is for humans; the parser ignores it.
4. **Type Safety:** Values are evaluated as native HCL types (string, number, bool, list, map, etc.).

---

### 1. The HCL Expression Language

To ensure consistency, Stormchaser follows standard HCL expression and template rules:

* **Literals:** Standard HCL literals are used for basic types.
  * `count = 5` (Number)
  * `enabled = true` (Boolean)
  * `tags = ["web", "prod"]` (List)
* **Expressions:** Values can be complex HCL expressions.
  * `replica_count = inputs.min_replicas + 2`
  * `is_prod = inputs.env == "production"`
  * `status = var.enabled ? "active" : "disabled"` (Conditionals)
* **Strings & Interpolation:** HCL supports template interpolation within double-quoted strings using the `${}` sequence.
  * `image = "my-app:${inputs.tag}"`
* **Templates:** Complex logic can be embedded in strings using `%{ if ... }` and `%{ for ... }` directives.
* **Escaping:** To use a literal `${` in a string, escape it as `$${`.

---

### 2. Workflow Definition

The root object of a `.storm` file.

```hcl
workflow "DeployProduction" {
    description = "Standard production deployment pipeline"

    // Optional: Schedule the workflow to run automatically using an external cron engine.
    // Standard cron expression format: "min hour day_of_month month day_of_week"
    cron = "0 2 * * *"

    // Global Strategy: Defines default execution behavior for all steps.
    // Group-level strategy blocks will shadow these values.
    strategy {
        affinity = "isolated"
        fail_fast = true
    }

    // Import step libraries from external sources (Git, Registry, etc.)
    libraries {
        import "slack" {
            source = "github.com/stormchaser-plugins/slack"
            version = "v2.1.0"
            // The SHA256 of the canonical source archive for this version
            checksum = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        }
    }

---

### 1.1 Library Integrity
When a library is imported, the Control Plane performs the following:
1.  **Download:** Fetches the source archive from the provider (Git, Registry, etc.).
2.  **Verify:** Calculates the SHA256 of the downloaded archive and compares it to the `checksum` field in the DSL.
3.  **Cache:** If verification passes, the library is extracted and cached locally in the Control Plane.

If the checksum is missing or mismatched, the workflow compilation will fail during the **Validation and Safety** phase.

    // Configuration blocks...
    inputs { ... }
    outputs { ... }

    // Resource Quotas: Define limits for the entire workflow execution
    quotas {
        // ... (existing quotas)
    }

    // Lifecycle Handlers: Global execution hooks
    // These run after all steps in the 'steps' block have completed or failed.

    // on_failure runs ONLY if the workflow state is 'failed' or 'aborted'.
    on_failure {
        step "notify_slack" "slack/slack_alert" {
            params {
                channel = "#ops-critical"
                message = "Workflow ${run.name} failed! Error: ${run.error}"
            }
        }
    }

    // finally runs ALWAYS, regardless of the workflow's final status.
    // It executes AFTER on_failure (if triggered).
    // NOTE: Step failures within a 'finally' block are treated as HARD FAILURES.
    // They will be logged as errors, and the overall run status will be marked
    // as 'failed' (even if the main 'steps' succeeded) to ensure visibility
    // of failed cleanup operations.
    finally {
        step "cleanup_temp_env" "aws/LambdaInvoke" { ... }
    }

    // Event Handlers: Listen for events throughout the workflow's lifecycle
    handlers { ... }

    steps { ... }
}
```

---

### 3. Inputs and Schemas

Inputs define the contract for triggering the workflow.

```hcl
inputs {
    input "service_name" {
        type = "string"
        description = "Name of the microservice"
        validation = "regex('^[a-z-]+$')"
    }

    input "replica_count" {
        type = "int"
        default = 3
        range = [1, 10]
    }

    input "environment" {
        type = "enum"
        options = ["staging", "production"]
        query = "sql('SELECT name FROM envs WHERE active = true')"
    }
}
```

#### 3.1 Type System

Stormchaser uses a type system derived from the **HCL Expression Language**. All inputs, variables, and outputs must belong to one of the following types:

| Type | Description | DSL Literal Example |
| :--- | :--- | :--- |
| `string` | UTF-8 encoded text | `"hello"`, `"v1.2.0"` |
| `int` | 64-bit signed integer | `42`, `-10` |
| `uint` | 64-bit unsigned integer | `100u` |
| `double` | 64-bit floating point | `3.14`, `-0.5` |
| `bool` | Boolean value | `true`, `false` |
| `bytes` | Byte sequence | `b"binary-data"` |
| `timestamp`| RFC 3339 formatted date/time | `timestamp("2024-01-01T00:00:00Z")` |
| `duration` | Time duration | `duration("5m")`, `duration("1h")` |
| `list` | Ordered collection of any type | `["a", "b", "c"]`, `[1, 2, 3]` |
| `map` | Key-value associative array | `{"key": "value", "id": 101}` |
| `enum` | Restricted string (Stormchaser specific) | `options = ["prod", "stage"]` |
| `null` | Represents the absence of a value | `null` |

#### 3.2 Type Conversion

HCL provides built-in functions for explicit casting between types (e.g., `int("123")`, `string(42)`).

#### 3.3 Dynamic Secret Masking

In addition to DSL-defined secrets, steps can dynamically register sensitive values during execution by emitting a special command to `stdout`:

`::add-mask::<secret_value>`

When the Runner Agent detects this sequence:

1. The `<secret_value>` is immediately added to the **Sensitive Values Registry** for the current run.
2. All subsequent logs (from the same step or future steps) will redact any occurrences of this value.
3. The `::add-mask::` line itself is scrubbed from the final log persistence.

---

### 4. Steps

Steps are the atomic units of execution. Every step has a unique name and a type.

#### 4.1 Aliases

Aliases allow users to map short, human-readable names to long variable paths. Aliases defined in a wider scope (workflow or group) are inherited by all child steps unless shadowed by a local definition.

```hcl
aliases {
    // Map 'build' to a deeply nested step path
    "build" = "steps.setup.group.build"
    "env_name" = "inputs.environment"
}
```

#### 4.2 Storage and Artifacts

Workflow-level, step-level, and group-level (Sequential/Parallel) storage definitions control the shared file system.

```hcl
storage "workspace" {
    size = "10Gi"
    backend = "my-oci-registry" // Optional: Route artifacts to a specific backend by name

    // I/O Speed Governor: Prevent this workflow from starving others
    // on shared infrastructure (EFS/NFS/Shared SSD).
    limits {
        max_throughput = "100MiB/s"; // Sustained read/write speed
        max_iops       = 1000;       // Maximum I/O operations per second
    }

    // Provisioning: Fetch external resources into this storage space
    // BEFORE the steps start executing.
    provision {
        // Fetch from platform-native config providers (K8s ConfigMap, ECS Params)
        config "app_settings" {
            source = "prod-env-config"
            destination = "config/settings.json"
        }

        // Fetch from a public or private URL
        download "utility_script" {
            url = "https://internal.acme.com/scripts/setup.sh"
            destination = "scripts/setup.sh"
            mode = "0755"
            checksum = "sha256:e3b0c442..."
        }

        // Pull in artifacts from other runs or storage declarations
        artifact "previous_binary" {
            from = "build_workflow.workspace.binary"
            destination = "bin/old_app"
        }
    }

    // Glob patterns to keep on the shared FS between steps
    preserve = [
        "src/**",
        "node_modules/**",
        "target/release/app"
    ]

    // Files to publish to long-term object storage
    artifact "binary" {
        path = "target/release/app"
        retention = "30d"
    }

    artifact "logs" {
        path = "logs/*.log"
        retention = "7d"
    }

    // When to keep the storage volume after execution
    // Options: "always", "on_failure", "on_success", "none"
    retainment = "on_failure"
}
```

#### 4.3 Mounting Storage in Steps

Defining a `storage` block at the workflow level creates the volume, but it must be explicitly mounted into a `RunContainer` or `RunK8sJob` step using the `storage_mounts` field.

```hcl
step "my_step" "RunContainer" {
    image = "alpine"
    storage_mounts = [
        { name = "workspace", mount_path = "/data", read_only = false }
    ]
    command = ["ls", "-la", "/data"]
}
```

#### 4.4 Using Library Steps

Imported steps are referenced by their library alias and name (e.g., `lib_alias/step_name`).

```hcl
step "notify_ops" "slack/slack_alert" {
    params {
        channel = "#ops-alerts"
        message = "Deployment for ${inputs.service_name} complete."
    }
}

step "cleanup_resources" "aws/LambdaInvoke" {
    params {
        function_name = "cleanup-temp-resources"
        payload = "${run.id}"
    }
}
```

#### 4.5 Base Step Structure

A `condition` can be applied to ANY step. If the condition evaluates to `false`, the step is skipped.

```hcl
step "fetch_source" "GitCheckout" {
    // HCL expression for conditional execution
    condition = "inputs.environment == 'production'"

    params {
        repo = "https://github.com/org/repo"
        branch = "${input.service_name}"
    }

    retry {
        count = 3
        backoff = "exponential"
        max_delay = "5m"
    }

    timeout = "10m"

    // If true, the workflow continues even if this step fails.
    // The step is marked as 'failed_ignored' in the DB.
    allow_failure = false

    // Routing: Support for both Pull (default) and Push (mapped) models.
    next = [
        "build_image",
        // "Push" model: Explicitly map variables to the next step's inputs/params
        step "notify_webhook" {
            map {
                "url" = "https://hooks.acme.com/v1"
                "message" = "Checkout complete for ${inputs.service_name}"
            }
        }
    ]

    on_failure = step "slack_alert" {
        map {
            "channel" = "#ops-alerts"
            "error" = "${run.error}"
        }
    }

    // Test Reports: specialized handling for test results (e.g. junit.xml)
    // These are collected by the runner and persisted in the orchestrator.
    reports {
        report "unit-tests" {
            path = "target/surefire-reports/*.xml"
            format = "junit"
        }
    }

    // Output Mapping: Extract data from step execution
    outputs {
        // Search stdout for a specific marker and parse subsequent lines as Dotenv
        extract "metadata" {
            source = "stdout"
            marker = "--- OUTPUT ---"
            format = "dotenv"
        }

        // Search stdout for a specific marker and parse subsequent lines as JSON
        extract "api_response" {
            source = "stdout"
            marker = "--- JSON ---"
            format = "json"
        }

        // Extract a single value using regex
        extract "version" {
            source = "stdout"
            regex = "Version: ([0-9.]+)"
            group = 1
        }

        // Read and parse a file from the shared file system
        extract "test_report" {
            source = "file('results.json')"
            format = "json"
        }

        // Extract a single value and register it as a secret for masking
        extract "session_token" {
            source = "stdout"
            regex = "SessionToken: ([a-zA-Z0-9._-]+)"
            group = 1
            sensitive = true; // Automatically adds to Sensitive Values Registry
        }
    }
}
```

#### 4.6 Parallel and Sequential Groups

Groups allow for nested execution logic and iteration.

```hcl
step "process_data" "Parallel" {
    // The collection to iterate over (HCL expression)
    iterate = "inputs.file_list"

    // Name of the variable for the current item
    as = "file"

    // Execution strategy
    strategy {
        max_parallel = 5
        fail_fast = true
        affinity = "shared"
    }

    // Optional: Aggregate results from all iterations into group-level outputs
    aggregation "built_images" {
        description = "A list of all successfully built image tags"
        // HCL expression with access to the 'iterations' collection
        value = "iterations.filter(i, i.steps.build.status == 'succeeded').map(i, i.steps.build.outputs.tag)"
    }

    aggregation "failure_count" {
        value = "iterations.filter(i, i.steps.build.status == 'failed').size()"
    }

    steps {
        step "start_db" "RunContainer" {
            image = "postgres:15"

            // Define processes that are permitted to survive the
            // reaping phase AFTER this step completes.
            process_allow_list = ["postgres"]

            next = ["run_tests"]
        }

        step "run_tests" "RunContainer" {
            image = "test-runner:latest"
            params {
                db_url = "localhost:5432"
            }
        }
    }
}
```

##### 4.6.1 Output Collection and Reduction (Map/Reduce)

When a step uses the `iterate` strategy, Stormchaser automatically collects the outputs from every iteration.

**Accessing Data:**

1. **Aggregated Outputs (Preferred):** If an `aggregation` block is defined, the result is available at `${steps.<group_name>.outputs.<agg_name>}`.
2. **Raw Iteration Data:** The full raw list is always available at `${steps.<group_name>.iterations}`.

Each iteration object contains:

* `input`: The value of the item being iterated over.
* `index`: The zero-based index.
* `steps`: A map of the outputs for each step within that iteration.

**HCL Macros for Aggregation:**
Aggregation blocks use HCL macros (`map`, `filter`, `exists`, `all`, `size`) to transform the `iterations` list into a simplified output.

#### 4.7 Human-in-the-Loop & Async Events (Approval / Wait)

Stormchaser provides two dedicated step types for pausing execution: `Approval` and `Wait`.

**Runtime Behavior:**

* **Execution Suspension:** When an `Approval` or `Wait` step is reached, the workflow's state is persisted, and active execution is **suspended** (status `WaitingForEvent`). All runner resources for this workflow are released to the pool.
* **Timeout:** The step-level `timeout` (e.g., `24h`) applies to the suspension period. If no correlated event or approval is received within this window, the step fails.
* **Re-activation:** Upon receipt of a matching NATS event (either via the API for approvals or webhook correlations), the Controller re-hydrates the workflow state, injects any provided inputs/payloads as step outputs, and schedules the next step.

##### 4.7.1 The `Approval` Step

The `Approval` step halts the workflow until a user explicitly approves or rejects it via the API or CLI. It supports RBAC constraints and dynamic inputs.

```hcl
step "manual_approval" "Approval" {
    spec {
        // Optional: List of authorized approvers (users or OPA groups)
        approvers = ["group:admins", "user:alice"]

        // Optional: Form inputs required from the human upon approval
        inputs = [
            {
                name = "deploy_env"
                type = "string"
                options = ["staging", "production"]
                default = "staging"
            }
        ]

        // Optional: Send an email notification when the step is reached
        notify {
            from = "ops@paninfracon.net"
            to   = ["admin@paninfracon.net"]
            subject = "Approval required for run ${run.id}"
            // The body can use 'approve_link' and 'reject_link' variables
            body = <<-EOT
                Hello,

                Workflow ${run.id} requires your approval.

                Click below to take action:
                - [Approve]({{ approve_link }})
                - [Reject]({{ reject_link }})
            EOT
            html = true
        }

        // Optional: Fail the step if not approved within this window
        timeout = "24h"
    }
    next = ["deploy_approved"]
}

```

*Note: Unauthenticated, encrypted approval links are also supported for use in emails, bypassing the need for a UI session. See `scripts/generate-approval-link.py`.*

##### 4.7.2 The `Wait` Step

The `Wait` step pauses the workflow until an external event (e.g., a GitHub webhook or Jira status change) arrives matching a specific correlation key/value pair.

```hcl
step "wait_for_github" "Wait" {
    spec {
        // The key to match in the incoming event payload
        correlation_key = "github.commit.sha"
        // The expected value
        correlation_value = "expected_sha_12345"
        timeout = "1h"
    }
    next = ["echo_event_received"]
}
```

##### 4.7.3 The `LambdaInvoke` Step

The `LambdaInvoke` step allows for the direct execution of AWS Lambda functions as a step in the workflow. This is executed by the engine itself and does not require a separate runner.

*Note: This step type is only available if the `aws-lambda` feature is enabled in the engine build.*

```hcl
step "invoke_my_lambda" "LambdaInvoke" {
    spec {
        function_name = "my-function-name"
        // Optional: JSON payload to pass to the function (HCL expression)
        payload = {
            "id" = "${run.id}"
            "env" = "${inputs.environment}"
        }
        // Optional: "RequestResponse" (default), "Event", or "DryRun"
        invocation_type = "RequestResponse"
        // Optional: Version or Alias
        qualifier = "prod"
        // Optional: AWS Region
        region = "us-west-2"
    }
    next = ["process_lambda_output"]
}
```

**Outputs:**
The response from the Lambda function is available as a JSON object in the `response` output of the step.
`${steps.invoke_my_lambda.outputs.response}`

##### 4.7.4 The `EmailSend` Step

The `EmailSend` step allows sending emails using SMTP. The email body is rendered using the **MiniJinja** template engine, with access to the full workflow context (`inputs`, `steps`, `run`).

*Note: This step type is only available if the `email` feature is enabled in the engine build.*

```hcl
step "send_notification" "EmailSend" {
    spec {
        from = "ops@paninfracon.net"
        to   = ["admin@paninfracon.net", "${inputs.on_call_email}"]
        cc   = ["audit@paninfracon.net"]
        subject = "Workflow ${run.id} completed"

        // MiniJinja template body
        body = <<-EOT
            Hello,

            The workflow {{ run.id }} has finished with status: {{ steps.deploy.status }}.

            Summary of artifacts:
            {% for name, info in steps.build.outputs.artifacts %}
              - {{ name }}: {{ info.remote_path }}
            {% endfor %}
        EOT

        html = false

        // Optional SMTP configuration (defaults to environment variables)
        smtp_server   = "smtp.paninfracon.net"
        smtp_port     = 587
        smtp_username = "stormchaser"
        smtp_password = "${secrets.SMTP_PASSWORD}"
    }
}
```

**Environment Variables for SMTP:**
If not provided in the `spec`, the engine will look for the following environment variables:

* `SMTP_SERVER` (default: `localhost`)
* `SMTP_PORT` (default: `25`)
* `SMTP_USERNAME`
* `SMTP_PASSWORD`

---

### 5. Expressions (HCL)

Stormchaser uses **HCL Expression Language** for all logic, conditions, and variable interpolation.

#### 5.1 Variable Resolution & Shadowing

When an expression references a variable without a fully qualified prefix (e.g., `${my_var}` instead of `${inputs.my_var}`), the engine resolves the name using a **"Most Specific Wins"** shadowing model.

The resolution order (from highest to lowest precedence) is:

1. **Iteration Local:** The `as` variable defined in an `iterate` block (e.g., `${file}`).
2. **Step Aliases:** Aliases defined explicitly within the current `step` block.
3. **Group Aliases:** Aliases inherited from the immediate parent `Parallel` or `Sequential` group.
4. **Workflow Aliases:** Aliases defined at the root `workflow` level.
5. **Step Outputs:** Data from previously completed steps (e.g., `${steps.setup.outputs.id}`).
6. **Workflow Inputs:** Trigger-time inputs (e.g., `${inputs.branch}`).
7. **Global Secrets & Env:** System-level values (e.g., `${secrets.DB_PASSWORD}` or `${env.HOME}`).

**Example of Shadowing:**

```hcl
workflow "Example" {
    aliases { "tag" = "inputs.global_tag"; }

    inputs { input "global_tag" { type = "string"; } }

    steps {
        step "build" "Parallel" {
            iterate = ["v1", "v2"]
            as = "tag"; // SHADOWS the workflow alias 'tag'

            steps {
                step "run" "RunContainer" {
                    // This resolves to the iteration value ("v1" or "v2")
                    // NOT the workflow input.
                    args = ["--version", "${tag}"]
                }
            }
        }
    }
}
```

#### 5.2 Common HCL Patterns

* `build.exit_code == 0` (where 'build' is an alias for 'steps.setup.group.build')
* `inputs.replica_count > 5 && env_name == 'prod'` (where 'env_name' is an alias for 'inputs.environment')
* `has(inputs.optional_flag) && inputs.optional_flag == true`

#### 5.3 Stormchaser Standard Library (HCL)

Stormchaser provides a set of built-in functions extending HCL to handle common workflow tasks.

**String Manipulation:**

* `uuid()`: Generates a unique v4 UUID.
* `slugify(string)`: Converts a string to a URL-safe slug (lowercase, dashes).
* `base64_encode(bytes)` / `base64_decode(string)`: Standard base64 operations.
* `json_encode(any)` / `json_decode(string)`: Converts objects to/from JSON strings.

**Path & Filesystem:**

* `path_join(list)`: Joins path components (e.g., `path_join(["/", "tmp", "source"])` -> `"/tmp/source"`).
* `file_exists(string)`: Checks if a file exists on the shared filesystem (Runtime only).
* `file_size(string)`: Returns the size of a file in bytes.

**Networking & URLs:**

* `url_parse(string)`: Returns a map of URL components (scheme, host, path, query).
* `is_valid_ipv4(string)` / `is_valid_ipv6(string)`: IP validation helpers.

**Workflow Context:**

* `is_retry()`: Returns `true` if the current step execution is a retry.
* `retry_count()`: Returns the number of times the current step has been retried.
* `env(string)`: Accesses an environment variable (Equivalent to `${env.NAME}`).
* `secret(string)`: Performs a dynamic secret lookup from the configured backend (e.g., Vault). Format is `secret("path#key")`.

**Example of Secret Lookup:**

```hcl
step "deploy" "RunContainer" {
    params {
        db_password = "${secret('kv/data/db#password')}"
    }
}
```

**Validation Macros:**

* `regex_match(string, pattern)`: Boolean check for regex compliance.
* `is_in_range(value, min, max)`: Numeric range validation.

---

### 6. Validation and Safety

Every workflow MUST undergo a validation phase before execution. The `stormchaser lint` CLI command (and the Control Plane's pre-run handler) performs the following checks:

1. **Grammar Validation:** Ensures the `.storm` file matches the Tree-sitter grammar specification.
2. **DAG Cycle Detection:** The workflow graph is validated as a Directed Acyclic Graph (DAG) for all explicit `next` mappings. If a cycle is detected, the workflow is rejected.
3. **Variable Resolution:** All `${inputs.*}`, `${steps.*}`, and `${secrets.*}` references are checked for existence and scope (including alias shadowing).
4. **Policy Compliance:** The OPA engine is queried to ensure the workflow structure and resource requirements comply with organizational rules.
5. **External Dependency Verification (Optional):** When the `--verify-external` flag is used, the linter performs non-destructive "pre-flight" checks to ensure the workflow is executable:
    * **Image Registry Check:** Verifies that all referenced container images exist and are accessible (e.g., via `docker manifest inspect`).
    * **Git Repository Check:** Confirms that all referenced Git repositories and branches are reachable (e.g., via `git ls-remote`).
    * **Library Integrity:** Validates that all `libraries` blocks have matching checksums and that the remote sources are available.
    * **OPA Policy Existence:** Verifies that the specific OPA policies required for the run are loaded and reachable in the OPA server.

6. **Checksum Generation:** To simplify the adoption of the mandatory `checksum` field in `libraries`, the linter provides a `--generate-checksums` flag.
    * **Behavior:** The linter fetches the remote source for any library missing a checksum, calculates its SHA256 fingerprint, and provides the value to update the DSL.
    * **Security Note:** Users SHOULD only use this flag when first adding a trusted library or when intentionally upgrading a version. Once generated, the checksum provides the "Lockfile" guarantee for all future executions.

If any check fails, the linter returns a non-zero exit code and a detailed report of the missing or unreachable dependencies.

---

### 7. Full Example

```hcl
stormchaser_dsl_version = "1.0"

workflow "ContainerBuildAndDeploy" {
    description = "Builds a docker image and deploys to K8s"

    inputs {
        input "image_tag" {
            type = "string"
            description = "The tag to apply to the build"
        }
    }

    steps {
        step "checkout" "GitCheckout" {
            params {
                url = "git@github.com:acme/app.git"
            }
            next = ["build"]
        }

        step "build" "RunContainer" {
            image = "gcr.io/kaniko-project/executor:latest"
            storage_mounts = [
                { name = "workspace", mount_path = "/workspace" }
            ]
            args = [
                "--destination=myrepo/app:${inputs.image_tag}",
                "--context=dir:///workspace/source"
            ]
            next = ["deploy_prod"]
        }

        step "deploy_prod" "RunContainer" {
            image = "bitnami/kubectl:latest"
            args = ["set", "image", "deployment/myapp", "app=myrepo/app:${inputs.image_tag}"]

            retry {
                count = 5
                backoff = "linear"
            }

            on_failure = "rollback"
        }

        step "rollback" "RunContainer" {
            image = "bitnami/kubectl:latest"
            args = ["rollout", "undo", "deployment/myapp"]
        }
    }

    outputs {
        output "status" {
            value = "${step.deploy_prod.status}"
        }
    }
}
```

## Step Libraries

You can define reusable steps inside a workflow using the `step_library` block. When you define a step that uses the library name as its `type`, it will inherit the properties defined in the library. Properties defined on the step itself will be merged with the library (with the step's properties taking precedence).

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

## Template Workflows and Includes

You can mark a workflow as a template by using the `workflow_template` block instead of `workflow`.

```hcl
workflow_template "standard_build" {
  input "repo" { type = "string" }

  step "build" "RunContainer" {
    // ...
  }
}
```

Other workflows can then include this template using the `include` block. The steps from the included workflow will be merged into the caller, with step names prefixed to prevent collisions. Inputs provided in the include block will be injected into the included steps.

```hcl
workflow "main" {
  include "my_build" {
    workflow = "standard_build.storm"
    inputs = {
      repo = "my-repo"
    }
  }
}
```
