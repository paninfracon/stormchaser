package stormchaser.enterprise

import rego.v1

default allow := false

# Decode the JWT token passed by Stormchaser
token_payload := payload if {
    [_, payload, _] := io.jwt.decode(input.token)
}

# --- Core Authorization Logic ---

allow if {
    is_valid_domain
    has_permission
}

# --- Data-Driven RBAC ---

# Check if the user's groups in the JWT overlap with the allowed IDP groups for a specific Stormchaser role
user_has_role(role_name) if {
    # Get the list of IDP groups mapped to this role from roles.json (loaded via data.role_mappings)
    allowed_idp_groups := data.role_mappings[role_name]

    # Check if any group in the user's token exists in the allowed IDP groups
    some user_group in token_payload.groups
    user_group in allowed_idp_groups
}

# --- Permissions Mapping ---

# Admins can do anything
has_permission if {
    user_has_role("admin")
}

# Developers can view and start runs
has_permission if {
    user_has_role("developer")
    input.method in ["GET", "POST"]
    startswith(input.path, "/api/v1/runs")
}

# Operators can manage webhooks and cron workflows
has_permission if {
    user_has_role("operator")
    input.method in ["GET", "POST", "DELETE"]
    startswith(input.path, "/api/v1/webhooks")
}
has_permission if {
    user_has_role("operator")
    input.method in ["GET", "POST", "DELETE"]
    startswith(input.path, "/api/v1/cron-workflows")
}

# Security can only view reports
has_permission if {
    user_has_role("security")
    input.method == "GET"
    startswith(input.path, "/api/v1/reports")
}

# --- General Security Policies ---

# Only allow users from approved corporate domains
is_valid_domain if {
    some domain in data.allowed_email_domains
    endswith(token_payload.email, concat("", ["@", domain]))
}
