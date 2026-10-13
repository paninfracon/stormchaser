# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Upgraded all dependencies to their latest releases, including Axum 0.8, Leptos 0.8, SQLx 0.9, reqwest 0.13, kube 4, schemars 1, MiniJinja 3 and jaq 3.
- The minimum supported Rust version is now 1.96, declared as `rust-version` in the workspace manifest. The Docker builder images use `rust:1.96`.
- The Kubernetes runner is now built against the Kubernetes 1.32 API (previously 1.31).
- Services now validate outbound TLS against the system certificate store instead of a bundled root list. The agent bundles the Mozilla roots and merges them with the system store, so it keeps working in task images without CA certificates.
- Templates are rendered with MiniJinja 3, which aligns more closely with Jinja2 (for example `tojson` separator spacing, round-half-even rounding, and an error on division by zero).
- Generated JSON Schemas remain Draft 7; documented enum variants are now expressed with `const` instead of a single-value `enum`.
- A `JQ` step whose program calls `halt` now fails the step.
- **Upgrade notes for the security changes below.** Each of these fails closed until configured:
  - Workflows that use `env()` need the variables adding to `STORMCHASER_ENV_ALLOWLIST`.
  - `sql` queries with no `connection` no longer run; name a connection with a `url`.
  - Workflows with `privileged = true` need `STORMCHASER_ALLOW_PRIVILEGED=true` on the runner.
  - Every IAM role in use, including an S3 storage connection's `aws_assume_role_arn`, needs adding to `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`.
  - Steps that call internal hosts over HTTP need those hosts adding to `STORMCHASER_SSRF_ALLOWED_HOSTS`. Redirects are no longer followed.
  - NATS clients need credentials (`NATS_USER` and `NATS_PASSWORD`, a creds file or a token).
  - Kubernetes runner: Jobs still running in the old shared namespace are no longer seen after the upgrade and need manual cleanup. An SFS PVC must live in the step namespace. ServiceAccounts, Secrets and ConfigMaps used by steps need allow-listing.
  - Workflows that use `secret()` need their paths adding to `STORMCHASER_VAULT_ALLOWED_PATHS`, and the engine needs `VAULT_TOKEN` set explicitly.
  - A `RestApi` step with a connection must call the connection's own origin, and an email step that names another SMTP server must supply its own credentials.
- **The OIDC client secret needs configuring.** The Helm charts no longer default it to `stormchaser-cli-secret`. Set `global.oidc.existingSecret` or `global.oidc.clientSecret` to the secret registered with your identity provider, and rotate it if you were using the default. `stormchaser login` no longer takes `--issuer` or `--client-id` (both are ignored) and stores the token in a profile.
- **Webhooks without a secret stop working.** Give each one a secret (`PATCH /api/v1/webhooks/{id}` with `secret_token`) and configure the sender to use it. Event-type patterns in rules are now anchored: a pattern that relied on matching part of an event type needs `.*` added.
- **Approval links need configuring.** Set `STORMCHASER_APPROVAL_LINK_KEY` (32+ characters, the same on the engine and the API; Helm: `global.approvalLinks.existingSecret`) or approval emails carry no links. Links already sent stop working. `POST /api/v1/approve-link/{token}` now performs the action and `GET` returns a confirmation page; `stormchaser runs approve-link` uses `POST`.
- **Upgrade notes for ownership.** Applies when OPA is configured. A new migration adds `created_by` to connections, webhooks, event rules and cron schedules; existing rows have no owner. With the shipped policy, users stop seeing each other's runs, webhooks, rules and schedules, and runs started by a rule or schedule belong to whoever created it. Configure `admins` or `admin_groups` in the policy settings (Helm: `api.opa.sidecar.settings`); until you do, objects with no owner stay changeable by everyone. A custom policy must answer the new resource question (see `docs/opa-rbac.md`); the old connection question (`connection_name`, `initiating_user`) is no longer asked. The Helm value `api.opa.sidecar.policy` now defaults to empty, meaning the shipped policy.
- OPA policies: the query service asks about connection use as the calling user instead of the fixed `schema_hydration_service`, and the engine's input for queued runs no longer includes schema defaults derived from queries.
- API responses: `secrets` is no longer part of run details, and stored secrets come back masked. Sending the mask back in an update keeps the stored value.
- Storage `mount_path` values that are system locations (for example `/`, `/usr`, `/etc`, `/bin`, `/stormchaser`) are refused on both runners.
- Kubernetes runner: an artifact's `path` is now relative to its storage volume, as on the Docker runner (it was relative to the container's working directory). Volumes on a shared PVC are no longer also parked to object storage.
- Kubernetes runner: the runner Role needs `patch` on pods in the step namespace, and the runner's NATS user needs to publish on `stormchaser.v1.step.artifact_auth`. Drain running steps before upgrading: jobs started by an older runner lose their storage hashes and artifacts.
- The engine waits for NATS to acknowledge each task message, and needs a new database migration (`dispatched_task_messages`).

