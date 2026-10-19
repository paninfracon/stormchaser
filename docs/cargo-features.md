# Cargo Features

This document lists the Cargo features of the `stormchaser` workspace crates and what each one does.

Only three crates have features: `stormchaser-api`, `stormchaser-engine` and `stormchaser-tls`, plus the build-mode features of `stormchaser-web`. The defaults turn on every integration. Turning one off removes its dependencies and makes steps that need it fail with an error saying the feature is not enabled in this build. The other crates have no features.

## `stormchaser-api`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `mcp` | yes | Builds the [Model Context Protocol (MCP) server](mcp-server.md) into the API at `/api/v1/mcp`, exposing the OpenAPI operations as tools over Streamable HTTP. Pulls in `rmcp` and `rmcp-openapi`. |

## `stormchaser-engine`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `aws-lambda` | yes | `LambdaInvoke` steps. Turns on `aws-sdk-sts`. |
| `aws-ses` | yes | The SES backend for `EmailSend` and `TestReportEmail` steps. Turns on `aws-sdk-sts`. |
| `aws-s3-sts` | yes | Assuming an IAM role (`aws_assume_role_arn`) for S3 storage connections. Turns on `aws-sdk-sts`. |
| `email` | yes | SMTP email through `lettre`: approval notifications, and the SMTP backend for `EmailSend` and `TestReportEmail`. Without it, approval steps log a warning and send nothing. |
| `vault` | yes | Reading secrets from HashiCorp Vault (`secret()` in workflows). |
| `aws-sdk-sts` | yes | The STS client. On its own, it lets Terraform steps assume `aws_assume_role_arn`; the other AWS features turn it on. |
| `rustls-native-certs` | yes | Pulls in the `rustls-native-certs` crate. No engine code uses it at present; services already validate TLS against the system certificate store without it. |
| `aws-cloudcontrol` | yes | Workflow input queries backed by the AWS Cloud Control API. |
| `aws-sops` | yes | Assuming an IAM role (through STS) before decrypting SOPS secrets. Turns on `aws-sdk-sts`. Decrypting SOPS files itself needs only the `sops` binary on the engine's `PATH`. |
| `chatops-slack` | yes | `SlackMessage` steps. |
| `chatops-teams` | yes | `TeamsMessage` steps. |

CI builds and lints the engine both with all features and with `--no-default-features`.

## `stormchaser-tls`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `listener` | no | An HTTPS (or plain HTTP) listener for Axum services, used by the API, the query service and the web UI's server. Pulls in `axum`, `hyper`, `hyper-util`, `tokio-rustls` and `tower`. |

## `stormchaser-web`

| Feature | Default | Description |
| :--- | :---: | :--- |
| `ssr` | no | Builds the server-side rendering binary (Axum server). Selected automatically by `cargo leptos`. Without it, the `stormchaser-web` binary does nothing. |
| `hydrate` | no | Builds the WASM frontend that hydrates the server-rendered page. Selected automatically by `cargo leptos`. |

Build or run the web UI with `cargo leptos build -p stormchaser-web` or `cargo leptos watch -p stormchaser-web`, not plain `cargo run`.

## Disabling Default Features

To build a smaller binary, use `--no-default-features` and enable only what you need:

```bash
# Build the API without the MCP server
cargo build -p stormchaser-api --no-default-features

# Build the engine with no optional integrations
cargo build -p stormchaser-engine --no-default-features

# Build the engine with only Vault and SMTP email
cargo build -p stormchaser-engine --no-default-features --features vault,email
```

`stormchaser-api` and `stormchaser-query` depend on `stormchaser-engine` with its default features, so building them turns those features on in the engine library they link.
