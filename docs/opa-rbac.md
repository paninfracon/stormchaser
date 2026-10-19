# OPA RBAC Policies

Stormchaser integrates with Open Policy Agent (OPA) out of the box to enforce API
access control and workflow execution policies.

## Which policy is enforced

Every service that asks policy (the API, the engine and the query service) enforces
one, chosen at startup:

1. `OPA_WASM_PATH`: a policy compiled to WebAssembly (see section 5), evaluated
   in-process. `OPA_DATA_PATH` names a JSON file with its `data` document.
2. `OPA_URL`: an OPA server, which loads its own policy and data. Give the full
   decision URL, for example `http://127.0.0.1:8181/v1/data/stormchaser/allow`.
   The service posts `{"input": ...}` there and needs `{"result": true}` back;
   `false` is a refusal, and anything else (an error status, an undefined result)
   is an error, which refuses too.
3. Neither: the **built-in policy**, `deploy/opa/policy.rego` compiled into the
   services (crate `stormchaser-opa`). `OPA_DATA_PATH` gives it its settings, in the
   shape of `deploy/opa/settings.json` (`{"stormchaser_settings": {...}}`). Without
   settings, users can act only on what they own (and on objects that predate
   ownership) and nobody is an admin.

`OPA_ENTRYPOINT` (default `stormchaser/allow`) names the rule a WebAssembly or
built-in policy is evaluated at. It has no effect with `OPA_URL`, whose path already
names the rule. An empty `OPA_URL` counts as unset. The Helm chart always sets
`OPA_URL` to the OPA sidecar in each pod (`global.opa.url`).

There is no way to run a release build without a policy. A debug build started with
`STORMCHASER_AUTHZ_DISABLED=true` skips every check, for local development only; a
release build refuses to start with it.

## What policy is asked

For every protected request, the API asks:

```json
{
  "path": "/api/v1/runs",
  "method": "POST",
  "subject": { "sub": "auth0|123", "email": "alice@example.com", "groups": ["developers"] }
}
```

`subject` is the identity the API verified from the caller's token: its subject,
email and the identity provider's groups. It is `null` when the request has no
valid token. On the API's protected routes that cannot happen, since a request
without one is refused with `401` before policy is asked. It happens on the MCP
transport (`/api/v1/mcp`), which takes requests without a token, and on the query
service:

```json
{
  "path": "/api/v1/mcp",
  "method": "POST",
  "subject": null
}
```

The API does not ask policy about `/healthz`, `/api/health`, login, token exchange
and refresh, webhook deliveries (`POST /api/v1/webhooks/{id}`), approval links
(`/api/v1/approve-link/...`), cron triggers (`/api/v1/cron-trigger/{id}`) or
`GET /api/v1/schema`. Each of those authenticates its caller in its own way, or
needs no caller.

Policy is never sent the token itself, so a policy cannot read claims other than
these three, and an OPA server's decision logs hold no credentials.

The same entrypoint (`stormchaser/allow`) is also asked three other kinds of
question, and a policy has to answer all four. They are told apart by the fields
present in `input`:

| Question | Fields | Asked by |
| :--- | :--- | :--- |
| Request | `method`, `path`, `subject` | API and query service, for every protected request |
| Resource | `action`, `resource`, `subject` | API, when a request names an object; engine and query service, before using a connection |
| Engine | `run_id`, `initiating_user`, `workflow_ast`, `inputs` | Engine, before a run starts |
| Approval | `run_id`, `initiating_user`, `step_ast`, `inputs`, `run_outputs`, `approver` | API, when a step is approved or rejected |

A resource question looks like this:

```json
{
  "action": "delete",
  "resource": { "type": "webhook", "id": "6f1c...", "owner": "alice@example.com" },
  "subject": { "sub": "auth0|123", "email": "bob@example.com", "groups": ["developers"] }
}
```

