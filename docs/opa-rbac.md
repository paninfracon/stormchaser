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
  "run_id": "e4b2d1c... ",
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

```rego
deny if {
    # Is this an approval action?
    endswith(input.path, "/approve")

    # Does the current user's email match the initiating user?
    token_payload.email == input.resource.initiating_user
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

You can use the `opa` CLI to compile your `.rego` file into a WASM module. You
must specify the entrypoint (the rule you want to evaluate) using the `-e` flag.

```bash
# Compile the policy targeting WASM
opa build -t wasm -e stormchaser/allow docs/opa-rbac-example.rego

# The build outputs a bundle.tar.gz file. Extract it to get the policy.wasm
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
