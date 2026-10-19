//! Code shared by the Stormchaser services: the API, the engine and the query service.
//!
//! Nothing here depends on one service's internals, so that no service has to depend
//! on another.

/// Who may do what to the API's objects: owners and policy questions.
pub mod access;
/// Bearer token checks and the policy middleware.
pub mod auth;
/// Policy checks on connection use.
pub mod authz;
/// Operator allow-lists for AWS roles and Lambda functions.
pub mod aws_guard;
/// Settings of the HTTP services (API and query service).
pub mod config;
/// Database access shared by the services.
pub mod db;
/// Git caching and cloning utilities.
pub mod git_cache;
/// Reading a workflow file from a Git connection's repository.
pub mod git_source;
/// Human-In-The-Loop (HITL) approval links.
pub mod hitl;
/// Runs workflow-supplied jq programs in a child process that can be stopped.
pub mod jq_sandbox;
/// The HTTPS or plain HTTP listener of the HTTP services.
pub mod listener;
/// Queries that fill in a workflow's input options (SQL, REST, AWS Cloud Control, mock).
pub mod query;
/// Ceilings on the quotas a run may give itself.
pub mod run_limits;
/// What the shared HTTP code needs from a service's state.
pub mod service;
/// Starting an HTTP service: TLS, database, NATS, policy, log backend, identity provider.
pub mod setup;
