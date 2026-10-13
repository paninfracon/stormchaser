package stormchaser_test

import data.stormchaser.allow
import rego.v1

alice := {"sub": "alice-id", "email": "alice@example.com"}

bob := {"sub": "bob-id", "email": "bob@example.com"}

with_admins := {"admins": ["root@example.com"], "admin_groups": ["platform-admins"]}

token_with_groups(groups) := io.jwt.encode_sign(
	{"alg": "HS256", "typ": "JWT"},
	{"sub": "x", "groups": groups},
	{"kty": "oct", "k": "c2VjcmV0"},
)

resource(action, type, owner, subject) := {
	"action": action,
	"resource": {"type": type, "id": "1", "owner": owner},
	"subject": subject,
	"token": null,
}

# --- Unknown questions ---------------------------------------------------------------

test_unknown_input_is_refused if {
	not allow with input as {}
	not allow with input as {"connection_name": "db", "initiating_user": "alice@example.com"}
}

# --- Request tier --------------------------------------------------------------------

test_authenticated_request_is_allowed if {
	allow with input as {"method": "DELETE", "path": "/api/v1/runs/1", "token": "t"}
}

test_tokenless_request_may_only_read if {
	allow with input as {"method": "GET", "path": "/api/v1/runs", "token": null}
	not allow with input as {"method": "POST", "path": "/api/v1/runs", "token": null}
	not allow with input as {"method": "DELETE", "path": "/api/v1/connections/1"}
}

test_login_and_exchange_need_no_token if {
	allow with input as {"method": "POST", "path": "/api/v1/auth/exchange", "token": null}
}

test_mcp_transport_is_allowed_and_other_mcp_writes_are_not if {
	allow with input as {"method": "POST", "path": "/api/v1/mcp", "token": null}
	allow with input as {"method": "DELETE", "path": "/api/v1/mcp/", "token": null}
	not allow with input as {"method": "POST", "path": "/api/v1/mcp/other", "token": null}
	not allow with input as {"method": "PUT", "path": "/api/v1/mcp", "token": "t"}
}

# --- Resource tier -------------------------------------------------------------------

test_owner_may_do_anything_with_their_own if {
	every action in ["read", "update", "delete"] {
		allow with input as resource(action, "run", "alice@example.com", alice)
	}

	# Ownership recorded as a subject id also counts
	allow with input as resource("delete", "webhook", "alice-id", alice)
}

test_others_may_not_touch_an_owned_object if {
	every type in ["run", "webhook", "event_rule", "cron_workflow"] {
		every action in ["read", "update", "delete"] {
			not allow with input as resource(action, type, "alice@example.com", bob)
		}
	}
}

test_connections_are_shared_for_reading_and_use_only if {
	allow with input as resource("read", "connection", "alice@example.com", bob)
	allow with input as resource("use", "connection", "alice@example.com", bob)
	not allow with input as resource("update", "connection", "alice@example.com", bob)
	not allow with input as resource("delete", "connection", "alice@example.com", bob)
}

test_unowned_objects_are_readable_by_everyone if {
	allow with input as resource("read", "run", null, bob)
	allow with input as resource("read", "run", "system", bob)
	allow with input as resource("read", "run", "system:cron", bob)
	allow with input as resource("read", "run", "webhook:github", bob)
	allow with input as resource("read", "webhook", "", bob)
}

test_unowned_objects_are_changeable_until_admins_exist if {
	allow with input as resource("delete", "webhook", null, bob)
	not allow with input as resource("delete", "webhook", null, bob)
		with data.stormchaser_settings as with_admins
	not allow with input as resource("delete", "run", "system", bob)
		with data.stormchaser_settings as with_admins
}

test_admins_may_do_anything if {
	root := {"sub": "root-id", "email": "root@example.com"}
	allow with input as resource("delete", "run", "alice@example.com", root)
		with data.stormchaser_settings as with_admins
	allow with input as resource("update", "connection", null, root)
		with data.stormchaser_settings as with_admins
}

