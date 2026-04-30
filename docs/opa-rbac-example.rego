package stormchaser.example

import rego.v1

default allow := false

default token_payload := {}

# Helper to decode and verify JWT token.
# In production, io.jwt.verify_rs256 or similar should be used if the API
# hasn't already verified the signature. Since the API passes the raw token
# in `input.token`, we decode it here to read claims.
token_payload := payload if {
	token := object.get(input, "token", null)
	token != null
	regex.match(`^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$`, token)
	[_, payload, _] := io.jwt.decode(token)
}

# ---------------------------------------------------------------------------
# PUBLIC ROUTES
# ---------------------------------------------------------------------------

# Allow public health checks
allow if {
	input.method == "GET"
	input.path == "/healthz"
}

allow if {
	input.method == "GET"
	input.path == "/api/health"
}

# Allow login and token exchange without a token
allow if {
	input.method == "GET"
	input.path == "/api/v1/auth/login"
}

allow if {
	input.method == "POST"
	input.path == "/api/v1/auth/exchange"
}

# ---------------------------------------------------------------------------
# RBAC RULES
# ---------------------------------------------------------------------------

# 1. Admin Persona
# Has access to all endpoints
allow if {
	"admin" in token_payload.groups
}

# 2. Developer Persona
# Can view runs, start workflows, and approve steps.
allow if {
	"developer" in token_payload.groups
	input.method in ["GET", "POST"]
	startswith(input.path, "/api/v1/runs")
}

# Developers can view webhooks but not modify them
allow if {
	"developer" in token_payload.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/webhooks")
}

# 3. Operator Persona
# Can view runs but not start them manually.
allow if {
	"operator" in token_payload.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/runs")
}

# Operators can manage cron workflows
allow if {
	"operator" in token_payload.groups
	startswith(input.path, "/api/v1/cron-workflows")
}

# Operators can manage webhooks
allow if {
	"operator" in token_payload.groups
	startswith(input.path, "/api/v1/webhooks")
}

# Operators can manage storage backends
allow if {
	"operator" in token_payload.groups
	startswith(input.path, "/api/v1/storage")
}

# 4. Security Persona
# Read-only access to runs for auditing
allow if {
	"security" in token_payload.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/runs")
}

# Can manage event rules (e.g. security policies)
allow if {
	"security" in token_payload.groups
	startswith(input.path, "/api/v1/rules")
}

# Can view test reports/vulnerabilities
allow if {
	"security" in token_payload.groups
	input.method == "GET"
	startswith(input.path, "/api/v1/reports")
}

# ---------------------------------------------------------------------------
# ATTRIBUTE-BASED ACCESS CONTROL (ABAC) EXAMPLES
# ---------------------------------------------------------------------------
# Example: A user can only access a specific run if they were the initiating_user.
# Note: This requires the OPA context to include the `run` resource details,
# which is currently handled by the `EngineOpaContext` during execution evaluation,
# or would require an external data fetch to the DB from within OPA.
#
# allow if {
#     "developer" in token_payload.groups
#     input.method == "GET"
#     startswith(input.path, "/api/v1/runs/")
#     token_payload.email == input.resource.initiating_user
# }
