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
        "sec@paninfracon.net": "EntraID-SecOps"
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
