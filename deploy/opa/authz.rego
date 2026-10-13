package system.authz

import rego.v1

# Who may use OPA's own HTTP API, enforced when OPA runs with --authorization=basic.
#
# Stormchaser's services only evaluate the policy. Without this, anything that can reach
# OPA's port could replace the policy or its data through the same API and so grant
# itself everything.

default allow := false

# Policy decisions
allow if {
	input.method == "POST"
	input.path[0] == "v1"
	input.path[1] == "data"
	input.path[2] == "stormchaser"
}

# Health checks
allow if {
	input.method == "GET"
	input.path == ["health"]
}
