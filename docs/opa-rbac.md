# OPA RBAC Policies

Stormchaser integrates with Open Policy Agent (OPA) out of the box to enforce API
access control and workflow execution policies.

By default, Stormchaser's OPA middleware passes the following context structure
into the policy evaluation under the `input` document:

```json
{
  "path": "/api/v1/runs",
  "method": "POST",
  "token": "eyJhbGc..."
}
```

> ⚠️ **Important:** `token` is `null` for unauthenticated requests (e.g., the login
> endpoint). Policies **must** guard against `null` before calling `io.jwt.decode`.
> The `ApiOpaContext` serializes `None` as JSON `null`, not as a missing field:
>
> ```json
> {
>   "path": "/api/v1/auth/login",
>   "method": "GET",
>   "token": null
> }
> ```

The `token` is the raw JWT access token provided by the client (if available).

> 💡 **Tip:** The fastest way to learn and debug Rego policies is the
> [Rego Playground](https://play.openpolicyagent.org/). You can paste your policy
> there, supply a mock `input` JSON, and instantly see what evaluates to true
> or false.

## 1. Enterprise Data-Driven RBAC

Hardcoding groups into your Rego policy (e.g., `"admin" in token_payload.groups`)
is fine for simple setups, but in an enterprise environment, identity provider
(IdP) groups change frequently.

OPA allows you to **decouple policy logic from organizational data**. See the
`docs/enterprise-rbac/` directory for a complete example.

You can provide a `roles.json` file to OPA:

```json
{
  "role_mappings": {
    "admin": ["Okta-Global-Admins", "EntraID-Platform-Owners"],
    "developer": ["Okta-Engineering", "EntraID-Developers"]
  }
}
```

And your Rego policy (`policy.rego`) simply checks this data map:

```rego
user_has_role(role_name) if {
    allowed_idp_groups := data.role_mappings[role_name]
    some user_group in token_payload.groups
    user_group in allowed_idp_groups
}
```

When your organization's Okta groups change, you only update the JSON data,
not the Rego code.

## 2. Policy Unit Testing (`opa test`)

The hardest part of writing Rego is figuring out why a rule evaluated to `false`.
OPA has a built-in testing framework that makes this easy.

See `docs/enterprise-rbac/policy_test.rego` for an example. You can write tests
that mock the `input` (like the API path and a fake JWT) and the `data` (like
the `roles.json` mapping).

Run the tests using the OPA CLI:

```bash
opa test docs/enterprise-rbac/ -v
```

This guarantees your authorization logic works before deploying it to the cluster.

## 3. ABAC (Attribute-Based Access Control)

Stormchaser's Orchestration Engine performs a secondary OPA check right before
executing a workflow step. At this point, the context is richer, allowing for
Attribute-Based Access Control. This `EngineOpaContext` is injected as:

```json
{
  "run_id": "e4b2d1c...",
  "initiating_user": "developer@example.com",
  "workflow_ast": { ... },
  "inputs": { ... }
}
```

### Concrete ABAC Examples

**Example 1: Restrict deployment target based on groups.**
Only allow users in the "ReleaseManagers" group to deploy to production.

```rego
allow if {
    # Check if the user is a Release Manager
    "ReleaseManagers" in token_payload.groups

    # Check if the workflow inputs have 'env' set to 'production'
    input.inputs.env == "production"
}
```

**Example 2: Prevent privileged containers.**
Deny any workflow attempting to run a Docker container with `privileged: true`
unless the user is a global admin.

```rego
deny if {
    # Is the step a RunContainer step?
    input.workflow_ast.step_type == "RunContainer"

    # Is privileged set to true in the AST?
    input.workflow_ast.config.privileged == true

    # Is the user NOT an admin?
    not user_has_role("admin")
}
```

## 4. OPA Cookbook (Helpful Snippets)

Here are some common copy-pasteable snippets for enterprise environments.

**MFA Enforcement**
Only allow access if the Identity Provider (IdP) confirms MFA was used.

```rego
allow if {
    # 'amr' (Authentication Methods References) claim usually contains 'mfa'
    "mfa" in token_payload.amr
}
```

**Email Domain Whitelisting**
Deny all access if the user's email does not end with your corporate domain.

```rego
allow if {
    endswith(token_payload.email, "@yourcompany.com")
}
```

**Approval Separation of Duties (SoD)**
Prevent the person who initiated a run from approving their own manual steps.

> **Note:** This rule is intended to be evaluated during engine execution for approval-step
> actions. The Engine OPA context (`EngineOpaContext`) includes `initiating_user` at the top
> level of `input`, whereas the API authorization context only contains `path`, `method`, and
> `token`. The `input.path` field is **not** available in the engine context, so this rule
> relies solely on `input.initiating_user`.

```rego
deny if {
    # Does the current user's email match the initiating user?
    # input.initiating_user is provided by the EngineOpaContext
    token_payload.email == input.initiating_user
}
```

## 5. Compiling Policies to WASM

For significantly higher performance (sub-millisecond evaluation times) and to
run policies completely locally without network hops, Stormchaser's API and
Orchestration Engine support executing pre-compiled OPA WebAssembly (WASM)
modules directly.

> **Note:** While WASM execution is incredibly fast, using OPA over HTTP
> (via `OPA_URL`) is generally more flexible. It is preferable for centrally
> managed RBAC solutions or in environments where policies may change
> frequently, as it allows policies to be updated without restarting the
> Stormchaser services.

### Compile the Rego Policy

You can use the `opa` CLI to compile your `.rego` file into a single WASM module.
You must specify the entrypoint (the primary rule you want Stormchaser to evaluate)
using the `-e` flag.

**Do I need multiple WASM files for multiple rules?**
No. A single entrypoint (like `stormchaser/allow`) acts as the "root" of the
evaluation tree. When Stormchaser asks OPA to evaluate `stormchaser/allow`, OPA
will automatically traverse and evaluate all other rules, functions, and data
references (e.g., `user_has_role`, `has_api_permission`, `engine_deny`) that
are invoked from within that root rule.

You only need one compiled `policy.wasm` file containing all of your RBAC and
ABAC logic, as long as it's all reachable from the single entrypoint you specify.

For example, if your entrypoint is `stormchaser/allow`, your Rego file might
look like this:

```rego
package stormchaser

# This is the "root" rule (entrypoint) evaluated by Stormchaser
default allow := false

allow if {
    # It evaluates to true if BOTH of these helper rules are true
    is_valid_domain
    has_permission
}

# Read the raw JWT string safely (token is null when absent).
raw_token := object.get(input, "token", null)

# Default to an empty payload when no token is present or the token is malformed.
default token_payload := {}

# Guard: check the token has basic JWT structure (3 non-empty dot-separated segments)
# before attempting to decode it, so malformed tokens result in deny rather than an
# OPA evaluation error (which would return 500 from the middleware).
has_jwt_structure if {
    is_string(raw_token)
    raw_token != ""
    segments := split(raw_token, ".")
    count(segments) == 3
    segments[0] != ""
    segments[1] != ""
    segments[2] != ""
}

# Decode the JWT and extract its payload (claims) only when the token
# passes the structural guard. Otherwise, the default empty payload is used.
token_payload := io.jwt.decode(raw_token)[1] if {
    has_jwt_structure
}

# --- Helper Rules ---

is_valid_domain if {
    # Custom logic here...
    endswith(token_payload.email, "@yourcompany.com")
}

has_permission if {
    # Evaluates to true if the user is an admin
    "admin" in token_payload.groups
}

has_permission if {
    # OR it evaluates to true if they are a developer reading runs
    "developer" in token_payload.groups
    input.method == "GET"
    startswith(input.path, "/api/v1/runs")
}
```

```bash
# Compile the entire policy targeting WASM.
# We set 'stormchaser/allow' as the root entrypoint.
opa build -t wasm -e stormchaser/allow docs/opa-rbac-example.rego

# The build outputs a bundle.tar.gz file. Extract it to get the single policy.wasm
tar -xzf bundle.tar.gz /policy.wasm
```

### Configure Stormchaser Servers

Once you have the `policy.wasm` file, you can configure both the
`stormchaser-api` and `stormchaser-engine` to load it directly into memory at
startup by setting the `OPA_WASM_PATH` environment variable.

For example, when running locally or via Docker Compose, you can map the file
and set the variable:

```yaml
services:
  orchestration-api:
    environment:
      # Tell the API to load the WASM module
      OPA_WASM_PATH: /etc/opa/policy.wasm
      # Optional: Override the default entrypoint
      # OPA_ENTRYPOINT: stormchaser/allow
    volumes:
      - ./policy.wasm:/etc/opa/policy.wasm:ro
```

When `OPA_WASM_PATH` is set, Stormchaser skips making HTTP requests to the external
OPA server (like `OPA_URL`) and evaluates the policies directly inside its own
process using Wasmtime.
