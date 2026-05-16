# NATS Schema Improvements Plan

## Background & Motivation

The current NATS messaging implementation uses ad-hoc JSON payloads and unversioned subjects (e.g., `stormchaser.run.completed`). As the system grows, this creates a risk of breaking consumers when event schemas evolve. We need to implement CloudEvents conformance, versioning (both subject-based and header-based), and an OCI-backed schema registry to ensure robust, backwards-compatible, and discoverable eventing.

## Scope & Impact

* **Target Components**: `stormchaser-engine`, `stormchaser-api`, `stormchaser-runner-*` crates, and any place where `js.publish` or `nats.publish` is called.
* **Dependencies**: Introducing `cloudevents-sdk` to standard crates.
* **OCI Schema Registry**: Implementing a caching mechanism for schemas pulled from an OCI container repository.
* **Future Note**: We will add a note in `docs/future_todo.md` to consume event streams from the OCI schema registry to trigger cache updates dynamically.

## Proposed Solution

We will adopt the **Permissive (Background Sync)** approach combined with the official `cloudevents-sdk` crate.

### 1. CloudEvents Implementation

* Add the `cloudevents-sdk` crate to workspace dependencies.
* Refactor all `js.publish` calls across the ecosystem to wrap their inner JSON payloads inside a CloudEvents envelope.
* Standardize attributes:
  * `type`: The fully qualified event name (e.g., `stormchaser.run.started`).
  * `source`: The originating component (e.g., `/stormchaser/engine`).
  * `subject`: The specific entity ID (e.g., `run-id`).
  * `data`: The existing JSON payload.

### 2. Subject & Header Versioning

* **Subject Pattern**: Update all subjects to include a version token.
  * Old: `stormchaser.run.queued`
  * New: `stormchaser.v1.run.queued`
* **NATS Headers**: Include metadata in the NATS message headers (distinct from the CloudEvent payload headers).
  * `Nats-Msg-Schema-Version`: e.g., `1.0`
  * `Content-Type`: `application/cloudevents+json`
  * `Schema-ID`: A reference to the schema in the OCI registry.

### 3. OCI Schema Version Repository & Caching

* Implement a background sync service in `stormchaser-engine` and `stormchaser-api` that periodically fetches JSON schemas from configured OCI registries.
* The schemas will be stored in an in-memory cache (`RwLock<HashMap<String, Value>>`).
* Validation against these schemas is permissive (optional/asynchronous) so it does not block the critical path of NATS publishing and subscribing.
* Add a documentation note outlining NATS Subject Mapping for future migrations transparently.

## Implementation Steps

1. **Dependencies & Future Todos**:
    * Add `cloudevents-sdk` to `Cargo.toml`.
    * Update `docs/future_todo.md` with the feature to trigger cache updates via OCI event streams.
2. **Schema Caching Layer**:
    * Create `stormchaser-model/src/schema_cache.rs` defining the cache structure and background sync trait.
3. **CloudEvents Wrapper Utility**:
    * Create a helper module in `stormchaser-engine/src/nats.rs` (or similar shared location) that takes an event payload, infers or accepts the version, wraps it in a CloudEvent, and attaches the appropriate NATS headers (`Nats-Msg-Schema-Version`, `Schema-ID`).
4. **Publishing Refactor**:
    * Find all `js.publish` calls across `stormchaser-engine`, `stormchaser-api`, and runners.
    * Update subjects to the `v1` format (e.g., `stormchaser.v1.run.*`).
    * Route them through the new CloudEvents helper.
5. **Subscribing Refactor**:
    * Update all `nats.subscribe` and stream configurations to listen to the new `.v1.` subjects (e.g., `stormchaser.v1.run.>` instead of `stormchaser.run.>`).
    * Ensure consumers extract the `data` field from the CloudEvent payload before parsing the legacy models.
6. **Documentation**:
    * Update `docs/nats-schema-improvements.md` (or a dedicated integration doc) to explain Subject Mapping for future version routing.

## Verification

* Run the full test suite (`./scripts/test-unit.sh` and `./scripts/test-integration.sh`).
* Verify that `stormchaser-api` SSE streams correctly parse the unwrapped CloudEvent payload and send it to the UI.
* Verify the OCI caching task starts and functions without crashing.

## Migration & Rollback

* If we were in a production environment with live legacy runners, we would utilize NATS Subject Mapping to map `stormchaser.v1.run.>` to `stormchaser.run.>` during the rollout. Since this is an atomic code change, we will update all publishers and subscribers simultaneously.
