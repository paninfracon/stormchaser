package stormchaser

import rego.v1

# Stormchaser asks this policy four kinds of question, told apart by the shape of the
# input:
#
#   request   {method, path, token}                      may this API request proceed?
#   resource  {action, resource, subject, token}         may this user act on this object?
#   engine    {run_id, initiating_user, workflow_ast, inputs}   may this run start?
#   approval  {run_id, initiating_user, step_ast, inputs, run_outputs, approver, token}
#
# Anything else is refused.
#
# Deployment settings come from data.stormchaser_settings (see settings.json):
#
#   admins                 emails or subject ids that may act on any object
#   admin_groups           values of the token's `groups` claim that make a user an admin
#   separation_of_duties   when true, a run's initiator may not approve its steps
#   author_groups          values of the token's `groups` claim whose members may write
#                          workflows. Empty (the default) means everyone may. When set,
#                          everyone else may only run workflows held in repositories
#                          that an author or admin has registered as a connection.

default allow := false

admins contains admin if some admin in data.stormchaser_settings.admins

admin_groups contains group if some group in data.stormchaser_settings.admin_groups

separation_of_duties if data.stormchaser_settings.separation_of_duties == true

admins_configured if count(admins) + count(admin_groups) > 0

author_groups contains group if some group in data.stormchaser_settings.author_groups

authors_configured if count(author_groups) > 0

# --- Request tier -------------------------------------------------------------------

allow if {
	is_request
	not request_denied
}

is_request if {
	input.method
	input.path
}

read_only_methods := {"GET", "HEAD", "OPTIONS"}

token_is_empty if not input.token

token_is_empty if input.token == null

# Requests without a token come from the MCP server, which does not pass the user's
# token on. They may only read. (The login and token exchange endpoints legitimately
# have no token.)
request_denied if {
	token_is_empty
	not input.method in read_only_methods
	not startswith(input.path, "/api/v1/mcp")
	input.path != "/api/v1/auth/exchange"
	input.path != "/api/v1/auth/login"
}

# The MCP server is a single Streamable HTTP endpoint mounted at /api/v1/mcp.
# /api/v1/mcp/messages is served by the same handler and is kept for clients that
# were configured with it before the mount point itself was permitted.
mcp_transport_paths := {"/api/v1/mcp", "/api/v1/mcp/", "/api/v1/mcp/messages"}

# The transport needs POST for JSON-RPC messages and DELETE to end a session.
mcp_transport_methods := {"POST", "DELETE"}

mcp_transport_request if {
	input.path in mcp_transport_paths
	input.method in mcp_transport_methods
}

# Calls to the MCP server may only read, except for the transport's own requests,
# which the JSON-RPC protocol needs in order to function.
request_denied if {
	startswith(input.path, "/api/v1/mcp")
	not input.method in read_only_methods
	not mcp_transport_request
}

# --- Authors and run-only users -----------------------------------------------------
#
# With author_groups set, a user outside them (and not an admin) may run workflows but
# not write them. Three things would let such a user choose what runs, or what a run
# form's queries are, and are refused:
#
#   - starting a run from workflow text they supply (a direct run);
#   - having a run form hydrated from workflow text they supply;
#   - creating or changing a connection, which is how a repository of their own, or a
#     database or API of their choosing, would become something workflows can use.
#
# What is left to them is workflows in the repositories authors have registered.

token_claims := payload if {
	is_string(input.token)
	[_, payload, _] := io.jwt.decode(input.token)
} else := {}

request_is_from_author if {
	some group in object.get(token_claims, "groups", [])
	group in author_groups
}

request_is_from_author if {
	some group in object.get(token_claims, "groups", [])
	group in admin_groups
}

request_is_from_author if {
	some claim in ["email", "sub"]
	object.get(token_claims, claim, "") in admins
}

author_only_paths := {"/api/v1/runs/direct", "/api/v1/schema/hydrate-direct"}