`action` is `read`, `update`, `delete`, `use` or `make_default`. `make_default` is
asked when a request makes a connection the default storage backend
(`is_default_sfs`), which receives the files, artifacts and test reports of every
run that names no backend of its own; grant it to admins only. The shipped policy
does, once any admin is configured. `resource.type` is `run`,
`connection`, `webhook`, `event_rule` or `cron_workflow`. `resource.owner` is
`null` for objects created before ownership was recorded. `subject` is the
identity the API verified; when the engine asks on a run's behalf, `sub` and
`email` both hold the run's user and `groups` is empty. An approval's `approver`
has the same shape, and is `null` for a decision made through an emailed link
sent to several approvers. `step_ast` is the step as written in the workflow
(`name`, `type`, `spec`, ...); the shipped policy reads the approvers a step names
from `step_ast.spec.approvers`.

The policy Stormchaser ships, `deploy/opa/policy.rego`, answers all four and is the
best starting point for your own. Its tests are in `deploy/opa/policy_test.rego`
(`opa test deploy/opa`). A policy written before the resource question existed
that denies by default will refuse every per-object request until it gains a rule
for it.

> **Tip:** The fastest way to learn and debug Rego policies is the
> [Rego Playground](https://play.openpolicyagent.org/). You can paste your policy
> there, supply a mock `input` JSON, and instantly see what evaluates to true
> or false.

## 1. Enterprise Data-Driven RBAC

Hardcoding groups into your Rego policy (e.g., `"admin" in input.subject.groups`)
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
    some user_group in input.subject.groups
    user_group in allowed_idp_groups
}
```

When your organization's Okta groups change, you only update the JSON data,
not the Rego code.

## 2. Policy Unit Testing (`opa test`)

The hardest part of writing Rego is figuring out why a rule evaluated to `false`.
OPA has a built-in testing framework that makes this easy.

See `docs/enterprise-rbac/policy_test.rego` for an example. You can write tests
that mock the `input` (like the API path and the caller) and the `data` (like
the `roles.json` mapping).

Run the tests using the OPA CLI:

```bash
opa test docs/enterprise-rbac/ -v
```

This guarantees your authorization logic works before deploying it to the cluster.

## 3. ABAC (Attribute-Based Access Control)

The engine asks policy once per run, after it has parsed the workflow and before
anything runs, including the workflow's queries. The context is richer than a
request's, allowing for Attribute-Based Access Control. This `EngineOpaContext`
is sent as:

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
Only allow users in the "ReleaseManagers" group to deploy to production. The engine
knows a run's user only by name, so the groups come from your own data, as in
`docs/enterprise-rbac/`:

```rego
allow if {
    # Check if the user is a Release Manager
    "ReleaseManagers" in data.user_groups[input.initiating_user]

    # Check if the workflow inputs have 'env' set to 'production'
    input.inputs.env == "production"
}
```

`workflow_ast` is the whole parsed workflow. Its steps are under `steps`, each with
`name`, `type` and `spec`; steps of a `Parallel` or `Sequential` step nest under
that step's own `steps`, and `on_failure` and `finally` hold more. A step that uses
a step library appears as the library's step, with the library's type and the
merged spec.

**Example 2: Prevent privileged containers.**
Deny any workflow that runs a container with `privileged = true` anywhere in it,
unless the user is an admin (again known only by name, so the role comes from data):

```rego
privileged_step if {
    walk(input.workflow_ast, [_, node])
    node.type == "RunContainer"
    node.spec.privileged == true
}

deny if {
    privileged_step
    not "admin" in data.user_groups[input.initiating_user]
}
```

A `deny` rule has effect only if your `allow` rule for the engine question says
`not deny`. The runners refuse privileged steps anyway unless started with
`STORMCHASER_ALLOW_PRIVILEGED=true`.

## 4. OPA Cookbook (Helpful Snippets)

Here are some common copy-pasteable snippets for enterprise environments.

**MFA Enforcement**
Policy sees only the subject, email and groups of a token, not claims such as `amr`.
Require MFA in the identity provider, for the Stormchaser client, instead.

**Email Domain Whitelisting**
Deny all access if the user's email does not end with your corporate domain.

```rego
allow if {
    endswith(input.subject.email, "@yourcompany.com")
}
```

**Run-only users**
The shipped policy has this built in: set `author_groups` in its settings (Helm:
`api.opa.sidecar.settings.authorGroups`) and users outside those groups can run
workflows from registered repositories but cannot supply workflow text or change
connections. In a policy of your own, the requests to refuse are the non-read
methods on these paths, which the API and the query service both ask about:

```rego
author_only_request if input.path in {"/api/v1/runs/direct", "/api/v1/schema/hydrate-direct"}
author_only_request if input.path == "/api/v1/connections"
author_only_request if startswith(input.path, "/api/v1/connections/")

