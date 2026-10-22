package stormchaser_test

import data.stormchaser.allow
import rego.v1

alice := {"sub": "alice-id", "email": "alice@example.com"}

bob := {"sub": "bob-id", "email": "bob@example.com"}

with_admins := {"admins": ["root@example.com"], "admin_groups": ["platform-admins"]}

resource(action, type, owner, subject) := {
	"action": action,
	"resource": {"type": type, "id": "1", "owner": owner},
	"subject": subject,
}

# --- Unknown questions ---------------------------------------------------------------

test_unknown_input_is_refused if {
	not allow with input as {}
	not allow with input as {"connection_name": "db", "initiating_user": "alice@example.com"}
}

# --- Request tier --------------------------------------------------------------------

test_authenticated_request_is_allowed if {
	allow with input as {"method": "DELETE", "path": "/api/v1/runs/1", "subject": alice}
}

test_tokenless_request_may_only_read if {
	allow with input as {"method": "GET", "path": "/api/v1/runs", "subject": null}
	not allow with input as {"method": "POST", "path": "/api/v1/runs", "subject": null}
	not allow with input as {"method": "DELETE", "path": "/api/v1/connections/1"}
}

test_login_and_exchange_need_no_token if {
	allow with input as {"method": "POST", "path": "/api/v1/auth/exchange", "subject": null}
}

test_mcp_transport_is_allowed_and_other_mcp_writes_are_not if {
	allow with input as {"method": "POST", "path": "/api/v1/mcp", "subject": null}
	allow with input as {"method": "DELETE", "path": "/api/v1/mcp/", "subject": null}
	not allow with input as {"method": "POST", "path": "/api/v1/mcp/other", "subject": null}
	not allow with input as {"method": "PUT", "path": "/api/v1/mcp", "subject": alice}
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

test_only_admins_may_choose_the_default_storage_backend if {
	not allow with input as resource("make_default", "connection", "alice@example.com", alice)
		with data.stormchaser_settings as with_admins
	allow with input as resource("make_default", "connection", "alice@example.com", {"sub": "r", "email": "root@example.com"})
		with data.stormchaser_settings as with_admins
	allow with input as resource("update", "connection", "alice@example.com", alice)
		with data.stormchaser_settings as with_admins
}

test_anyone_may_choose_the_default_storage_backend_until_admins_exist if {
	allow with input as resource("make_default", "connection", "alice@example.com", alice)
	not allow with input as resource("make_default", "connection", "alice@example.com", bob)
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

test_admin_by_group if {
	admin := object.union(bob, {"groups": ["platform-admins"]})
	allow with input as resource("delete", "run", "alice@example.com", admin)
		with data.stormchaser_settings as with_admins

	not_admin := object.union(bob, {"groups": ["developers"]})
	not allow with input as resource("delete", "run", "alice@example.com", not_admin)
		with data.stormchaser_settings as with_admins

	garbage := object.union(bob, {"groups": "platform-admins"})
	not allow with input as resource("delete", "run", "alice@example.com", garbage)
		with data.stormchaser_settings as with_admins
}

test_a_token_in_the_input_grants_nothing if {
	# Policy reads groups only from the identity the API verified
	forged := io.jwt.encode_sign(
		{"alg": "HS256", "typ": "JWT"},
		{"sub": "x", "groups": ["platform-admins"]},
		{"kty": "oct", "k": "c2VjcmV0"},
	)
	request := object.union(resource("delete", "run", "alice@example.com", bob), {"token": forged})
	not allow with input as request with data.stormchaser_settings as with_admins
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

approval_listing(approvers, approver) := object.union(
	approval(approver),
	{"step_ast": {"name": "approve", "spec": {"approvers": approvers}}},
)

test_only_a_listed_approver_may_decide_a_step_that_names_them if {
	allow with input as approval_listing(["bob@example.com"], bob)
	allow with input as approval_listing(["bob-id"], bob)
	not allow with input as approval_listing(["bob@example.com"], alice)

	# An empty list names no one, so anyone may decide
	allow with input as approval_listing([], alice)
}

test_a_listed_group_or_an_admin_may_approve if {
	allow with input as approval_listing(["release-managers"], object.union(alice, {"groups": ["release-managers"]}))

	root := {"sub": "root-id", "email": "root@example.com"}
	allow with input as approval_listing(["bob@example.com"], root)
		with data.stormchaser_settings as with_admins
}

test_a_link_sent_to_several_approvers_is_not_refused_for_naming_no_one if {
	allow with input as approval_listing(["a@example.com", "b@example.com"], null)
}

# --- Authors and run-only users -------------------------------------------------------

with_authors := {
	"admins": ["root@example.com"],
	"admin_groups": ["platform-admins"],
	"author_groups": ["workflow-authors"],
}

run_only_user := {"sub": "carol-id", "email": "carol@example.com", "groups": ["engineers"]}

author := {"sub": "dave-id", "email": "dave@example.com", "groups": ["workflow-authors"]}

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
		allow with input as object.union(request, {"subject": run_only_user})
			with data.stormchaser_settings as with_admins
	}
}

test_run_only_users_may_not_supply_workflows_or_change_connections if {
	every request in author_only_requests {
		not allow with input as object.union(request, {"subject": run_only_user})
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
		allow with input as object.union(request, {"subject": run_only_user})
			with data.stormchaser_settings as with_authors
	}
}

test_authors_and_admins_may_write_workflows if {
	admin_by_group := {"sub": "erin-id", "groups": ["platform-admins"]}
	admin_by_email := {"sub": "root-id", "email": "root@example.com"}
	every subject in [author, admin_by_group, admin_by_email] {
		every request in author_only_requests {
			allow with input as object.union(request, {"subject": subject})
				with data.stormchaser_settings as with_authors
		}
	}
}
