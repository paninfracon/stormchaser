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

# A user as the API describes them to policy, from their verified token
user(email, groups) := {"sub": email, "email": email, "groups": groups}

# --- Tests ---

test_admin_can_access_anything if {
	allow with input as {
		"method": "DELETE",
		"path": "/api/v1/critical-system",
		"subject": user("admin@paninfracon.net", ["Okta-Global-Admins"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_developer_can_start_run if {
	allow with input as {
		"method": "POST",
		"path": "/api/v1/runs",
		"subject": user("dev@paninfracon.net", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_developer_cannot_delete_webhooks if {
	not allow with input as {
		"method": "DELETE",
		"path": "/api/v1/webhooks/123",
		"subject": user("dev@paninfracon.net", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_invalid_domain_is_rejected if {
	not allow with input as {
		"method": "POST",
		"path": "/api/v1/runs",
		"subject": user("hacker@evil.com", ["Okta-Global-Admins"]), # Has admin group, but wrong domain
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_sod_approval_denied if {
	not allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "staging"},
		"step_ast": {"name": "approve", "params": {}},
		"approver": user("dev@paninfracon.net", ["Okta-Global-Admins"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_sod_approval_allowed if {
	allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "staging"},
		"step_ast": {"name": "approve", "params": {}},
		"approver": user("admin@paninfracon.net", ["Okta-Global-Admins"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_production_approval_denied_if_not_lead if {
	not allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "production"},
		"step_ast": {"name": "approve", "params": {}},
		"approver": user("operator@paninfracon.net", ["Okta-DevOps"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_production_approval_allowed_if_lead if {
	allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "production"},
		"step_ast": {"name": "approve", "params": {}},
		"approver": user("lead@paninfracon.net", ["Okta-SRE-Leads"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

# --- Resource (per-object) tests ---

resource_request(action, type, owner, email, groups) := {
	"action": action,
	"resource": {"type": type, "id": "1", "owner": owner},
	"subject": {"sub": "some-id", "email": email, "groups": groups},
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
	# The engine asks on a run's behalf, knowing the user only by name
	allow with input as {
		"action": "use",
		"resource": {"type": "connection", "id": "prod-db", "owner": "admin@paninfracon.net"},
		"subject": {"sub": "dev@paninfracon.net", "email": "dev@paninfracon.net"},
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	not allow with input as resource_request("delete", "connection", "admin@paninfracon.net", "dev@paninfracon.net", ["Okta-Engineering"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_only_an_admin_may_choose_the_default_storage_backend if {
	not allow with input as resource_request("make_default", "connection", "dev@paninfracon.net", "dev@paninfracon.net", ["Okta-Engineering"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	allow with input as resource_request("make_default", "connection", "dev@paninfracon.net", "admin@paninfracon.net", ["Okta-Global-Admins"])
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_anyone_in_the_domain_may_check_their_session if {
	allow with input as {
		"method": "GET",
		"path": "/api/v1/auth/session",
		"subject": user("dev@paninfracon.net", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	not allow with input as {
		"method": "GET",
		"path": "/api/v1/auth/session",
		"subject": user("dev@example.com", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

# --- Approvers named by the step, and engine checks ---

test_only_a_named_approver_may_approve if {
	step := {"name": "approve", "type": "Approval", "spec": {"approvers": ["lead@paninfracon.net"]}}
	allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "staging"},
		"step_ast": step,
		"approver": user("lead@paninfracon.net", []),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	not allow with input as {
		"initiating_user": "dev@paninfracon.net",
		"inputs": {"env": "staging"},
		"step_ast": step,
		"approver": user("other@paninfracon.net", ["Okta-Engineering"]),
	}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}

test_a_nested_privileged_container_needs_an_admin if {
	workflow := {"name": "w", "steps": [{
		"name": "group",
		"type": "Parallel",
		"steps": [{"name": "build", "type": "RunContainer", "spec": {"image": "alpine", "privileged": true}}],
	}]}
	not allow with input as {"run_id": "1", "initiating_user": "dev@paninfracon.net", "workflow_ast": workflow, "inputs": {"env": "staging"}}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
	allow with input as {"run_id": "1", "initiating_user": "admin@paninfracon.net", "workflow_ast": workflow, "inputs": {"env": "staging"}}
		with data.role_mappings as mock_data.role_mappings
		with data.allowed_email_domains as mock_data.allowed_email_domains
}
