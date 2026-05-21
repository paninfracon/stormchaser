# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.4.0] - 2026-05-21

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
