# Model Context Protocol (MCP) Server

Stormchaser includes an integrated [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server that allows AI agents and LLMs to natively interact with the Stormchaser API.

By exposing its OpenAPI specification as MCP tools, Stormchaser empowers AI assistants to autonomously inspect workflows, trigger runs, approve manual steps, and monitor execution states directly from their context window.

## Overview

The MCP server is embedded directly into the `stormchaser-api` crate. It utilizes the official Rust SDK (`rmcp`) and dynamically translates the OpenAPI specification into callable tools via the `rmcp-openapi` crate.

### Key Features

- **Native Integration**: The MCP server runs within the same Axum router as the core REST API.
- **Zero Configuration Mapping**: Tools are automatically generated from the OpenAPI `utoipa` schema. No manual tool definitions are required.
- **Streamable HTTP Transport**: Communicates using the MCP Streamable HTTP transport: clients `POST` JSON-RPC messages to a single endpoint and receive responses as a Server-Sent Events (SSE) stream.
- **Secure by Default**: All MCP interactions are routed through the same Open Policy Agent (OPA) middleware as standard API traffic.

## Enabling the MCP Server

The MCP server is enabled by default in `stormchaser-api`.

To build and run the API with default features (including MCP):

```bash
cargo build -p stormchaser-api
```

If you need to disable MCP, run with `--no-default-features`.

## Configuration

When enabled, the server is mounted at `/api/v1/mcp`. It is a single Streamable HTTP endpoint; the same handler answers every path below the mount point, so there are no separate `sse` or `messages` routes.

- `POST` carries JSON-RPC requests (starting with `initialize`) and returns the response as an SSE stream. Clients must send `Accept: application/json, text/event-stream`.
- `GET` opens a server-to-client SSE stream for an existing session and requires the `Mcp-Session-Id` header returned by `initialize`.
- `DELETE` ends a session.

Because MCP tools often need to "call back" to the very API that hosts them, the server must know its own externally accessible address. This is configured via the `API_BASE_URL` environment variable.

- **`API_BASE_URL`**: The base URL used by generated tools to construct callback requests. Defaults to `http://localhost:3000`.

### Host header restriction

To guard against DNS rebinding, the transport only accepts requests whose `Host` header is a loopback name: `localhost`, `127.0.0.1` or `::1`. Any other `Host` is rejected with `403 Forbidden: Host header is not allowed`. This list is not currently configurable, so a client that reaches the API through another hostname must do so via a local port-forward or a proxy that rewrites the `Host` header.

## Security & OPA Policies

The MCP server is fully integrated with Stormchaser's Open Policy Agent (OPA) integration.

1. **Transport Endpoint**: The default policy denies every non-read-only request under `/api/v1/mcp` except `POST /api/v1/mcp/messages`, which is permitted without a token. Because the transport needs `POST`, clients running against the default policy must use `/api/v1/mcp/messages` as the endpoint URL; a `POST` to `/api/v1/mcp` itself is denied, as is the `DELETE` that ends a session. Read-only requests (`GET`) are permitted on any path under the mount.
2. **Tool Execution**: When an AI executes a tool (which triggers a REST API call), that secondary API call is evaluated by OPA. If the AI client does not pass the user's token during the tool execution, the request will be unauthenticated.
3. **Default Policy**: The default Rego policy explicitly blocks unauthenticated requests that are not read-only (e.g., `POST`, `DELETE`), preventing an AI client from modifying state without proper authorization proxying.

## Connecting an AI Client

Configure the AI client to use the Streamable HTTP transport with one of these URLs:

- **With the default OPA policy**: `http://localhost:3000/api/v1/mcp/messages`
- **With OPA disabled, or a policy that permits `POST /api/v1/mcp`**: `http://localhost:3000/api/v1/mcp`

Replace the port as needed, but keep a loopback host name (see [Host header restriction](#host-header-restriction)). Clients that only support the older HTTP+SSE transport, with separate SSE and message endpoints, are not supported.

Clients should provide a valid Authorization header (`Bearer <token>`) when available, especially for tool calls that require authenticated write access.
