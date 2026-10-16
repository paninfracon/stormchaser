package stormchaser.enterprise

import rego.v1

default allow := false

# Handle JWT decoding safely: guard against a null/absent token (e.g. EngineContext)
# before calling io.jwt.decode, which would raise a built-in type error on null input.
token_payload := payload if {
	input.token != null
	[_, payload, _] := io.jwt.decode(input.token)
} else := {"groups": [], "email": ""}

# --- Core Authorization Logic ---

allow if {
	# 1. Allow unauthenticated/public API routes explicitly.
	input.method
	input.path
	is_public_api_route
}

allow if {
	# 2. Check if it's an API request (has method/path)
	input.method
	input.path
	is_valid_domain
	has_api_permission
}

allow if {
	# 3. Check if it's an Engine request (has workflow_ast/inputs)
	input.workflow_ast
	input.inputs
	is_valid_engine_user
	has_engine_permission
}

allow if {
	# 4. Check if it's an Approval request (has step_ast, inputs, token, initiating_user)
	input.step_ast
	input.inputs
	input.token
	input.initiating_user
	is_valid_domain
	has_approval_permission
}

allow if {
	# 5. Check if it's a resource request: may this user act on this object?
	# (has action, resource {type, id, owner} and subject {sub, email})
	input.action
	input.resource
	has_resource_permission
}

# Public API endpoints that must remain accessible without a token.
is_public_api_route if {
	input.method == "GET"
	input.path == "/api/v1/auth/login"
}

is_public_api_route if {
	input.method == "POST"
	input.path == "/api/v1/auth/exchange"
}

is_public_api_route if {
	input.method == "GET"
	input.path == "/api/health"
}

is_public_api_route if {
	input.method == "GET"
	input.path == "/healthz"
}

is_public_api_route if {
	input.method == "GET"
	input.path == "/readyz"
}

is_public_api_route if {
	input.method == "GET"
	input.path == "/livez"
}

# --- Data-Driven RBAC ---

# Check if the user's groups in the JWT overlap with the allowed IDP groups for a specific Stormchaser role
user_has_role(role_name) if {
	allowed_idp_groups := data.role_mappings[role_name]
	some user_group in token_payload.groups
	user_group in allowed_idp_groups
}

# Check if an email belongs to a role (for Engine Context where we only have initiating_user email)
email_has_role(email, role_name) if {
	# In a real environment, this mapping would come from an external data source or token propagation
	# For this example, we mock a mapping
	mock_email_to_group := {
		"admin@paninfracon.net": "Okta-Global-Admins",
		"dev@paninfracon.net": "Okta-Engineering",
		"ops@paninfracon.net": "Okta-SRE",
		"sec@paninfracon.net": "EntraID-SecOps",
	}
	user_group := mock_email_to_group[email]
	allowed_idp_groups := data.role_mappings[role_name]
	user_group in allowed_idp_groups
}

# --- API Permissions Mapping ---

# Admins can do anything in the API
has_api_permission if {
	user_has_role("admin")
}

# Anyone may ask who their own token identifies (the web UI checks a session this way)
has_api_permission if {
	input.method == "GET"
	input.path == "/api/v1/auth/session"
}

# Developers can view and start runs
has_api_permission if {
	user_has_role("developer")
	input.method in ["GET", "POST"]
	startswith(input.path, "/api/v1/runs")
}

# Operators can view runs
has_api_permission if {
	user_has_role("operator")
	input.method == "GET"
	startswith(input.path, "/api/v1/runs")
}

# Operators can manage webhooks and cron workflows
has_api_permission if {
	user_has_role("operator")
	input.method in ["GET", "POST", "DELETE"]
	startswith(input.path, "/api/v1/webhooks")
}

has_api_permission if {
	user_has_role("operator")
	input.method in ["GET", "POST", "DELETE"]
	startswith(input.path, "/api/v1/cron-workflows")
}

# Security can view reports
has_api_permission if {
	user_has_role("security")
	input.method == "GET"
	startswith(input.path, "/api/v1/reports")
}