request_denied if {
    author_only_request
    not input.method in {"GET", "HEAD", "OPTIONS"}
    not "workflow-authors" in input.subject.groups
}
```

**Approval Separation of Duties (SoD)**
Prevent the person who initiated a run from approving their own manual steps.

The approval question carries both the run's `initiating_user` and the `approver`.
The shipped policy does this when `separation_of_duties` is set in its settings.

```rego
approval_denied if {
    input.step_ast
    input.approver.email == input.initiating_user
}
```

## 5. Compiling Policies to WASM

To run policies locally without network hops, the API, the engine and the query
service can execute pre-compiled OPA WebAssembly (WASM) modules directly, with
Wasmtime. The built-in policy is run this way.

> **Note:** While WASM execution is incredibly fast, using OPA over HTTP
> (via `OPA_URL`) is generally more flexible. It is preferable for centrally
> managed RBAC solutions or in environments where policies may change
> frequently, as it allows policies to be updated without restarting the
> Stormchaser services.
>
> **Unsupported builtins:** The embedded WASM runtime does not provide the
> YAML builtins (`yaml.marshal`, `yaml.unmarshal`, `yaml.is_valid`). Policies
> that call them must be evaluated over HTTP via `OPA_URL` instead.

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

# --- Helper Rules ---

is_valid_domain if {
    # Custom logic here...
    endswith(input.subject.email, "@yourcompany.com")
}

has_permission if {
    # Evaluates to true if the user is an admin
    "admin" in input.subject.groups
}

has_permission if {
    # OR it evaluates to true if they are a developer reading runs
    "developer" in input.subject.groups
    input.method == "GET"
    startswith(input.path, "/api/v1/runs")
}
```

```bash
# Compile the entire policy targeting WASM.
# The example's package is stormchaser.example, so its rule is stormchaser/example/allow.
opa build -t wasm -e stormchaser/example/allow docs/opa-rbac-example.rego

# The build outputs a bundle.tar.gz file. Extract it to get the single policy.wasm
tar -xzf bundle.tar.gz /policy.wasm
```

### Configure Stormchaser Servers

Once you have the `policy.wasm` file, configure the API, the engine and the query
service to load it at startup by setting the `OPA_WASM_PATH` environment variable.
A service that cannot read or load the file does not start.

For example, when running locally or via Docker Compose, you can map the file
and set the variable:

```yaml
services:
  orchestration-api:
    environment:
      # Tell the API to load the WASM module
      OPA_WASM_PATH: /etc/opa/policy.wasm
      # The rule to evaluate; the default is stormchaser/allow. The example
      # policy's is:
      OPA_ENTRYPOINT: stormchaser/example/allow
    volumes:
      - ./policy.wasm:/etc/opa/policy.wasm:ro
```

When `OPA_WASM_PATH` is set, `OPA_URL` is ignored and the policy is evaluated
inside the service's own process.

A policy that reads `data`, such as the shipped one's settings, gets it from the
JSON file named by `OPA_DATA_PATH`, read at startup. It must hold a JSON object.
With `OPA_URL` the file is ignored (with a warning), since the OPA server loads its
own data:

```yaml
      OPA_DATA_PATH: /etc/opa/settings.json
```

The built-in policy is built the same way, by `scripts/build-opa-policy.sh`. Run it
after changing `deploy/opa/policy.rego`; a unit test fails until you do.
