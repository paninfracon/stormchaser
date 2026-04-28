package stormchaser.enterprise_test

import rego.v1
import data.stormchaser.enterprise.allow

# Mock data representing what would be loaded from roles.json
mock_data := {
    "role_mappings": {
        "admin": ["Okta-Global-Admins"],
        "developer": ["Okta-Engineering"]
    },
    "allowed_email_domains": ["paninfracon.net"]
}

# Helper to mock a JWT token without having to sign a real one
mock_token(email, groups) := io.jwt.encode_sign(
    {"alg": "HS256", "typ": "JWT"},
    {"email": email, "groups": groups},
    {"k": "secret"}
)

# --- Tests ---

test_admin_can_access_anything if {
    allow with input as {
        "method": "DELETE",
        "path": "/api/v1/critical-system",
        "token": mock_token("admin@paninfracon.net", ["Okta-Global-Admins"])
    } with data as mock_data
}

test_developer_can_start_run if {
    allow with input as {
        "method": "POST",
        "path": "/api/v1/runs",
        "token": mock_token("dev@paninfracon.net", ["Okta-Engineering"])
    } with data as mock_data
}

test_developer_cannot_delete_webhooks if {
    not allow with input as {
        "method": "DELETE",
        "path": "/api/v1/webhooks/123",
        "token": mock_token("dev@paninfracon.net", ["Okta-Engineering"])
    } with data as mock_data
}

test_invalid_domain_is_rejected if {
    not allow with input as {
        "method": "POST",
        "path": "/api/v1/runs",
        "token": mock_token("hacker@evil.com", ["Okta-Global-Admins"]) # Has admin group, but wrong domain
    } with data as mock_data
}
