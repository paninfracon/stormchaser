# Model Context Protocol (MCP) Server

Stormchaser includes an integrated [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server that allows AI agents and LLMs to natively interact with the Stormchaser API.

It exposes the API's OpenAPI specification as MCP tools. Because tool calls reach the API without the caller's token (see [Security & OPA Policies](#security--opa-policies)), in practice only tools for routes that need no token, such as `GET /api/v1/schema`, succeed. Every route that needs a bearer token answers `401`. The server is usable for exploring the API's shape, not yet for operating Stormchaser.

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

The server is behind the `mcp` Cargo feature. To build without it, use `--no-default-features` (and add back any other default features you need). If `API_BASE_URL` does not parse as a URL, the API starts without the MCP route.

## Configuration

When enabled, the server is mounted at `/api/v1/mcp`. It is a single Streamable HTTP endpoint; the same handler answers every path below the mount point, so there are no separate `sse` or `messages` routes.

- `POST` carries JSON-RPC requests (starting with `initialize`) and returns the response as an SSE stream. Clients must send `Accept: application/json, text/event-stream`.
- `GET` opens a server-to-client SSE stream for an existing session and requires the `Mcp-Session-Id` header returned by `initialize`.
- `DELETE` ends a session.

Because MCP tools often need to "call back" to the very API that hosts them, the server must know its own externally accessible address. This is configured via the `API_BASE_URL` environment variable.

- **`API_BASE_URL`**: The base URL used by generated tools to construct callback requests. Defaults to `http://localhost:3000`.

### Allowed hosts

To guard against DNS rebinding, the transport only accepts requests whose `Host` header is on an allow-list. Any other `Host` is rejected with `403 Forbidden: Host header is not allowed`. The list is made up of:

- the loopback names `localhost`, `127.0.0.1` and `::1`, which are always accepted;
- the host of `API_BASE_URL`;
- any hosts listed in **`MCP_ALLOWED_HOSTS`**, a comma-separated list such as `stormchaser.example.com,api.example.com:8443`.

An entry without a port matches that host on any port; an entry with a port matches only that port. When the API is reached through a hostname other than the one in `API_BASE_URL` (for example an ingress), add that hostname to `MCP_ALLOWED_HOSTS`. In the Helm chart this is the `api.mcpAllowedHosts` value.

## Security & OPA Policies

The MCP server is fully integrated with Stormchaser's Open Policy Agent (OPA) integration.

1. **Transport Endpoint**: The default policy permits the requests the transport itself needs on `/api/v1/mcp` without a token: `POST` (JSON-RPC messages), `DELETE` (ending a session) and read-only requests such as `GET`. Every other non-read-only request under `/api/v1/mcp` is denied. `POST` and `DELETE` are also permitted at `/api/v1/mcp/messages`, which older client configurations used; it is served by the same handler.
2. **Tool Execution**: When an AI executes a tool, the server makes the matching REST call to `API_BASE_URL`, and that call is evaluated by OPA like any other. The server does not forward the client's `Authorization` header to these calls (the `rmcp-openapi` crate only reads it under its actix-web transport, and Stormchaser uses Axum), so every tool call arrives unauthenticated.
3. **Default Policy**: The default Rego policy (`deploy/opa/policy.rego`) blocks unauthenticated requests that are not read-only (e.g., `POST`, `DELETE`). Separately, every route that needs a bearer token rejects a request without one (`401`) before policy is consulted. So tool calls that list runs, start runs or approve steps all fail; only routes that need no token work.

## Connecting an AI Client

Configure the AI client to use the Streamable HTTP transport with this URL:

- **MCP Endpoint**: `http://<stormchaser-api-host>/api/v1/mcp`

The host the client uses must be on the server's allow-list (see [Allowed hosts](#allowed-hosts)). Clients that only support the older HTTP+SSE transport, with separate SSE and message endpoints, are not supported.

A client may send an `Authorization: Bearer <token>` header, but it is not passed on to tool calls (see [Security & OPA Policies](#security--opa-policies)). Forwarding the caller's token is not implemented.