author_only_request if input.path in author_only_paths

author_only_request if input.path == "/api/v1/connections"

author_only_request if startswith(input.path, "/api/v1/connections/")

request_denied if {
	authors_configured
	author_only_request
	not input.method in read_only_methods
	not request_is_from_author
}

# --- Who is asking ------------------------------------------------------------------

# The API verifies the token before asking, and sends the verified subject and email.
subject_ids contains id if {
	id := input.subject.sub
	is_string(id)
	id != ""
}

subject_ids contains id if {
	id := input.subject.email
	is_string(id)
	id != ""
}

token_groups := groups if {
	is_string(input.token)
	[_, payload, _] := io.jwt.decode(input.token)
	groups := object.get(payload, "groups", [])
} else := []

is_admin if {
	some id in subject_ids
	id in admins
}

is_admin if {
	some group in token_groups
	group in admin_groups
}

# --- Resource tier ------------------------------------------------------------------
#
# resource.type is one of run, connection, webhook, event_rule, cron_workflow.
# resource.owner is whoever created the object (for a run, whoever started it), or
# null for objects that predate ownership and runs started by the system.
# action is one of read, update, delete, use.

allow if {
	is_resource
	resource_allowed
}

is_resource if {
	input.action
	input.resource
}

has_owner if {
	owner := input.resource.owner
	is_string(owner)
	owner != ""
	not is_system_label(owner)
}

# What runs started by the system, a schedule or a webhook record in place of a user
# when there is no user to act as.
is_system_label(owner) if owner == "system"

is_system_label(owner) if startswith(owner, "system:")

is_system_label(owner) if startswith(owner, "webhook:")

owned_by_subject if {
	has_owner
	input.resource.owner in subject_ids
}

# Admins may do anything.
resource_allowed if is_admin

# Owners may do anything with what they own.
resource_allowed if owned_by_subject

# Objects with no owner can be read by everyone.
resource_allowed if {
	not has_owner
	input.action == "read"
}

# Objects with no owner can be changed only by admins, once any are configured. Until
# then everyone may, as before ownership existed, so that an upgrade locks nothing.
resource_allowed if {
	not has_owner
	not admins_configured
}

# Connections are shared: everyone may see them (their credentials are masked) and
# every run may use them. Only the owner or an admin may change or delete one.
resource_allowed if {
	input.resource.type == "connection"
	input.action in {"read", "use"}
}

# --- Engine tier --------------------------------------------------------------------

# Any authenticated user may start any workflow. Add rules here to restrict by
# input.initiating_user, input.workflow_ast or input.inputs.
allow if {
	input.workflow_ast
	not is_request
	not is_resource
}

# --- Approval tier ------------------------------------------------------------------

allow if {
	input.step_ast
	not is_request
	not is_resource
	not approval_denied
}

approver_ids contains id if {
	id := input.approver.sub
	is_string(id)
	id != ""
}

approver_ids contains id if {
	id := input.approver.email
	is_string(id)
	id != ""
}

# With separation of duties on, whoever started a run may not approve its steps.
approval_denied if {
	separation_of_duties
	input.initiating_user in approver_ids
}

# A step that names its approvers (users, by email or subject, or groups) may be
# decided only by one of them, or by an admin. An approval made through an emailed
# link with no single approver is not checked here: the link is signed, and was sent
# only to the step's approvers.
step_approvers := approvers if {
	approvers := object.get(object.get(input.step_ast, "spec", {}), "approvers", [])
	is_array(approvers)
} else := []

approver_listed if {
	some id in approver_ids
	id in step_approvers
}

approver_listed if {
	some group in token_groups
	group in step_approvers
}

approver_is_admin if {
	some id in approver_ids
	id in admins
}

approver_is_admin if {
	some group in token_groups
	group in admin_groups
}

approval_denied if {
	count(step_approvers) > 0
	count(approver_ids) > 0
	not approver_listed
	not approver_is_admin
}
