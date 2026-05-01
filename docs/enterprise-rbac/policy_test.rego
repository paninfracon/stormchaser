package stormchaser.enterprise_test

import data.stormchaser.enterprise.allow
import rego.v1

# Mock data representing what would be loaded from roles.json
mock_data := {
	"role_mappings": {
		"admin": ["Okta-Global-Admins"],
		"developer": ["Okta-Engineering"],
		"operator": ["Okta-DevOps", "Okta-SRE"],
		"production_approvers": ["Okta-SRE-Leads"],
	},
	"allowed_email_domains": ["paninfracon.net"],
}

# Helper to mock a JWT token without having to sign a real one
mock_token(email, groups) := io.jwt.encode_sign(
	{"alg": "HS256", "typ": "JWT"},
	{"email": email, "groups": groups},
	{"k": "secret"},
)

# --- Tests ---

test_admin_can_access_anything if {
	allow with input as {
		"method": "DELETE",
		"path": "/api/v1/critical-system",
		"token": mock_token("admin@paninfracon.net", ["Okta-Global-Admins"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_developer_can_start_run if {
	allow with input as {
		"method": "POST",
		"path": "/api/v1/runs",
		"token": mock_token("dev@paninfracon.net", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_developer_cannot_delete_webhooks if {
	not allow with input as {
		"method": "DELETE",
		"path": "/api/v1/webhooks/123",
		"token": mock_token("dev@paninfracon.net", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_invalid_domain_is_rejected if {
	not allow with input as {
		"method": "POST",
		"path": "/api/v1/runs",
		"token": mock_token("hacker@evil.com", ["Okta-Global-Admins"]), # Has admin group, but wrong domain
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_sod_approval_denied if {
	not allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "staging"},
		"step_ast": {"name": "approve", "params": {}},
		"token": mock_token("dev@paninfracon.net", ["Okta-Global-Admins"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_sod_approval_allowed if {
	allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "staging"},
		"step_ast": {"name": "approve", "params": {}},
		"token": mock_token("admin@paninfracon.net", ["Okta-Global-Admins"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_production_approval_denied_if_not_lead if {
	not allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "production"},
		"step_ast": {"name": "approve", "params": {}},
		"token": mock_token("operator@paninfracon.net", ["Okta-DevOps"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_production_approval_allowed_if_lead if {
	allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "production"},
		"step_ast": {"name": "approve", "params": {}},
		"token": mock_token("lead@paninfracon.net", ["Okta-SRE-Leads"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}
