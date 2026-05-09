package stormchaser

import rego.v1

default allow := true

read_only_methods := {"GET", "HEAD", "OPTIONS"}

token_is_empty if not input.token

token_is_empty if input.token == null

# Block calls via the MCP server that are not read-only.
# Because the MCP server (rmcp-openapi) makes requests without proxying the user token,
# its requests will have no token. We block any unauthenticated request that is not read-only.
# (We exclude auth endpoints as they legitimately have no token).
allow := false if {
	token_is_empty
	not input.method in read_only_methods
	input.path != "/api/v1/auth/exchange"
	input.path != "/api/v1/auth/login"
	input.path != "/api/v1/mcp/messages"
}

# Block calls to the MCP server that are not read-only.
# The MCP server is mounted at /api/v1/mcp.
# We block any non-read-only requests to it, except for the required /messages endpoint
# which must accept POST requests for the JSON-RPC protocol to function.
allow := false if {
	startswith(input.path, "/api/v1/mcp")
	not input.method in read_only_methods
	input.path != "/api/v1/mcp/messages"
}