test_admin_by_token_group if {
	request := object.union(
		resource("delete", "run", "alice@example.com", bob),
		{"token": token_with_groups(["platform-admins"])},
	)
	allow with input as request with data.stormchaser_settings as with_admins

	not_admin := object.union(
		resource("delete", "run", "alice@example.com", bob),
		{"token": token_with_groups(["developers"])},
	)
	not allow with input as not_admin with data.stormchaser_settings as with_admins

	garbage := object.union(resource("delete", "run", "alice@example.com", bob), {"token": "garbage"})
	not allow with input as garbage with data.stormchaser_settings as with_admins
}

test_an_owner_named_system_is_nobody if {
	system := {"sub": "system", "email": "system"}
	not allow with input as resource("delete", "run", "system", system)
		with data.stormchaser_settings as with_admins
}

test_a_subject_with_no_identity_owns_nothing if {
	nobody := {"sub": "", "email": null}
	not allow with input as resource("read", "run", "alice@example.com", nobody)
}

# --- Engine tier ---------------------------------------------------------------------

test_runs_may_start if {
	allow with input as {
		"run_id": "1",
		"initiating_user": "alice@example.com",
		"workflow_ast": {"name": "w"},
		"inputs": {},
	}
}

# --- Approval tier -------------------------------------------------------------------

approval(approver) := {
	"run_id": "1",
	"initiating_user": "alice@example.com",
	"step_ast": {"name": "approve"},
	"inputs": {},
	"run_outputs": {},
	"approver": approver,
	"token": "t",
}

test_anyone_may_approve_by_default if {
	allow with input as approval(alice)
	allow with input as approval(bob)
}

test_separation_of_duties_stops_the_initiator_approving if {
	sod := {"separation_of_duties": true}
	not allow with input as approval(alice) with data.stormchaser_settings as sod
	allow with input as approval(bob) with data.stormchaser_settings as sod
}

# --- Authors and run-only users -------------------------------------------------------

with_authors := {
	"admins": ["root@example.com"],
	"admin_groups": ["platform-admins"],
	"author_groups": ["workflow-authors"],
}

token_for(claims) := io.jwt.encode_sign({"alg": "HS256", "typ": "JWT"}, claims, {"kty": "oct", "k": "c2VjcmV0"})

runner_token := token_for({"sub": "carol-id", "email": "carol@example.com", "groups": ["engineers"]})

author_token := token_for({"sub": "dave-id", "email": "dave@example.com", "groups": ["workflow-authors"]})

author_only_requests := [
	{"method": "POST", "path": "/api/v1/runs/direct"},
	{"method": "POST", "path": "/api/v1/schema/hydrate-direct"},
	{"method": "POST", "path": "/api/v1/connections"},
	{"method": "POST", "path": "/api/v1/connections/test"},
	{"method": "PUT", "path": "/api/v1/connections/5f0c1e0e-0000-0000-0000-000000000000"},
	{"method": "DELETE", "path": "/api/v1/connections/5f0c1e0e-0000-0000-0000-000000000000"},
]

test_without_author_groups_everyone_may_write_workflows if {
	every request in author_only_requests {
		allow with input as object.union(request, {"token": runner_token})
			with data.stormchaser_settings as with_admins
	}
}

test_run_only_users_may_not_supply_workflows_or_change_connections if {
	every request in author_only_requests {
		not allow with input as object.union(request, {"token": runner_token})
			with data.stormchaser_settings as with_authors
	}
}

test_run_only_users_may_still_run_and_hydrate_repository_workflows if {
	every request in [
		{"method": "POST", "path": "/api/v1/runs"},
		{"method": "POST", "path": "/api/v1/schema/hydrate"},
		{"method": "POST", "path": "/api/v1/schema/parse-git"},
		{"method": "GET", "path": "/api/v1/connections"},
		{"method": "GET", "path": "/api/v1/runs/direct"},
	] {
		allow with input as object.union(request, {"token": runner_token})
			with data.stormchaser_settings as with_authors
	}
}

test_authors_and_admins_may_write_workflows if {
	admin_by_group := token_for({"sub": "erin-id", "groups": ["platform-admins"]})
	admin_by_email := token_for({"sub": "root-id", "email": "root@example.com"})
	every token in [author_token, admin_by_group, admin_by_email] {
		every request in author_only_requests {
			allow with input as object.union(request, {"token": token})
				with data.stormchaser_settings as with_authors
		}
	}
}
