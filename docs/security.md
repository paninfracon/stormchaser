# Stormchaser Security Documentation

This document describes the security architecture, authentication mechanisms, and authorization checks currently implemented in the Stormchaser Orchestration Engine.

## Overview

Stormchaser employs a multi-layered security model designed to ensure that only authenticated users can trigger workflows and that every workflow execution conforms to centralized organizational policies.

## 1. Authentication

### API Bearer Tokens

Access to the Stormchaser API is secured using **JSON Web Tokens (JWT)**. All protected endpoints (such as enqueuing a workflow) require a valid `Authorization: Bearer <token>` header.

* **Token Validation**: The API server validates the signature, expiration (`exp`), and subject (`sub`) of every incoming token.
* **Security Posture**: Requests missing a token or providing an invalid/expired token are rejected with a `401 Unauthorized` status.

### SSO Token Exchange

To facilitate integration with external Identity Providers (IdPs), Stormchaser provides a token exchange endpoint:

* **Endpoint**: `POST /api/v1/auth/exchange`
* **Mechanism**: A user provides a short-lived SSO token (OIDC `id_token`) from a trusted provider. Stormchaser validates this token's signature cryptographically against the Identity Provider's JSON Web Key Set (JWKS) and issues a signed Stormchaser JWT for subsequent API calls.
* **Dynamic JWKS Fetching**: Stormchaser actively fetches and caches public keys from the IdP. If an incoming token specifies a Key ID (`kid`) that is not in the local cache, Stormchaser will proactively reach out to the IdP to fetch the latest keys. This ensures zero downtime during regular key rotations by the IdP (such as Dex).
* **User Identity**: The user's identity (ID or Email) is extracted from the SSO claims and embedded in the `sub` field of the resulting JWT.

## 2. Authorization (Open Policy Agent)

Stormchaser uses **Open Policy Agent (OPA)** to enforce fine-grained, decoupled authorization policies. Authorization occurs at two distinct stages of the workflow lifecycle.

### Tier 1: API Level (Request Authorization)

The first check occurs at the API gateway before a request is processed.

* **Context Sent to OPA**:
  * `path`: The requested URL path (e.g., `/api/v1/runs`).
  * `method`: The HTTP method (e.g., `POST`).
  * `token`: The raw bearer token (if provided).
* **Purpose**: To determine if the user is allowed to access specific API endpoints or perform certain actions (e.g., "Can this user trigger *any* workflow?").

### Tier 2: Engine Level (Execution Authorization)

The second check occurs in the Orchestration Engine immediately after the workflow DSL (`.storm` file) has been fetched and parsed, but **before** any execution begins.

* **Context Sent to OPA**:
  * `run_id`: Unique identifier for the workflow run.
  * `initiating_user`: The ID of the user who triggered the run.
  * `workflow_ast`: The full, JSON-serialized AST of the workflow definition.
  * `inputs`: The actual parameter values provided for this run.
* **Purpose**: This allows for extremely granular policies based on the workflow's structure and intent. For example:
  * "Users in the 'Junior' group cannot trigger workflows containing 'Production' in the name."
  * "Workflows that use 'privileged' containers require 'Admin' group membership."
  * "Workflows cannot be triggered with a `timeout` parameter greater than 4 hours unless approved."

## 3. Transport Security (mTLS)

Stormchaser supports **mutual TLS (mTLS)** for all server-to-server communications. This ensures that only authorized components (Engine, API, Runners, NATS, Postgres) can communicate with each other, and that all data in transit is encrypted.

### Internal Component mTLS

The Orchestration Engine and API support mounting certificates (e.g., from `cert-manager` in Kubernetes) and dynamically reloading them without a restart.

