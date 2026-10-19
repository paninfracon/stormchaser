# A persona-based example policy. It answers all four questions Stormchaser asks
# (request, resource, engine, approval; see docs/opa-rbac.md). Build it with:
#
#   opa build -t wasm -e stormchaser/example/allow docs/opa-rbac-example.rego
#
# and set OPA_ENTRYPOINT=stormchaser/example/allow on the services that load it.
package stormchaser.example

import rego.v1

default allow := false

# Who is asking: the identity the API verified from their token, {sub, email, groups}.
# A request without a valid token has none. The token itself is never sent.
default caller := {"groups": []}

caller := object.union({"groups": []}, input.subject) if is_object(input.subject)

# The API does not ask policy about health checks, login, token exchange and refresh,
# webhook deliveries, approval links or cron triggers, so they need no rules here.
# Requests to the MCP transport arrive without a subject; this policy refuses them.

# Anyone with a token may ask who it identifies (the web UI checks its session this way)
allow if {
	input.method == "GET"
	input.path == "/api/v1/auth/session"
	is_object(input.subject)
}

# ---------------------------------------------------------------------------
# RBAC RULES
# ---------------------------------------------------------------------------

# 1. Admin Persona
# Has access to all endpoints
allow if {
	"admin" in caller.groups
}

# 2. Developer Persona
# Can view runs, start workflows, and approve or reject steps.
allow if {
	"developer" in caller.groups
	input.method in ["GET", "POST"]
	startswith(input.path, "/api/v1/runs")
}

# Developers can view webhooks but not modify them
allow if {
	"developer" in caller.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/webhooks")
}

# 3. Operator Persona
# Can view runs but not start them manually.
allow if {
	"operator" in caller.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/runs")
}

# Operators can manage cron workflows
allow if {
	"operator" in caller.groups
	startswith(input.path, "/api/v1/cron-workflows")
}

# Operators can manage webhooks
allow if {
	"operator" in caller.groups
	startswith(input.path, "/api/v1/webhooks")
}

# Operators can manage connections (storage, registries, Git, databases, APIs)
allow if {
	"operator" in caller.groups
	startswith(input.path, "/api/v1/connections")
}

# 4. Security Persona
# Read-only access to runs, and so their test reports (/api/v1/runs/{id}/reports),
# for auditing
allow if {
	"security" in caller.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/runs")
}

# Can manage event rules (e.g. security policies)
allow if {
	"security" in caller.groups
	startswith(input.path, "/api/v1/rules")
}

# ---------------------------------------------------------------------------
# PER-OBJECT (RESOURCE) QUESTIONS
# ---------------------------------------------------------------------------
# Asked when a request names a run, connection, webhook, event rule or cron
# schedule, and by the engine and query service before using a connection:
# {action, resource: {type, id, owner}, subject}. Without these rules every
# per-object request would be refused.

subject_ids contains id if {
	some id in [caller.sub, caller.email]
	is_string(id)
	id != ""
}

# Admins may do anything to any object
allow if {
	input.action
	"admin" in caller.groups
}

# Owners may do anything to what they created, except make a connection the
# default storage backend, which receives every other user's run data
allow if {
	input.action
	input.resource.owner in subject_ids
	input.action != "make_default"
}

# Operators and security may read every run
allow if {
	input.action == "read"
	input.resource.type == "run"
	some group in ["operator", "security"]
	group in caller.groups
}

# Connections are shared: everyone may see them and every run may use them
allow if {
	input.resource.type == "connection"
	input.action in {"read", "use"}
}

# ---------------------------------------------------------------------------
# ENGINE AND APPROVAL QUESTIONS
# ---------------------------------------------------------------------------

# Engine, before a run starts: {run_id, initiating_user, workflow_ast, inputs}.
# Any run may start. Restrict here by workflow content or inputs.
allow if {
	input.workflow_ast
	not input.method
}

# Approval: {run_id, initiating_user, step_ast, inputs, run_outputs, approver}.
# Whoever started a run may not approve its steps.
allow if {
	input.step_ast
	is_object(input.approver)
	input.approver.email != input.initiating_user
}
