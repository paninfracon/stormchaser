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
- Storage `mount_path` values that are system locations (for example `/`, `/usr`, `/etc`, `/bin`, `/stormchaser`) are refused on both runners.
- Kubernetes runner: an artifact's `path` is now relative to its storage volume, as on the Docker runner (it was relative to the container's working directory). Volumes on a shared PVC are no longer also parked to object storage.
- Kubernetes runner: the runner Role needs `patch` on pods in the step namespace, and the runner's NATS user needs to publish on `stormchaser.v1.step.artifact_auth`. Drain running steps before upgrading: jobs started by an older runner lose their storage hashes and artifacts.
- The engine waits for NATS to acknowledge each task message, and needs a new database migration (`dispatched_task_messages`).

### Fixed

- OCI artifacts are now pushed from the Docker runner. `oras` refuses absolute paths, so these pushes always failed.
- The MCP server can now be reached through a non-loopback hostname. It accepts the host of `API_BASE_URL` and any hosts listed in the new `MCP_ALLOWED_HOSTS` setting (Helm: `api.mcpAllowedHosts`), in addition to loopback names.
- The default OPA policy now permits the MCP transport's `POST` and `DELETE` requests on `/api/v1/mcp`. Previously only `POST /api/v1/mcp/messages` was permitted, so clients using the real endpoint were denied.

### Removed

- The embedded OPA WASM runtime no longer provides the YAML builtins (`yaml.marshal`, `yaml.unmarshal`, `yaml.is_valid`).
- Removed the unused `sqlx` feature from `stormchaser-tls`.

### Security

- A step can no longer make the engine read arbitrary objects from storage: test reports are accepted only if the step declares them, their location is worked out by the engine, and report archives are size-limited (32 MiB compressed, 64 MiB unpacked, 1,000 files).
- Storage is parked and artifacts are published by a helper the step's code does not share (an agent sidecar on Kubernetes, the parking container on Docker), and runners read storage hashes and artifact metadata from that helper only. The step's container no longer receives upload URLs.
- The username and password of an OCI artifact registry no longer leave the engine. Runners get a token scoped to the run's repositories, minted when the step is ready to push. Registries that cannot issue such tokens need `STORMCHASER_OCI_ALLOW_STATIC_CREDENTIALS=true` on the engine.
- Task messages, which carry a step's resolved secrets, are erased from the NATS stream once the step is finished, and the stream has a maximum message age (`STORMCHASER_NATS_STREAM_MAX_AGE_HOURS`, default 168).
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
