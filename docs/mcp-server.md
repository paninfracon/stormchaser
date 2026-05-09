# Model Context Protocol (MCP) Server

Stormchaser includes an integrated [Model Context Protocol (MCP)](https://modelcontextprotocol.io/) server that allows AI agents and LLMs to natively interact with the Stormchaser API.

By exposing its OpenAPI specification as MCP tools, Stormchaser empowers AI assistants to autonomously inspect workflows, trigger runs, approve manual steps, and monitor execution states directly from their context window.

## Overview

The MCP server is embedded directly into the `stormchaser-api` crate. It utilizes the official Rust SDK (`rmcp`) and dynamically translates the OpenAPI specification into callable tools via the `rmcp-openapi` crate.

### Key Features

- **Native Integration**: The MCP server runs within the same Axum router as the core REST API.
- **Zero Configuration Mapping**: Tools are automatically generated from the OpenAPI `utoipa` schema. No manual tool definitions are required.
- **SSE Transport**: Communicates using the standard Server-Sent Events (SSE) transport protocol over HTTP.
- **Secure by Default**: All MCP interactions are routed through the same Open Policy Agent (OPA) middleware as standard API traffic.

## Enabling the MCP Server

The MCP server is enabled by default in `stormchaser-api`.

To build and run the API with default features (including MCP):

```bash
cargo build -p stormchaser-api
```

If you need to disable MCP, run with `--no-default-features`.

## Configuration

When enabled, the server mounts its endpoints under `/api/v1/mcp`.

Because MCP tools often need to "call back" to the very API that hosts them, the server must know its own externally accessible address. This is configured via the `API_BASE_URL` environment variable.

- **`API_BASE_URL`**: The base URL used by generated tools to construct callback requests. Defaults to `http://localhost:3000`.

## Security & OPA Policies

The MCP server is fully integrated with Stormchaser's Open Policy Agent (OPA) integration.

1. **Transport Endpoints**: Under the default policy, `GET /api/v1/mcp/sse` and `POST /api/v1/mcp/messages` are permitted without a token so MCP clients can establish SSE and exchange JSON-RPC messages.
2. **Tool Execution**: When an AI executes a tool (which triggers a REST API call), that secondary API call is evaluated by OPA. If the AI client does not pass the user's token during the tool execution, the request will be unauthenticated.
3. **Default Policy**: The default Rego policy explicitly blocks unauthenticated requests that are not read-only (e.g., `POST`, `DELETE`), preventing an AI client from modifying state without proper authorization proxying.

## Connecting an AI Client

An AI client supporting MCP via SSE can connect to the server using the following URL structure:

- **SSE Endpoint**: `http://<stormchaser-api-host>/api/v1/mcp/sse`
- **Messages Endpoint**: `http://<stormchaser-api-host>/api/v1/mcp/messages` (Discovered automatically via the SSE initialization event)

Clients should provide a valid Authorization header (`Bearer <token>`) when available, especially for tool calls that require authenticated write access.
