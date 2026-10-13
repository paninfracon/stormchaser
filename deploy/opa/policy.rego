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
	not startswith(input.path, "/api/v1/mcp")
	input.path != "/api/v1/auth/exchange"
	input.path != "/api/v1/auth/login"
}

# The MCP server is a single Streamable HTTP endpoint mounted at /api/v1/mcp.
# /api/v1/mcp/messages is served by the same handler and is kept for clients that
# were configured with it before the mount point itself was permitted.
mcp_transport_paths := {"/api/v1/mcp", "/api/v1/mcp/", "/api/v1/mcp/messages"}

# The transport needs POST for JSON-RPC messages and DELETE to end a session.
mcp_transport_methods := {"POST", "DELETE"}

mcp_transport_request if {
	input.path in mcp_transport_paths
	input.method in mcp_transport_methods
}

# Block calls to the MCP server that are not read-only, except for the transport's
# own requests, which the JSON-RPC protocol needs in order to function.
allow := false if {
	startswith(input.path, "/api/v1/mcp")
	not input.method in read_only_methods
	not mcp_transport_request
}