* **Certificate Watching**: The `stormchaser-tls` crate uses the `notify` crate to watch for file modifications in the certificate directory. When certificates are rotated (e.g., by Vault or Let's Encrypt), the system automatically swaps the `rustls` configuration in memory.
* **mTLS for NATS**: All NATS connections are configured to use client certificates for authentication and encryption.
* **mTLS for Postgres**: Database connections use `sqlx` with `rustls` to verify the server certificate and provide a client certificate for mutual authentication.
* **mTLS for OPA**: Outbound requests to the OPA server can be configured to use mTLS certificates.

### External Endpoint mTLS

For connections to 3rd-party services (e.g., S3-compatible storage, external Webhooks), Stormchaser allows storing certificates directly in the PostgreSQL database.

* **Database-Backed Certs**: The `storage_backends` and `webhooks` tables include columns for `ca_cert`, `client_cert`, and `client_key`.
* **Dynamic Configuration**: When connecting to an external service, the Engine retrieves these certificates from the database and constructs a transient `rustls::ClientConfig` for that specific connection.

## 4. Security Posture: Fail-Closed

Stormchaser follows a strict **Fail-Closed** security philosophy regarding OPA:

1. **Optional Configuration**: If `OPA_URL` is not configured in the environment, the checks are treated as a **no-op** (all requests allowed). This facilitates local development and testing.
2. **Hard Failure**: If `OPA_URL` **is** configured, the system requires a successful "allow" response from OPA to proceed.
3. **Strict Reliability**: If the OPA server is unreachable, returns a non-200 status code, or experiences an internal error, Stormchaser treats this as a **Deny**.
    * The API will return `500 Internal Server Error`.
    * The Orchestration Engine will transition the workflow run to the `Failed` state and halt execution.

## Summary of Environment Variables

| Variable | Description | Security Impact |
| :--- | :--- | :--- |
| `DATABASE_URL` | Connection string for Postgres. | Core data security. |
| `NATS_URL` | Connection string for NATS. | Message integrity. |
| `OPA_URL` | URL of the remote OPA server. | **Enables Authorization Enforcement.** |
| `JWT_SECRET` | Secret key for signing/verifying JWTs. | **Must be kept highly secure.** |
| `TLS_CA_CERT_PATH` | Path to the CA certificate file. | Trust root for mTLS. |
| `TLS_CERT_PATH` | Path to the component's client certificate. | Component identity for mTLS. |
| `TLS_KEY_PATH` | Path to the component's private key. | Component identity for mTLS. |
| `TLS_SERVER_NAME` | Expected TLS server name (optional). | Prevents MITM attacks. |

## 5. Encryption at rest

There are two main areas to consider for encryption at rest:

1. Postgres DB - standard techniques are well documented for this
2. NATS Jetstream - NATS documentation notes that whilst NATS supports at rest encryption, host native file system encryption is preferred

## 6. MCP Server Security

The Model Context Protocol (MCP) server, which exposes the OpenAPI specification as tools for AI agents, adheres to the same core security constraints as the rest of the system:

* **Authentication Requirement**: All MCP initial connections (`/api/v1/mcp/sse`) and subsequent JSON-RPC interactions (`/api/v1/mcp/messages`) require a standard API Bearer token.
* **Tool Call Proxying**: The MCP server dynamically invokes local API endpoints as requested by the AI agent. Because these generated internal HTTP calls do not inherently proxy the user's Bearer token, they arrive unauthenticated. *(Note: This is currently the recommended "conformant" way to use `rmcp-openapi`, but they are working on standardizing authorization passthrough in the near future).*
* **OPA Protection**: The default `deploy/opa/policy.rego` includes explicit rules to handle MCP security:
  * Unauthenticated calls initiated by the MCP tool logic are strictly blocked **unless** they are safe, read-only methods (`GET`, `HEAD`, `OPTIONS`) or target designated authentication/initialization endpoints.
  * Direct calls to the MCP endpoint itself that attempt non-read-only operations are blocked, except for `POST` requests to `/api/v1/mcp/messages` (which is necessary for the MCP JSON-RPC protocol to function).
* **Fail-Closed Integration**: If an AI agent attempts to invoke an MCP tool that performs a state-modifying action (e.g., `POST`, `DELETE`) without explicit authorization handling, the internal API call will be denied by OPA with a `403 Forbidden` response.
