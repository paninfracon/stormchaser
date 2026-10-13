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
	{"kty": "oct", "k": "c2VjcmV0"},
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

# --- Resource (per-object) tests ---

resource_request(action, type, owner, email, groups) := {
	"action": action,
	"resource": {"type": type, "id": "1", "owner": owner},
	"subject": {"sub": "some-id", "email": email},
	"token": mock_token(email, groups),
}

test_owner_can_delete_own_run if {
	allow with input as resource_request("delete", "run", "dev@paninfracon.net", "dev@paninfracon.net", ["Okta-Engineering"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_developer_cannot_read_another_users_run if {
	not allow with input as resource_request("read", "run", "other@paninfracon.net", "dev@paninfracon.net", ["Okta-Engineering"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_operator_can_read_but_not_delete_another_users_run if {
	allow with input as resource_request("read", "run", "dev@paninfracon.net", "ops@paninfracon.net", ["Okta-SRE"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	not allow with input as resource_request("delete", "run", "dev@paninfracon.net", "ops@paninfracon.net", ["Okta-SRE"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_admin_can_change_any_object if {
	allow with input as resource_request("update", "connection", "dev@paninfracon.net", "admin@paninfracon.net", ["Okta-Global-Admins"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_connections_are_usable_by_any_run_but_not_changeable if {
	# The engine asks on a run's behalf, with no token
	allow with input as {
		"action": "use",
		"resource": {"type": "connection", "id": "prod-db", "owner": "admin@paninfracon.net"},
		"subject": {"sub": "dev@paninfracon.net", "email": "dev@paninfracon.net"},
		"token": null,
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	not allow with input as resource_request("delete", "connection", "admin@paninfracon.net", "dev@paninfracon.net", ["Okta-Engineering"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}
