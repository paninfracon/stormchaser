# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Webhooks have a third source type, `signed`, which verifies deliveries the [Standard Webhooks](https://www.standardwebhooks.com/) way: an HMAC-SHA256 signature over the message id, a timestamp and the body (`webhook-id`, `webhook-timestamp`, `webhook-signature`). The secret never travels with the delivery, and a delivery more than five minutes old is refused. See `docs/security.md`.

- `SqlExecute` takes `params`, values for the statement's `$1`, `$2`, ... placeholders, bound by the database rather than written into the SQL. See `docs/steps/SqlExecute.md`.

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
- **The query service's hydration API changed.** `POST /api/v1/schema/hydrate` now takes `connection`, `workflow_path`, `git_ref` and `inputs` instead of `schema` and `queries`; pasted workflows use the new `POST /api/v1/schema/hydrate-direct`. Upgrade the web UI, TUI and query service together: an older client's run forms will not fill in against a newer service. A custom OPA policy that restricts direct runs should restrict `/api/v1/schema/hydrate-direct` the same way.
- **NATS has two more users.** docker-compose needs `STORMCHASER_NATS_API_PASSWORD` and `STORMCHASER_NATS_QUERY_PASSWORD` in `.env` (`scripts/setup.sh` adds them to an existing file). The umbrella chart generates the Secrets `stormchaser-nats-api` and `stormchaser-nats-query` and changes the bundled server's configuration, so the NATS pod restarts on upgrade. With your own NATS server nothing changes until you set `global.nats.apiAuthSecret` and `global.nats.queryAuthSecret`; do that, with the permissions in `docs/nats.md`.
- **Kubernetes runner: drain steps before upgrading.** Steps now read their environment from a per-step Secret. A runner that restarts into the new version picks up Jobs started by the old one as before, but if you set no state encryption key, a step adopted after a restart no longer has its environment values in the definition the runner reads back (nothing the runner does after adoption uses them). Storage restore now needs `curl` to read the URL from the `STORMCHASER_FETCH_URL` environment variable, which the agent image already supports.
- **Upgrade the engine and the runners together.** A runner must now name itself when it asks for a step's registry push token; an older runner's request is refused and its OCI artifacts are not pushed.
- **New limits on workflows.** A workflow file over 1 MiB, an `iterate` over more than 1000 items, a `jq` program that runs for more than 10 seconds or uses more than 512 MiB, and text with more than 16,384 brackets and operators in a file or 8,192 in one expression are now refused. All but the file size can be raised; see "Limits on What a Workflow Can Cost" in `docs/security.md`.
- **Steps now have resource limits by default.** A step that sets no `cpu` or `memory` used to run unlimited and now gets 1 CPU and 2 GiB; a step that needs more must say so, up to the runner's maximum (8 CPUs, 16 GiB by default). Steps that were quietly using more will be throttled or killed for running out of memory until they declare it. Raise `config.stepLimits` on the runner charts if your steps are larger. A step may run for at most 24 hours unless the runner's maximum is raised.
- **TLS is on by default, and switching it off is a deployment choice.**
  - **Database.** The API, the query service and the engine now connect to PostgreSQL with `verify-full` unless told otherwise. Before, they did so only with `STORMCHASER_DB_SSL=true` and a CA configured, and otherwise connected in clear or with `prefer`, which checks nothing. Give them the CA that signed the database's certificate in `TLS_CA_CERT_PATH` unless a public authority did, and a host name in `DATABASE_URL` that the certificate carries. To choose otherwise, write an `sslmode` in `DATABASE_URL` or set `STORMCHASER_DB_SSL=false`. `STORMCHASER_DB_SSL=true` insists on `verify-full` whatever the URL says. The umbrella chart's bundled PostgreSQL has no TLS, so the umbrella chart switches database TLS off for it.
  - **Listeners.** The API, the query service and the web tier now serve HTTPS with the certificate at `TLS_CERT_PATH` and `TLS_KEY_PATH`, and refuse to start without it. Set `STORMCHASER_LISTEN_TLS=false` (Helm: `global.tls.listen=false`) where an ingress or mesh terminates TLS in front of them and the network behind is trusted. With the Helm chart, the certificate must name the api, query and web Services. The umbrella chart generates one, with a CA of its own, unless `tlsCerts.generate=false`, and it adds `backend-protocol: HTTPS` for ingress-nginx to its Ingresses; other ingress controllers need their equivalent. Port-forwards and health checks need `https`.
  - **Settings.** `STORMCHASER_DB_SSL` and `STORMCHASER_LISTEN_TLS` accept only `true` or `false`. Anything else stops the service rather than being read as off.
  - **Local stack.** docker-compose switches both off, because it runs on one machine.