### Fixed

- A runner that restarts keeps its identity and its steps. Runner ids now default to the host or pod name (`docker-<host>`, `k8s-<pod>`) instead of a new random id on every start, and a runner that adopts a step's container or job tells the engine, which moves the step to it. Previously the engine failed such steps as lost about 30 seconds after the restart even though they were still running.
- OCI artifacts are now pushed from the Docker runner. `oras` refuses absolute paths, so these pushes always failed.
- The MCP server can now be reached through a non-loopback hostname. It accepts the host of `API_BASE_URL` and any hosts listed in the new `MCP_ALLOWED_HOSTS` setting (Helm: `api.mcpAllowedHosts`), in addition to loopback names.
- The default OPA policy now permits the MCP transport's `POST` and `DELETE` requests on `/api/v1/mcp`. Previously only `POST /api/v1/mcp/messages` was permitted, so clients using the real endpoint were denied.

### Removed

- The embedded OPA WASM runtime no longer provides the YAML builtins (`yaml.marshal`, `yaml.unmarshal`, `yaml.is_valid`).
- Removed the unused `sqlx` feature from `stormchaser-tls`.

### Security

- Approval links are sealed with their own key (`STORMCHASER_APPROVAL_LINK_KEY`), expire, open a confirmation page instead of acting on a GET, go through the approval policy, record who they were sent to, and are kept out of request logs. The engine used to fall back to the key `test-secret`.
- **Object-level authorization.** Policy is now asked who may act on each run, connection, webhook, event rule and cron schedule, with the object's owner and the caller's verified identity. The shipped policy lets users see and change only what they created, shares connections for reading and use, and lets configured admins do anything. Lists and the runs stream are filtered the same way. Previously any authenticated user could read, change or delete everything.
- The engine and query service ask policy before using a connection on a run's behalf, as the run's user.
- The shipped policy refuses anything it does not recognise instead of allowing by default, and the Helm chart now ships it in place of an allow-all policy. OPA's own API is restricted to evaluating the policy, so it cannot be replaced over the network.
- Request-level policy is shown the path with its `/api/v1` prefix. Path rules in the shipped and example policies never matched before.
- A test report is only served under the run it belongs to.
- Outbound HTTP requests chosen by workflow authors (`RestApi` and `WebhookInvoke` steps, `api` queries, Slack and Teams notifications, WASM module downloads, the API's connection test) are refused when they point at loopback, private, link-local, carrier-grade NAT or other internal addresses, including IPv6 forms that embed an IPv4 address. Connections are pinned to the validated addresses and redirects are not followed. Operators can exempt hosts with `STORMCHASER_SSRF_ALLOWED_HOSTS` (Helm: `global.ssrfAllowedHosts`). Response bodies are capped at 10 MiB and WASM modules at 64 MiB.
- `env()` in workflow expressions only reads variables listed in `STORMCHASER_ENV_ALLOWLIST` (empty by default), and `jq` programs can no longer read the engine's environment. Rotate any secret that was in the engine's environment.
- `sql` queries never run on the engine's own database: they must name a connection, run as a single statement in a read-only transaction that is rolled back, and are limited to 30 seconds and 10,000 rows. The query service checks connection use as the authenticated caller. The policy check for queued runs happens before any query executes.
- Privileged steps are refused unless a runner is started with `STORMCHASER_ALLOW_PRIVILEGED=true` (Helm: `config.allowPrivileged`).
- Kubernetes runner: steps run in a dedicated namespace (`STORMCHASER_STEP_NAMESPACE`, by default `<release namespace>-steps`) with a NetworkPolicy that blocks cluster-internal addresses. Step pods get no API token, and the ServiceAccounts, Secrets and ConfigMaps a `RunK8sJob` step names must be allow-listed by the operator. The runner's Role is reduced accordingly, and author-supplied labels and annotations cannot replace Stormchaser's own.
- Docker runner: step containers join `STORMCHASER_STEP_NETWORK` instead of the runner's network. The compose file publishes the Postgres, NATS and OPA ports on loopback only.
- NATS requires authentication in the quickstart (docker-compose and the umbrella chart), with a separate, restricted user for runners. All components read NATS credentials and TLS settings from the environment. See `docs/nats.md`.
- A step event is rejected if its step does not belong to the run it names, and steps the engine finishes itself (approvals, waits, built-in steps) require a per-step completion token, so a runner credential cannot complete an approval. A registered WASM step definition can no longer replace a built-in step type.
- Every route in the API's protected group now requires a valid token. `POST /api/v1/events/correlate` was reachable without one.
- Run details no longer return a run's decrypted secrets. Secret inputs are masked, and a run's known secret values are replaced with a mask in step specs, parameters, outputs, errors and log lines returned by the API. Outputs marked sensitive are masked whole.
- Connection credentials, webhook signing secrets and cron trigger tokens are masked in API responses. A cron trigger token is shown once, when the schedule is created.
- Workflow and include files are read from the git object database, so a symlink committed to a repository is refused instead of followed. SOPS files may not be or pass through a symlink. Repository URLs are limited to `https` and `ssh` (`STORMCHASER_GIT_ALLOWED_SCHEMES`).
- A `RestApi` step that names an `http_api` connection gets the connection's headers and token only when the request goes to the connection's own `base_url`.
- The engine's SMTP username and password are used only with the engine's own SMTP server, and a step cannot turn off TLS the operator turned on while they are in use.
- WASM steps have a wall-clock limit (30 s, `STORMCHASER_WASM_TIMEOUT_SECS`), a memory cap (256 MiB, `STORMCHASER_WASM_MAX_MEMORY_MB`) and a 10 MiB result limit.
- The engine assumes an IAM role only if it is listed in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS`, and a `LambdaInvoke` step without a role may only invoke functions listed in `STORMCHASER_AWS_ALLOWED_LAMBDA_FUNCTIONS` (Helm: `global.aws.allowedRoleArns`, `global.aws.allowedLambdaFunctions`). Both are empty by default.
- A step's image must be on the registry its `registry_connection` belongs to, so pull credentials are never sent to another registry. The Kubernetes runner's per-step pull secret is now deleted with the step's Job.
- Encryption at rest for the database is a documented requirement on the database administrator; Stormchaser adds no encryption layer of its own. See `docs/security.md`.
- A step can no longer make the engine read arbitrary objects from storage: test reports are accepted only if the step declares them, their location is worked out by the engine, and report archives are size-limited (32 MiB compressed, 64 MiB unpacked, 1,000 files).
- Storage is parked and artifacts are published by a helper the step's code does not share (an agent sidecar on Kubernetes, the parking container on Docker), and runners read storage hashes and artifact metadata from that helper only. The step's container no longer receives upload URLs.
- The username and password of an OCI artifact registry no longer leave the engine. Runners get a token scoped to the run's repositories, minted when the step is ready to push. Registries that cannot issue such tokens need `STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS=true` on the engine.
- Task messages, which carry a step's resolved secrets, are erased from the NATS stream once the step is finished, and the stream has a maximum message age (`STORMCHASER_NATS_STREAM_MAX_AGE_HOURS`, default 168).
- The connection test applies the internal-address rules to Git, Postgres and S3 targets as well as HTTP ones, refuses local Git paths and Postgres Unix sockets, and never runs an S3 test as the API itself: it needs `access_key` and `secret_key`, or a role listed in `STORMCHASER_AWS_ALLOWED_ROLE_ARNS` (now also passed to the API by the Helm chart).
- A `lost_zombie` step failure is acted on only when the step's runner really is offline or stale, so a runner cannot fail another runner's step as lost.
- Logs no longer contain evaluated template results, raw event payloads, a step's definition on a parse failure, the values that failed event schema validation, or a step command's arguments. The engine and API log at `info` unless `RUST_LOG` says otherwise (they defaulted to `debug`), as do the services in the compose file.
- `secret()` reads only the Vault paths listed in `STORMCHASER_VAULT_ALLOWED_PATHS` (empty by default), and the engine no longer falls back to the Vault token `root` when `VAULT_TOKEN` is unset: lookups are disabled instead.
- Step names are limited to letters, digits, `_` and `-`, and a step name can no longer alter the log query used to fetch its logs.
- The API rate limiter is a token bucket per client (`API_RATE_LIMIT_BURST_SIZE` requests, refilled at `API_RATE_LIMIT_PER_SECOND`; the rate was previously ignored). Behind a reverse proxy, set `STORMCHASER_TRUSTED_PROXIES` (Helm: `api.trustedProxies`) so clients are told apart by `X-Forwarded-For`; the header is ignored from any other source. IPv6 clients are counted per /64. The `TEST_BYPASS_RATE_LIMIT` switch is gone.
- The run log and status streams return 404 for a run that does not exist and stop polling the database when the client disconnects or the run is removed.
- Browser login from the CLI and TUI carries a random `state` and a PKCE challenge, listens before the browser opens, and ignores anything on the callback port that is not the answer to that login. The CLI no longer contains an OIDC client secret or talks to the identity provider itself: like the TUI, it hands the authorization code to the API. `GET /api/v1/auth/login` accepts `code_challenge` and `POST /api/v1/auth/exchange` accepts `code_verifier`.
- `stormchaser login` stores the token in a profile instead of printing it (`--print-token` prints it). `profiles.json` is written with mode 0600 in a 0700 directory, and a profile's token is no longer sent to a different server named with `--url`.
- The OIDC client secret has no default. The Helm value `global.oidc.clientSecret` was `stormchaser-cli-secret`; set `global.oidc.existingSecret` (key `client-secret`) or `global.oidc.clientSecret`. The query service no longer receives the secret, and `scripts/generate_dex_config.py` requires `STORMCHASER_CLI_SECRET`.
- The CLI removes terminal control characters from log lines, stream events and error bodies before printing them to a terminal, so a step's output cannot rewrite the screen. Output that is piped or redirected is left as it is.
- A token naming a signing key the API does not know triggers at most one key-set refresh per 30 seconds, and a refresh that fails or returns no keys leaves the cached keys in place. Previously every such token refetched the keys, and a failed fetch emptied the cache.
- The web UI no longer writes the runs list to its log, the request's cookies (session token included) to its log, or stream events to the browser console.
- The TUI offers the development auto-login from `deploy/dex/credentials.generated` only when `STORMCHASER_TUI_DEV_LOGIN=true`.
- Webhooks must have a secret: one is required to create a webhook, it can be replaced but not cleared, and a webhook without one refuses every delivery with 403 (previously it accepted them unauthenticated). Rules no longer see the `Authorization`, `Cookie` and signature headers of a delivery, and a rule's event-type pattern must match the whole event type.
- Repository clones are staged and moved into the cache only when complete and on the requested revision. A failed clone, or a revision that did not exist, used to leave a cache entry that later runs used as if it were good, running the default branch's workflow instead. Cached branches and tags are fetched again after `STORMCHASER_GIT_CACHE_REFRESH_SECS` (default 60); they were never refreshed. Downloads are limited to `STORMCHASER_GIT_MAX_CLONE_MB` (default 1024) and `STORMCHASER_GIT_CLONE_TIMEOUT_SECS` (default 300), and the engine no longer clones on its async executor.
- The web UI sends `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin` and a Content-Security-Policy on every response, and `Strict-Transport-Security` when `WEB_URL` is HTTPS. Pages allow scripts only from the site itself or with that response's nonce, may not be framed, and may frame only the configured Grafana. Logging out is a `POST` that other sites cannot trigger; `GET /auth/logout` no longer exists.
- Removed `h2` 0.3 (RUSTSEC-2026-0258) by building the AWS SDK without its legacy HTTP client.
- Removed the unmaintained `unsafe-libyaml` (RUSTSEC-2024-0449) by replacing `serde_yaml`.

## [1.5.1] - 2026-05-25

### Fixed

- Patch release to resolve a crates.io publishing issue caused by immutability of version 1.5.0 which lacked the sorting tables feature on the initial tag.

## [1.5.0] - 2026-05-25

### Added

- Added the new `stormchaser-web` crate containing the Web UI component built with Leptos.

## [1.4.2] - 2026-05-21

### Added

- Add Zombie execution detection for runners losing connection.
- Add SOPS secrets integration.
- Add Fencing tokens for concurrency control.
- Add Sticky sharding support.
- Add Workflow Input Schemas to define schema properties for inputs and workspace parameters.
- Add the intrinsic `SqlExecute` step for engine-side database query execution.
- Add the intrinsic `RestApi` step for engine-side HTTP calls with templated requests and response extraction.

### Changed

- Refactor connections handling and dynamic queries security.
- Refine TUI and DSL.
- Prepare `stormchaser-tui` for publishing by removing the `publish = false` directive.
- Add ChatOps Slack/Teams intrinsic dispatch updates to publish v1 CloudEvents and avoid duplicate step-completion transitions.
- Document that schema hydration is served by `stormchaser-query` at `/api/v1/schema/hydrate` (the API service only serves `/api/v1/schema`).

## [1.3.2] - 2026-05-09

### Added

- feat(runner-docker): Implement SFS optimization via Host Bind Mounts (#23)
- feat: Add MCP Server support (#22)

### Changed

- Replace ratatui form (#25)
- Refactor long branches (#24)

### Fixed

- Fix/string literals (#21)

## [1.3.1] - 2026-05-08

### Added

- docs: add crate-specific README.md files for crates.io publishing

## [1.3.0] - 2026-05-07

### Added

- Feature/strongly typed ids (#20)
- Nats messaging improvements (#16)
- Comprehensive unit and snapshot tests for dialogs and panes (#19)

### Changed

- Reduce cyclomatic complexity in parser and dispatch (#17)
- Shorten long Rust entity paths (#18)

### Fixed

- Fixed CI formatting issues in `stormchaser-runner-k8s`.
- Refactored `stormchaser-engine/src/handler/integrations/email/mod.rs` to extract test report logic and adhere to the 500-line rule.
- Refactored `stormchaser-api/src/routes/webhook.rs` to extract event rules logic.
- Refactored `stormchaser-engine/src/handler/step/intrinsic/terraform.rs` into a module, separating tests.
- Refactored `stormchaser-tls/src/lib.rs` and `stormchaser-api/src/lib.rs` to adhere strictly to the wiring facade pattern by moving business logic into sub-modules.

## [1.2.0] - 2026-05-03

### Added

- HCL standard functions implementation.
- Terraform steps and examples.
- Schema support for workflows and steps.
- Celery to feature comparison matrix and updated n8n docs.
- OPA integration examples in documentation.

### Changed

- Refactored orchestrator for test environments, runners, and TUI improvements.
- Split `k8s_utils.rs` and API `db.rs` into domain modules.
- Split semantic testing for unit and integration tests.
- Refactored quality pass for test integration.

### Fixed

- Resolved dogfood pipeline and runner parsing issues.
- Fixed long path names.

## [0.1.0] - 2026-04-29

### Added

- Graph-based workflow DSL.
- Initial distributed execution using K8s and Docker runners.
- Event-driven orchestration with NATS JetStream.
- Postgres state management.
- Native SFS (Shared File System) support with object storage verification.
- Human-in-the-loop approvals and native email dispatch.

### Changed

- Refactored orchestrator to handle job cleanup and adoption.

### Fixed

- Fixed axum trait bounds in the API.
- Fixed log stream proxying.
