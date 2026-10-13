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

Stormchaser uses **Open Policy Agent (OPA)** to enforce fine-grained, decoupled authorization policies. Policy is asked four kinds of question, all at the same entrypoint and told apart by the shape of the input: about a request, about an object, about a run that is starting, and about an approval.

> **Without OPA there is no isolation between users.** If no policy engine is configured (`OPA_URL` unset), every one of these checks is skipped: any authenticated user can read, change and delete every run, connection, webhook, rule and schedule. That is intended for single-user development only.

The policy Stormchaser ships (`deploy/opa/policy.rego`, also packaged in the Helm chart) refuses anything it does not recognise, and by default:

* lets any authenticated user call any endpoint and start any workflow;
* lets users read and change only the runs, webhooks, event rules and cron schedules they created, and lets admins do anything;
* shares connections: everyone may see them (credentials are always masked) and every run may use them, but only the creator or an admin may change or delete one;
* leaves objects that predate ownership readable by everyone, and changeable by everyone until an admin is configured, by admins only after that.

Settings for it live in `deploy/opa/settings.json` (Helm: `api.opa.sidecar.settings`): `admins` (emails or subject ids), `admin_groups` (values of the token's `groups` claim) and `separation_of_duties` (when true, whoever started a run may not approve its steps).

OPA's own HTTP API is restricted (`deploy/opa/authz.rego`, with `--authorization=basic`) to evaluating the policy, so that nothing which can reach OPA's port can replace the policy or its data.

### Tier 1: API Level (Request Authorization)

The first check occurs at the API gateway before a request is processed.

* **Context Sent to OPA**:
  * `path`: The requested URL path, including the `/api/v1` prefix (e.g., `/api/v1/runs`).
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

### Per-object authorization (ownership)

For every request whose path names a run, connection, webhook, event rule or cron schedule by id, the API looks the object up and asks policy whether this caller may act on it. Lists and the runs stream are filtered the same way, so a list never shows what fetching the object would refuse.

* **Context Sent to OPA**:
  * `action`: `read`, `update`, `delete` or `use`.
  * `resource`: `{ "type": "run" | "connection" | "webhook" | "event_rule" | "cron_workflow", "id": "...", "owner": "alice@example.com" | null }`.
  * `subject`: `{ "sub": "...", "email": "..." }`, the identity the API verified from the token.
  * `token`: the raw bearer token, for policies that read further claims such as groups.
* **Owner**: whoever created the object: their email, or their subject id when the token carries no email. For a run it is whoever started it. A run started by a webhook rule or a cron schedule acts as whoever created that rule or schedule. Objects created before ownership was recorded have no owner (`null`).
* **Connection use**: before the engine or the query service uses a connection on a run's behalf, it asks with `action: "use"` and the run's user as the subject (and no token).
* **Not covered**: approving or rejecting a step is decided by the approval question below, since approvers are by nature not the owner. Completing a `Wait` step through `POST /api/v1/events/correlate` is governed by request-level policy only. The runs list is filtered after paging, so a page can hold fewer than `limit` entries.

### Approvals

When a user approves or rejects a step, policy is asked with `run_id`, `initiating_user`, `step_ast`, `inputs`, `run_outputs`, the `approver` (`{ "sub", "email" }`, as verified) and the raw `token`.

### Approval links

An approval email can carry links that approve or reject the step without logging in. A link is a credential for one decision on one step, so:

* Links are sealed with a key of their own, `STORMCHASER_APPROVAL_LINK_KEY` (at least 32 characters; Helm: `global.approvalLinks.existingSecret`), set to the same value on the engine and the API. There is no default: without it the engine sends emails with no links and the API refuses every link.
* A link expires after `STORMCHASER_APPROVAL_LINK_TTL_HOURS` (default 72).
* Opening a link (GET) only shows a confirmation page. The decision is recorded when that page's form is submitted (POST), so mail scanners and link previews cannot approve anything by fetching the URL.
* The decision is recorded against the addresses the email was sent to. When that is one address, it is the `approver` in the approval policy question, so separation of duties applies; an email sent to several people carries one shared link and no single approver.
* The link's token is left out of the API's request logs and traces.

Anyone who obtains a link before it expires can use it. Send approval emails to individuals, keep the lifetime short, and prefer the UI or CLI where the approver's identity matters.

## 3. Transport Security (mTLS)

Stormchaser supports **mutual TLS (mTLS)** for all server-to-server communications. This ensures that only authorized components (Engine, API, Runners, NATS, Postgres) can communicate with each other, and that all data in transit is encrypted.

### Internal Component mTLS

The Orchestration Engine and API support mounting certificates (e.g., from `cert-manager` in Kubernetes) and dynamically reloading them without a restart.

* **Certificate Watching**: The `stormchaser-tls` crate uses the `notify` crate to watch for file modifications in the certificate directory. When certificates are rotated (e.g., by Vault or Let's Encrypt), the system automatically swaps the `rustls` configuration in memory.
* **mTLS for NATS**: The API and Engine present their client certificate to NATS, and it is enforced only when the NATS server is configured with TLS and `verify: true`. The runners do not present a client certificate; they authenticate with NATS credentials. The bundled quickstart NATS has no TLS. See [NATS: Quickstart and Production](nats.md).
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
| `NATS_CREDS_FILE`, `NATS_USER` / `NATS_PASSWORD`, `NATS_TOKEN` | NATS credentials (see [nats.md](nats.md)). | **Who may schedule and report on steps.** |
| `NATS_CA_FILE`, `NATS_TLS_REQUIRED` | Verify the NATS server and refuse plaintext. | Protects credentials and events in transit. |
| `OPA_URL` | URL of the remote OPA server. | **Enables Authorization Enforcement.** |
| `STORMCHASER_APPROVAL_LINK_KEY` | Engine and API: key that seals approve and reject links in approval emails (32+ characters, same on both). Unset by default. | Whoever has it can forge approvals. Links are disabled without it. |
| `STORMCHASER_APPROVAL_LINK_TTL_HOURS` | Engine: lifetime of an approval link. Default `72`. | Bounds how long a leaked link is usable. |
| `STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS` | Engine: send an OCI artifact backend's stored credentials to runner helpers when its registry cannot issue scoped tokens. Default `false`. | Long-lived registry credentials leave the engine. |
| `STORMCHASER_NATS_STREAM_MAX_AGE_HOURS` | Engine and runners: maximum age of messages in the NATS stream. Default `168`. | Bounds how long any message stays readable. |
| `VAULT_TOKEN` | Engine: token for `secret()` lookups. No default; lookups are disabled without it. | Everything the token can read is within reach of the paths below. |
| `STORMCHASER_VAULT_ALLOWED_PATHS` | Engine: secret paths workflows may read with `secret()` (comma-separated, trailing `*` for a prefix). Empty by default. | Limits which secrets a workflow author can read. |
| `STORMCHASER_TRUSTED_PROXIES` | API: addresses or CIDR ranges of the reverse proxies in front of it. Empty by default. | `X-Forwarded-For` is believed only from these; anything else could pick its own rate-limit bucket. |
| `API_RATE_LIMIT_PER_SECOND`, `API_RATE_LIMIT_BURST_SIZE` | API: refill rate and size of each client's request bucket. Defaults `5` and `10`. | Bounds request floods and credential guessing per client. |
| `JWT_SECRET` | Secret key for signing/verifying JWTs. | **Must be kept highly secure.** |
| `TLS_CA_CERT_PATH` | Path to the CA certificate file. | Trust root for mTLS. |
| `TLS_CERT_PATH` | Path to the component's client certificate. | Component identity for mTLS. |
| `TLS_KEY_PATH` | Path to the component's private key. | Component identity for mTLS. |
| `TLS_SERVER_NAME` | Expected TLS server name (optional). | Prevents MITM attacks. |
| `MCP_ALLOWED_HOSTS` | Extra `Host` header values the MCP server accepts (optional, comma-separated). | Limits DNS rebinding exposure; list only hostnames you serve. |

## 5. Encryption at rest

Stormchaser does not encrypt data itself before storing it. Protecting data at
rest is the responsibility of whoever operates the data stores: this section
states what must be protected and what Stormchaser expects; the database
administrator implements it.

### PostgreSQL

**Encryption at rest is required for any deployment that handles real
credentials.** Stormchaser writes secrets to PostgreSQL in the clear and
relies on the database being encrypted underneath it.

What the database administrator must provide:

* **Encryption of everything PostgreSQL writes to disk**: the data directory,
  write-ahead log, temporary files, replicas, and every backup and snapshot.
  Core PostgreSQL has no transparent data encryption, so this is done at the
  storage layer (LUKS/dm-crypt, an encrypted volume or storage class, or the
  encryption built into a managed PostgreSQL service) or with a PostgreSQL
  distribution that offers it.
* **TLS for connections from Stormchaser.** Set `STORMCHASER_DB_SSL=true` and
  `TLS_CA_CERT_PATH` on the API, engine and query service so they verify the
  server certificate; without the CA path the connection is not verified.
* **A dedicated, least-privileged database role** for Stormchaser, and control
  over who else can query the database or restore its backups. Storage
  encryption does not protect against anyone who can connect and read tables.
* **No statement parameters in server logs.** Secrets are sent as query
  parameters; leave parameter logging off (`log_parameter_max_length = 0`) and
  avoid `log_statement = 'all'` on this database.
* **Key management and rotation** for whichever encryption is used.

Stormchaser does not use `pgcrypto` or any other column-level encryption, and
none should be layered on from the application side: the database is the
single place this is handled.

So the administrator can judge what is at stake, these are the places secrets
are stored:

| Table | Columns | Contents |
| :--- | :--- | :--- |
| `run_contexts`, `archived_run_contexts` | `secrets`, `sensitive_values`, `inputs` | Decrypted SOPS secrets for a run, the list of secret values used for redaction, and workflow inputs, including those declared secret |
| `step_instances`, `archived_step_instances` | `spec`, `params` | Resolved step definitions, with secrets already substituted in |
| `step_instances` | `completion_token` | A per-step secret that authorises finishing an approval or other engine-handled step |
| `step_outputs` and archived copy | `value` | Step outputs, which may contain secrets a step emitted |
| `outbox_events` | `payload` | Task messages waiting to be published, with the same content as a resolved step |
| `connections` | `config`, `encrypted_credentials`, `client_key` | Credentials for storage, registries, databases, Git and HTTP APIs. Despite its name, `encrypted_credentials` is stored exactly as supplied |
| `webhooks` | `secret_token`, `client_key` | Webhook signing secrets and mTLS client keys |
| `cron_workflows` | `secret_token` | Trigger tokens for scheduled workflows |

The API masks these values in its responses (see the relevant endpoints), but
that is independent of how they are stored.

Run details get the same treatment. A run's decrypted secrets and its
`secret = true` inputs are replaced with `********` wherever they appear in the
step specs, parameters, outputs, errors and log lines the API returns, and an
output marked sensitive is masked whole. This works by matching known values,
so it does not catch a secret that a step transforms (for example
base64-encodes) before printing it, a value fetched with `secret()` from
Vault, or a value shorter than four characters.

The PostgreSQL bundled with docker-compose and the umbrella Helm chart is for
evaluation: it has no encryption at rest and, in the chart, connects with
`sslmode=disable`. Do not store real credentials in it.

### NATS JetStream

Task messages held in JetStream contain the same secrets as a resolved step.
NATS supports encryption at rest, but its documentation recommends
file-system encryption of the store directory instead; that is what
Stormchaser assumes. See [NATS: Quickstart and Production](nats.md).

## 6. Runner Security Considerations

Workflow steps run images chosen by workflow authors, so both runners keep them
away from the control plane:

* **Kubernetes Runner:** steps run in a dedicated namespace, never the
  runner's own. The chart applies a NetworkPolicy there that blocks cluster-internal
  addresses and the cloud metadata endpoint, step pods get no API token, and
  ServiceAccounts, Secrets and ConfigMaps are only attachable when the operator
  allow-lists them.
* **Docker Runner:** steps join the network named by `STORMCHASER_STEP_NETWORK`
  instead of the runner's own. The runner still needs the Docker socket and is
  therefore root on its host; use dedicated hosts.
* **Privileged steps** are refused unless the runner is started with
  `STORMCHASER_ALLOW_PRIVILEGED=true`.

`scripts/setup.sh` relaxes the network isolation and the privileged setting for
local development; see [NATS: Quickstart and Production](nats.md).

### What a step reports about itself

Test reports, storage hashes and artifact metadata are produced inside the step,
so the engine treats them as the workflow author's claims, not as facts:

* A test report is stored only if the step declares it. The engine works out
  where an uploaded report lives from the run, the step and the report name; it
  does not read a location, format or checksum from the step's result. Report
  archives are limited to 32 MiB compressed, 64 MiB unpacked and 1,000 files.
* A storage hash is recorded only for a volume the workflow declares.
* Storage is parked and artifacts are published by a helper the step's code does
  not share: a separate container on the Docker runner, an agent sidecar in the pod
  on the Kubernetes runner. Both runners read storage hashes and artifact metadata
  from that helper's output only, never from the step's, and refuse storage mount
  paths that would cover the helper's system directories (see
  [Mounting Storage in Steps](workflow_dsl.md)).
* Test reports are files anywhere in the step's own filesystem, so on Kubernetes
  they are still collected inside the step's container, with upload URLs for that
  step's own reports only.

### Credentials for artifact registries

The username and password of an OCI artifact backend stay in the engine. They are
not in task messages, pod specs, container arguments or environment variables.
When a step's command has finished, the runner asks the engine for push
credentials, and the engine answers with a token it has just obtained from the
registry's token endpoint for that run's repositories only. The engine answers
only for a step that is with a runner at that moment. The runner hands the token
to the helper alone: written into the helper container's filesystem on Docker, in
a Secret only the sidecar mounts on Kubernetes (deleted with the step's Job).

* The step's own container holds no upload URLs and no registry credentials.
* Any runner credential can ask for the token of any step that is currently
  running, so the token's narrow scope and short life are the limit there.
* The token's lifetime is the registry's (often five minutes). A push that
  outlasts it fails; raise the token lifetime at the registry for large artifacts.
* A registry that cannot issue per-repository tokens gets no credentials unless
  the engine runs with `STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS=true`. The stored
  username and password then go to the helper, still not to the step.
* The Kubernetes runner needs `patch` on pods in the step namespace (to signal the
  sidecar) in addition to `create` on secrets. The chart grants both.

When using the **Docker Runner** with the `STORMCHASER_SFS_HOST_PATH` optimization enabled, the runner creates direct bind mounts from the host file system into step containers.

* **Host Access:** This bypasses Docker's native volume isolation. The directory specified must be securely owned by the user running the Docker daemon.
* **Path Traversal:** The runner sanitizes `provision` destination paths to prevent traversal attacks (`../`) that might attempt to write outside the run's isolated subdirectory.
* **Data Remnants:** Host bind mounts are not automatically garbage collected by the runner. System administrators must ensure secure, automated cleanup (e.g., via cron) to prevent sensitive workflow data from accumulating on the host disk.

## 7. Outbound Request Protection (SSRF)

Workflow authors choose the URLs that `RestApi` and `WebhookInvoke` steps, `api`
queries, Slack and Teams notifications and WASM module downloads connect to, and
they can read the responses. To stop a workflow from reaching the control plane
or a cloud metadata endpoint, the engine, API and query service refuse any such
URL that:

* uses a scheme other than `http` or `https`, or
* resolves to a loopback, private, link-local, carrier-grade NAT, reserved or
  multicast address (including IPv6 forms that embed one of those).

The connection is pinned to the addresses that were checked, and redirects are
not followed, so a `30x` response is returned to the workflow as-is.

Operators can permit specific internal hosts with
`STORMCHASER_SSRF_ALLOWED_HOSTS` (Helm: `global.ssrfAllowedHosts`), a
comma-separated list of hostnames, IP literals or `*.suffix` patterns, for
example `vault.internal,*.svc.cluster.local`. The list is empty by default.
Response bodies are limited to 10 MiB (64 MiB for WASM modules).

The connection test (`POST /api/v1/connections/test`) applies the same address
rules to every connection type it probes. A Git URL must use a scheme the
engine would clone over and a host that is not internal; a Postgres URL must
name a network host (no Unix sockets) that is not internal; an S3 `endpoint` is
checked like any other URL. An S3 test never runs as the API itself: it needs
`access_key` and `secret_key`, or a role listed in
`STORMCHASER_AWS_ALLOWED_ROLE_ARNS`.

### Credentials the engine attaches

A workflow author chooses where a step connects, but some credentials belong to
someone else. The engine only sends those to the server they were issued for:

* **`http_api` connections.** A `RestApi` step that names a connection gets the
  connection's headers and token only when the request goes to the scheme, host
  and port of the connection's `base_url`. Any other URL fails the step.
* **Engine SMTP login.** `SMTP_USERNAME` and `SMTP_PASSWORD` are used only with
  the engine's own `SMTP_SERVER` and `SMTP_PORT`. An email step that names
  another server must supply its own credentials.

* **Registry connections.** A step's `registry_connection` is used only when
  the step's `image` is on that connection's registry (the host and port of its
  `url`; an empty `url` means Docker Hub). Any other image fails the step, and
  the Docker runner checks again before it pulls. On Kubernetes the per-step
  pull secret is owned by the step's Job and is deleted with it.
* **The engine's AWS identity.** The engine assumes a role named by a workflow
  step, a query, a SOPS setting or an S3 connection only if the role is listed
  in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS` (Helm: `global.aws.allowedRoleArns`).
  A `LambdaInvoke` step with no role runs as the engine itself, and its
  function must be listed in `STORMCHASER_AWS_ALLOWED_LAMBDA_FUNCTIONS` (Helm:
  `global.aws.allowedLambdaFunctions`). Both are comma-separated, a trailing
  `*` matches a prefix, and both are empty by default, which refuses every
  role and every function. Scope the engine's own IAM policy as well: these
  lists do not limit SES sending or `aws_cloudcontrol` queries made without a
  role.

WASM modules run with no host imports, a time limit and a memory cap; see
[the Wasm step](steps/Wasm.md#limits).

### Git repositories

Workflow files are read from repositories named by Git connections, which any
API user can create. To keep a repository from being used to read files off
the API or engine host:

* Workflow and included files are read from the git object database, never
  the working tree, so a symbolic link committed to a repository is refused
  rather than followed.
* Repository URLs must use `https` or `ssh` (including the
  `git@host:org/repo.git` form). Local paths and `file://`, `git://`, `http://`
  and `ext::` URLs are rejected. Operators can change the list with
  `STORMCHASER_GIT_ALLOWED_SCHEMES` (comma-separated, for example
  `https,ssh,http`).
* A SOPS file named in a run request must be a regular file inside the
  repository; paths with `..`, absolute paths and symbolic links are refused.

## 8. NATS Authentication

NATS carries every task and every result, so anyone who can publish to it can
schedule containers on runners and report on other users' steps. All components
authenticate to NATS with credentials read from the environment, and runners
should use a separate, restricted NATS user.

The bundled NATS (docker-compose and the umbrella Helm chart) requires
passwords and restricts the runner user, but it is a single node without TLS
and is meant for evaluation. [NATS: Quickstart and Production](nats.md) lists
what the quickstart provides, the exact permissions each role needs, and the
requirements for a hardened production server.

## 9. MCP Server Security

The Model Context Protocol (MCP) server, which exposes the OpenAPI specification as tools for AI agents, is integrated with the same OPA enforcement as the rest of the system:

* **MCP Endpoint Access**: The MCP server is a single Streamable HTTP endpoint mounted at `/api/v1/mcp`, and the same handler answers every path below it. In the default policy, read-only requests remain permitted without a token, and `POST` and `DELETE` on `/api/v1/mcp` are allowed so clients can exchange JSON-RPC messages and end sessions.
* **Host Restriction**: As protection against DNS rebinding, the MCP transport only accepts requests whose `Host` header is a loopback name, the host of `API_BASE_URL`, or a host listed in `MCP_ALLOWED_HOSTS`, and rejects others with `403 Forbidden`. See [MCP Server](mcp-server.md#allowed-hosts).
* **Tool Call Proxying**: The MCP server dynamically invokes local API endpoints as requested by the AI agent. Because these generated internal HTTP calls do not inherently proxy the user's Bearer token, they arrive unauthenticated. *(Note: This is currently the recommended "conformant" way to use `rmcp-openapi`, but they are working on standardizing authorization passthrough in the near future).*
* **OPA Protection**: The default `deploy/opa/policy.rego` includes explicit rules to handle MCP security:
  * Unauthenticated calls initiated by MCP tooling are blocked when they are non-read-only (`POST`, `PATCH`, `DELETE`, etc.), except for explicit protocol/auth endpoints that must accept unauthenticated traffic.
  * Direct calls to the MCP endpoint itself that attempt non-read-only operations are blocked, except for the `POST` and `DELETE` requests the transport itself needs on `/api/v1/mcp` (and on the legacy `/api/v1/mcp/messages` path).
* **Fail-Closed Integration for Writes**: If an AI agent attempts to invoke an MCP tool that performs a state-modifying action (e.g., `POST`, `DELETE`) without explicit authorization handling, the internal API call will be denied by OPA with a `403 Forbidden` response.