- **Swagger UI is off by default.** `/swagger-ui` and `/api-docs/openapi.json` are served only with `STORMCHASER_API_DOCS=true` (Helm: `api.docs`). docker-compose turns them on.
- **Kubernetes cron triggers need one more permission.** The API's service account needs `create` on `secrets` in its namespace, as well as `create` and `delete` on `cronjobs`. CronJobs made before the upgrade keep working with the token in their arguments; delete and re-create a schedule to move its token into a Secret.
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
- A webhook delivery is acted on once. A repeat of a GitHub delivery (recognised by a hash of its signed body) or of a signed delivery (by its signed id) within `STORMCHASER_WEBHOOK_DEDUPE_HOURS` (default 24) is answered with `"status": "duplicate"` and triggers nothing. Needs a new database migration (`webhook_deliveries`).
- A connection's `config` is masked in API responses unless a key is known to be harmless, instead of only when its name looked like a credential. Values such as a registry `auth` entry, an `X-Api-Key` header, a SAS token in a URL's query string or a credential under an unusual key were returned in clear. Sending the mask back in an update still keeps the stored value.
- The query service no longer runs queries sent in a request. A run form is hydrated from the workflow it belongs to: `POST /api/v1/schema/hydrate` names a Git connection, path and ref, and the service reads the schema and queries from the repository; `POST /api/v1/schema/hydrate-direct` takes workflow text, for direct runs. Previously any signed-in user could post queries of their own and have them run on any connection policy let them use.
- `POST /api/v1/schema/parse-git` and hydration from a repository require that the caller may use the Git connection. Any signed-in user could previously read any file from any registered repository.
- The shipped policy can separate workflow authors from users who may only run workflows: set `author_groups` (Helm: `api.opa.sidecar.settings.authorGroups`). Off by default.
- The API and the query service have NATS users of their own in the quickstart (docker-compose and the umbrella chart) instead of sharing the engine's unrestricted one. The API can queue runs, report approvals and waits, watch run events and keep its rate-limit buckets; the query service can only connect. Neither can publish a task to runners or read one, so a compromise of a user-facing service no longer bypasses the engine's policy check. The lists are in `docs/nats.md`; the orchestration chart takes `global.nats.apiAuthSecret` and `global.nats.queryAuthSecret`.
- Kubernetes runner: a step's environment values and the presigned URLs given to its helper containers are no longer written into the pod. They are kept in one Secret per step, owned by the step's Job and deleted with it, and the pod refers to them by key. Anyone who could read Jobs or pods in the step namespace could previously read every secret a step was given.
- Both runners: the copy of a step's definition kept on its Job or container has its environment and parameter values removed unless `STORMCHASER_STATE_ENCRYPTION_KEY` is set, in which case it is kept whole and encrypted as before. The runner charts can now take the key from a Secret (`config.stateEncryptionSecret`).
- Kubernetes runner: a Job that needs Secrets (a step's environment, a registry pull secret) is created suspended and resumed once they exist, so a pod never starts, or attempts its first image pull, without them.
- The engine accepts a step's result only from the runner the step is with, lets another runner take a step over only once its runner has missed a heartbeat, and gives a step's registry push token only to its own runner. Previously any runner could complete, fail or claim any other runner's step and ask for its token. Runners are still told apart by the id in their messages, not by a credential of their own. A Kubernetes runner that adopts a Job after a restart repeats its claim until the engine accepts it.
- **A workflow could crash the API, the engine or the query service.** The parser for workflow text and expressions has no depth limit, and about 200 nested brackets, or a few hundred `!`, `+` or `?:` in a row, in a workflow file, a step's condition, a template or a rule's expression overflowed the stack and ended the process. Such text is now refused or parsed on a stack sized for it (`STORMCHASER_DSL_MAX_COMPLEXITY`, `STORMCHASER_EXPR_MAX_COMPLEXITY`), workflow files are limited to 1 MiB, and no parsed or evaluated value may nest deeper than 100 levels.
- `jq` programs (`JQ` steps and a query's `jq_filter`) run in a child process with none of the service's environment, a memory ceiling (`STORMCHASER_JQ_MAX_MEMORY_MB`, 512) and a time limit (`STORMCHASER_JQ_TIMEOUT_SECS`, 10). A program that looped or grew for ever used to hold an engine thread and its memory until the engine died.
- An `iterate` over more than `STORMCHASER_MAX_ITERATIONS` items (1000) fails the step instead of creating a step instance per item, and at most `STORMCHASER_WASM_MAX_CONCURRENT` (4) WASM modules run at once.
- Runners enforce a step's `timeout`: the Kubernetes runner sets it as the Job's deadline and the Docker runner stops the container. Until now nothing enforced it, and a step with no timeout ran until it ended by itself. Steps without a timeout get 24 hours (`STORMCHASER_STEP_DEFAULT_TIMEOUT`).
- A run that fails or is aborted now stops its steps on the runners. The engine publishes a cancellation on `stormchaser.v1.runner.cancel`, and each runner stops the run's containers or deletes its Jobs. Until now those steps ran on until they finished or reached their own deadline, using the runner for work whose result nothing would read. Only the engine's NATS account may publish there; no change to NATS permissions is needed if they follow `docs/nats.md`.
- The API, the query service and the web tier serve HTTPS themselves, by default, with the certificate at `TLS_CERT_PATH`, re-read when it is renewed. Until now they could only speak plain HTTP, so bearer tokens and secrets crossed the network to the pod in clear unless an ingress or mesh covered every hop. The web tier trusts `TLS_CA_CERT_PATH` when calling the other two. A service switched to plain HTTP logs a warning at start. The CLI and TUI take `--ca-cert` (`STORMCHASER_CA_CERT`) to trust a private CA, rather than needing it in the system store.
- The API, the query service and the engine log a warning at start when their database connection is not verified TLS.
- A Kubernetes cron trigger's token is kept in a Secret owned by its CronJob and read from a mounted file, instead of sitting in the CronJob's arguments where anyone who can read workloads could see it. The trigger image is pinned by digest (it was `curlimages/curl:latest`; `STORMCHASER_CRON_IMAGE` replaces it), and the trigger pod runs non-root with no capabilities and no service account token.
- Runners hold every step to CPU and memory limits set by the operator. A step that names none gets 1 CPU and 2 GiB instead of no limit at all; one that asks for more than 8 CPUs or 16 GiB is refused (`STORMCHASER_STEP_DEFAULT_*`, `STORMCHASER_STEP_MAX_*`; Helm: `config.stepLimits`). The Docker runner also disables swap for step containers and limits them to 4096 processes; the Kubernetes runner refuses a Job with more than 10 parallel pods or completions.
- **A value spliced into a Jinja template was run as template code.** In fields the engine renders with MiniJinja (`SqlExecute` `query`, `JinjaRender` and `TestReportEmail` `template`, the `body` of `RestApi`, `WebhookInvoke` and `EmailSend`, an approval's `notify.body`), a `${...}` was evaluated and then rendered again, so an input holding `{{ ... }}` or `{% ... %}` ran in the second pass, with the run's inputs and step outputs to hand. A value spliced in with `${...}` is now printed as written. **Upgrade impact:** a workflow that built Jinja code from an input or a step's output must refer to the value in Jinja (`{{ inputs.name }}`) instead. Values for a `SqlExecute` statement belong in its new `params`, which the database binds; spliced into `query`, they are still SQL.
- The runner charts harden their own pods. The Kubernetes runner runs as an unprivileged user with no capabilities, a read-only filesystem and the default seccomp profile, and its Role no longer grants `update` on Jobs or `list`/`watch` on pod logs. The Docker runner drops every capability, has no service account token and a read-only filesystem, and can run as an unprivileged user in the socket's group (`dockerSocketGroup`). Both have resource requests and limits (1 CPU, 1 GiB) by default. **Upgrade impact:** a custom Kubernetes runner image that needs to write outside `/tmp` or run as root must set `podSecurityContext` and `securityContext` itself.
- The web tier accepts only UUIDs as the ids it puts into API paths, in its server functions and its stream routes, and percent-encodes a step's name when approving or rejecting it. An id such as `../../admin` or `x?y=1` was written into the path as it came, so a request could reach a different API endpoint than intended, with the signed-in user's token.
- `GitCheckout` and the Terraform steps no longer build their shell script from the step's values. A repository, ref, destination, sparse path, Terraform workspace, backend setting, region or plan file that came from a run's input could run commands in the step's container, which holds the Git connection's credentials or an assumed AWS role's. The values are now passed as environment variables and arguments, git gets `--` before each one, and git may only use the transports in `STORMCHASER_GIT_ALLOWED_SCHEMES` (`ext::` and `file://` are refused). **Upgrade impact:** a `GitCheckout` repository must be an `https` or `ssh` URL unless `STORMCHASER_GIT_ALLOWED_SCHEMES` on the engine says otherwise, and a Terraform `out_file` may not start with `-`.
- With both a Docker and a Kubernetes runner connected, a `RunContainer` step ran on both: each kind of runner had a consumer of its own on every container task, and the Kubernetes runner's took every task type. Runners now share one consumer for container tasks (`runner-container-tasks`), so each goes to one runner, and Kubernetes runners have another for `RunK8sJob` (`runner-k8s-job-tasks`). **Upgrade impact:** the old `docker-runner` and `k8s-runner` consumers are no longer read and can be deleted (`nats consumer rm stormchaser docker-runner`, likewise `k8s-runner`). No NATS permission changes are needed.
- Docker containers and volumes and Kubernetes Jobs are named with a run's or step's full id instead of its first eight characters, so two runs can no longer share a storage volume, or two steps a container or Job, by chance.
- The images `GitCheckout` and Terraform steps run in are pinned by digest (`alpine/git` 2.54.0, `hashicorp/terraform` 1.16.6) instead of `latest`; `STORMCHASER_GIT_IMAGE` and `STORMCHASER_TERRAFORM_IMAGE` replace them. The Docker runner's agent image can be set with `STORMCHASER_AGENT_IMAGE` (chart: `config.agentImage`).
- Stormchaser's images run as an unprivileged user (uid 65532) instead of root, and the `oras` download in them is checked against its published SHA-256. The orchestration chart runs the API, query service, web tier and engine non-root with no capabilities (`podSecurityContext`, `securityContext`); its `fsGroup` keeps the engine's git cache volume writable. Runners start agent containers as root explicitly, since the agent writes into steps' volumes, and the Docker runner runs as root with no capabilities in compose and in its chart unless `dockerSocketGroup` is set. Chart images default to the chart's release (`v<appVersion>`) instead of `latest`, and the Docker runner chart's image is `stormchaser-runner-docker`. **Upgrade impact:** with docker-compose, remove the engine's old git cache volume (`docker volume rm <project>_git-cache`), which earlier images filled as root; a deployment that relied on `latest` must name its tag; an image built from these Dockerfiles for something that needs root must set its user.
- docker-compose publishes Loki, Tempo, Prometheus and Grafana on loopback only (as it already did Postgres, NATS and OPA; `STORMCHASER_CONTROL_PLANE_BIND` overrides it), and Grafana's anonymous visitors are Viewers instead of Admins. Anyone who could reach the host could read every step's logs and change Grafana's datasources. The umbrella chart admits only pods in the release's namespace to its Loki (`lokiNetworkPolicy`), so workflow steps cannot read other runs' logs. The bundled observability stack is for the quickstart; production uses an existing log backend, which must require authentication (see `docs/environments.md`).
- CI and release workflows pin every action to a commit SHA, with Dependabot keeping the pins current, and run with read-only tokens except where a release job publishes (binaries: `contents: write`; images: `packages: write`). Checkouts in the release workflow no longer keep the token in the repository's git config.
- Upgraded `rustls` to 0.23.46 (RUSTSEC-2026-0346, -0347, -0348, -0349). Replaced the unmaintained `rustls-pemfile` (RUSTSEC-2025-0134) with the PEM reader in `rustls-pki-types`, and removed ten advisory ignores from `deny.toml` that no longer matched anything; the two left say why.
- Cron schedules work on Kubernetes out of the box: the orchestration chart gives the API a ServiceAccount with a Role that may create and delete CronJobs and create Secrets in its own namespace, and sets `CRON_ENGINE` (`api.cron.engine`) and `SYSTEM_URL` to the API's own Service, over HTTPS with the chart's CA. Until now the API had no permission to create CronJobs, and triggers called `localhost`.
- **Removed the `ofelia` cron engine.** Ofelia only takes `job-run` jobs from its own container, so the labelled containers the API created were never scheduled; their only effect was to keep each schedule's trigger token in a container label. The Docker quickstart now sets `CRON_ENGINE=none`, which stores the schedule and shows its token once for a scheduler of your own. Previously it fell back to `kubernetes` there and creating a schedule failed. **Upgrade impact:** `CRON_ENGINE=ofelia` is refused; remove leftover `stormchaser-cron-*` containers.
- More of a run's secrets are masked in what the API returns. Each secret is also matched base64-encoded (including inside a longer encoding, such as an HTTP Basic credential), URL-encoded, JSON-escaped and hex-encoded. Values fetched with `secret()` from Vault, and credentials the engine hands to steps (connection passwords in `STORMCHASER_CONN_*`, Git credentials, registry passwords), are now recorded for the run and masked like its other secrets; before, only the run's SOPS values and secret inputs were. Errors in the run list and the run stream are scrubbed as on a run's own page.
- **Runners prove which runner they are.** All runners share one NATS credential, and a runner named itself only in its messages, so any runner, or anything that obtained the credential, could report on, complete or fail another runner's steps, mark it offline, or ask for its artifact push token. Each runner now gets a key of its own from the engine when it starts, sealed so that only it can read it, and signs everything it reports; the engine ignores reports that are not signed by the runner they name, and will not issue a key for an id whose runner is still sending heartbeats. A step that no runner has claimed yet takes a result only from a runner with a key. See "Runner identity" in `docs/security.md`. **Upgrade impact:** upgrade runners with the engine, or set `STORMCHASER_RUNNER_SIGNATURES=optional` on the engine until they are; a runner that restarts without going offline cleanly waits up to 30 seconds to register; runners need to publish to `stormchaser.rpc.runner.session` (in `deploy/nats/nats-server.conf` and the umbrella chart).
- Fixed: a runner's requests to the engine (artifact push tokens, and whether a step it found after a restart is still wanted) were answered by the JetStream stream's acknowledgement instead of by the engine, because the stream captured their subjects, so artifact pushes and orphan adoption failed. Requests now use `stormchaser.rpc.*`, which the stream does not capture; runners create the stream with the engine's subjects (`stormchaser.v1.>`), and the engine narrows a stream an earlier runner created with `stormchaser.>`. **Upgrade impact:** runners need publish permission on `stormchaser.rpc.step.query` and `stormchaser.rpc.step.artifact_auth` instead of the `stormchaser.v1.step.*` subjects of the same names.
- Fixed the quickstarts after the move to non-root images and the June hardening, found by running them: the Docker quickstart's services could not read `tests/certs/tls.key`, which belongs to the user on the host (a one-shot `certs` service now copies the certificate and key into a volume the services' user owns); nobody could log in on a release build of the API, because neither docker-compose nor the Helm chart set `STORMCHASER_JWT_SECRET` (`scripts/setup.sh` now generates it into `.env`, `generate_dev_token.py` signs with it, and the umbrella chart generates the Secret `stormchaser-jwt`, read through `global.jwt.existingSecret`); and the umbrella chart's PostgreSQL and MinIO images no longer exist where it pulled them from (now `docker.io/bitnamilegacy`, unmaintained, for the quickstart only). **Upgrade impact:** add `STORMCHASER_JWT_SECRET` to an existing `.env` (re-running `scripts/setup.sh` does), and give the orchestration chart a JWT Secret if you do not use the umbrella chart.
- An API request could skip the per-object ownership check by percent-encoding a character of the object's id (`/webhooks/<id with %2D>`): the check read the raw path, failed to parse the id and let the request through, and the handler acted on the decoded id. Paths are now decoded before the check, and the path sent to OPA is decoded too.
- Runner identity, tightened after a second review:
  - Each NATS user may now read only its own inbox prefix, `_INBOX_<NATS_USER>` (or `NATS_INBOX_PREFIX`). On the shared `_INBOX.>`, the API, query and runner users could read the engine's pull deliveries (every runner's reports) and the answers the engine sent to others. A step's registry push token is also sealed with the asking runner's key, since runners share a user.
  - A runner's signature now covers the subject it is published on, so a signed report can no longer be replayed on another subject (as a runner going offline, say).
  - A storage report (`step.unpacking_sfs`, `step.packing_sfs`) moves a step only from a state where that makes sense, and only for the runner that holds it. Before, any runner with a key could name itself the holder of any step, finished ones included, and then report its result. A step's start is likewise accepted only from the runner that holds it. The Docker runner now signs these reports; they were sent unsigned and so ignored.
  - The engine's own verdict that a step's runner is lost is no longer ignored when that runner still has a key, and no runner can take a key for the ids the engine reports its SQL and REST API steps under.
  - A key cannot be issued for a runner id that was given one in the last 30 seconds, so a starting runner cannot lose its key before its first heartbeat; the check and the write are now one statement.
  - Runners may no longer create the engine's own consumers again, which would have changed what the engine receives.

  **Upgrade impact:** upgrade the engine and the runners together (signatures changed form); a NATS server you configure yourself needs the per-user inbox permissions in `docs/nats.md`, and a component logging in with a credentials file or a token needs `NATS_INBOX_PREFIX` set to the prefix its user may read.
- The SSRF guard could be bypassed with a trailing dot in the host name (`http://internal.example./`): it checked and pinned the name without the dot, while the HTTP client looked the pin up with it, missed, and resolved the name again when it connected, so a name that resolved differently the second time (DNS rebinding) reached internal addresses. The request now goes to exactly the name that was checked. The guarded client also ignores `HTTP_PROXY`/`HTTPS_PROXY`, since a proxy resolves the name again itself. **Upgrade impact:** outbound HTTP from the engine (webhooks, REST API steps, Slack and Teams, registry tokens, WASM modules) and the API's connection tests no longer go through a proxy set in the environment.
- Any user could attach an event rule to another user's webhook, so every delivery to it started the attacker's workflow with the delivery's payload as inputs. Creating a rule now needs permission to change the webhook (its owner or an admin, under the shipped policy); a webhook that does not exist answers 404.
- A step's test-report archive could exhaust the engine's memory despite the unpacking limits: the `tar` crate reads a GNU long-name or PAX record whole before the limits applied, so a small, highly compressed archive with a record of gigabytes was read into memory. Every byte out of the decompressor now counts against a cap (the 64 MiB file budget plus room for headers), every entry counts against a limit of 10,000, and the archive is unpacked off the engine's async workers.
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
