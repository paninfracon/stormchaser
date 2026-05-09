# Cargo Features

This document outlines the various Cargo features available across the `stormchaser` workspace crates and their purposes.

By default, most functional features are enabled to provide a complete "batteries-included" experience, but they can be opted out of to reduce binary size, dependencies, and compilation times.

## `stormchaser-api`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `mcp` | ✅ | Enables the integrated [Model Context Protocol (MCP) server](mcp-server.md), exposing the OpenAPI spec as AI-callable tools via SSE. Pulls in `rmcp` and `rmcp-openapi`. |

## `stormchaser-engine`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `aws-lambda` | ✅ | Enables the ability to dispatch workflow steps directly to AWS Lambda functions. |
| `aws-ses` | ✅ | Enables the "Send Email" step using AWS Simple Email Service (SES). |
| `aws-s3-sts` | ✅ | Enables AWS STS integration for assuming roles when interacting with S3 storage backends. |
| `email` | ✅ | Enables general SMTP email capabilities (e.g., sending approval requests) using the `lettre` crate. |
| `vault` | ✅ | Enables fetching dynamic secrets from HashiCorp Vault during workflow execution. |
| `aws-sdk-sts` | ✅ | Base STS support for AWS authentication. |
| `rustls-native-certs` | ✅ | Enables loading native root certificates for TLS connections, particularly useful outside of containerized environments. |

## `stormchaser-tls`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `sqlx` | ✅ | Provides helper functions for building `rustls::ClientConfig` dynamically from certificates stored in a PostgreSQL database via `sqlx`. |

## Disabling Default Features

To build a minimal version of a crate or the workspace, you can use the `--no-default-features` flag and explicitly enable only what you need:

```bash
# Build the API without the MCP server
cargo build -p stormchaser-api --no-default-features

# Build the engine with only core execution capabilities
cargo build -p stormchaser-engine --no-default-features
```
