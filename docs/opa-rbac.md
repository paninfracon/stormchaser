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

## Example Persona RBAC

An example policy file `docs/opa-rbac-example.rego` has been provided which
demonstrates how to decode the JWT token inside OPA and apply Role-Based Access
Control (RBAC) to the different endpoints in the API.

This example assumes you have an Identity Provider (like the bundled Dex) that
injects a `groups` claim into your OIDC tokens, such as:

- `admin`: Has full access to the platform.
- `developer`: Can view and execute workflows, as well as approve manual steps,
  but cannot modify platform configuration.
- `operator`: Cannot execute workflows directly, but manages underlying
  infrastructure like Cron Workflows, Webhooks, and Storage.
- `security`: Has read-only access to workflow runs, test reports, and manages
  platform event rules.

## Using the Example

To test this policy locally using Docker Compose, you can mount it into the OPA
container or replace the default policy:

```bash
cp docs/opa-rbac-example.rego deploy/opa/policy.rego
```

Then restart the OPA container:

```bash
docker compose restart opa
```

## ABAC (Attribute-Based Access Control)

Stormchaser's Orchestration Engine also performs a secondary OPA check right
before executing a workflow step. At this point, the context is richer, allowing
for Attribute-Based Access Control. This `EngineOpaContext` is injected as:

```json
{
  "run_id": "e4b2d1c... ",
  "initiating_user": "developer@example.com",
  "workflow_ast": { ... },
  "inputs": { ... }
}
```

You can use this richer context to write rules like: *"Only allow
production-deploy workflows if the initiating_user belongs to the SRE group."*