# Security can view runs
has_api_permission if {
	user_has_role("security")
	input.method == "GET"
	startswith(input.path, "/api/v1/runs")
}

# --- Resource (per-object) Permissions ---

# The API sends the identity it verified from the token; the engine sends the user a
# run acts for in both fields.
subject_ids contains id if {
	id := input.subject.sub
	is_string(id)
}

subject_ids contains id if {
	id := input.subject.email
	is_string(id)
}

resource_has_owner if {
	is_string(input.resource.owner)
	input.resource.owner != ""
}

# Admins may do anything to any object
has_resource_permission if {
	user_has_role("admin")
}

# Owners may do anything to what they created (for a run: started), except make a
# connection the default storage backend, which receives every other user's run data
has_resource_permission if {
	resource_has_owner
	input.resource.owner in subject_ids
	input.action != "make_default"
}

# Operators may read every run, whoever started it
has_resource_permission if {
	input.resource.type == "run"
	input.action == "read"
	user_has_role("operator")
}

# Objects created before ownership was recorded can be read by everyone
has_resource_permission if {
	not resource_has_owner
	input.action == "read"
}

# Connections are shared: everyone may see them and every run may use them
has_resource_permission if {
	input.resource.type == "connection"
	input.action in ["read", "use"]
}

# --- General Security Policies (API) ---

# Only allow users from approved corporate domains
is_valid_domain if {
	some domain in data.allowed_email_domains
	endswith(token_payload.email, concat("", ["@", domain]))
}

# --- Engine ABAC Policies ---

# Validate that the engine's initiating_user comes from an approved domain
is_valid_engine_user if {
	some domain in data.allowed_email_domains
	endswith(input.initiating_user, concat("", ["@", domain]))
}

# Engine rule: Allow execution unless explicitly denied
has_engine_permission if {
	not engine_deny
}

# ABAC Example 1: Prevent deployment to 'production' if user is not in 'operator' role
engine_deny if {
	input.inputs.env == "production"
	not email_has_role(input.initiating_user, "operator")
}

# ABAC Example 2: Prevent running containers with 'privileged' flag unless admin
engine_deny if {
	input.workflow_ast.step_type == "RunContainer"
	input.workflow_ast.config.privileged == true
	not email_has_role(input.initiating_user, "admin")
}

# --- Approval ABAC Policies ---

# Approval rule: Allow if not explicitly denied, AND if user is in approvers list
has_approval_permission if {
	not approval_deny
	user_is_in_approvers_list
}

user_is_in_approvers_list if {
	# If the step doesn't specify approvers, anyone can approve (subject to other policies)
	spec := object.get(input.step_ast, "params", {}) # Check params for intrinsic/template steps or spec for native
	approvers := object.get(spec, "approvers", [])
	count(approvers) == 0
}

user_is_in_approvers_list if {
	spec := object.get(input.step_ast, "params", {})
	approvers := object.get(spec, "approvers", [])
	count(approvers) > 0
	some approver in approvers

	# Map the requested approver (e.g. "admin") to IDP groups
	user_has_role(approver)
}

# Fallback to check "spec" object if "params" didn't match (for native vs templated steps)
user_is_in_approvers_list if {
	spec := object.get(input.step_ast, "spec", {})
	approvers := object.get(spec, "approvers", [])
	count(approvers) == 0
}

user_is_in_approvers_list if {
	spec := object.get(input.step_ast, "spec", {})
	approvers := object.get(spec, "approvers", [])
	count(approvers) > 0
	some approver in approvers
	user_has_role(approver)
}

# Approval ABAC Example 1: Separation of Duties (SoD)
approval_deny if {
	# Prevent the person who initiated the run from approving their own manual steps
	token_payload.email == input.initiating_user
}

# Approval ABAC Example 2: Environment-specific restrictions
approval_deny if {
	# Only allow "production_approvers" to approve if env == "production"
	input.inputs.env == "production"
	not user_has_role("production_approvers")
}
