//! Core data models and domain types for the Stormchaser workflow engine.
//!
//! This crate provides the foundational types representing workflows, steps,
//! executions, and other entities that make up the domain model.

/// Authentication and authorization models.
pub mod auth;
/// Cron scheduling definitions.
pub mod cron;
/// Domain-Specific Language (DSL) abstract syntax tree and types.
pub mod dsl;
/// System event and correlation models.
pub mod event;
/// Rule-based trigger definitions for events.
pub mod event_rules;
/// Event schemas for CloudEvents.
pub mod events;
/// HCL expression evaluation models.
pub mod hcl_eval;
/// Logging configuration and log streaming types.
pub mod logging;
/// NATS helpers.
pub mod nats;
/// Outbox pattern models for reliable message publishing.
pub mod outbox;
/// Execution runner definitions and status models.
pub mod runner;
/// Schema cache for OCI schemas.
pub mod schema_cache;
/// Schema generation functions.
pub mod schema_gen;
/// Workflow step execution models and state.
pub mod step;
/// Storage backend and artifact registry models.
pub mod storage;
/// Test reporting and summary models.
pub mod test_report;
/// Core workflow run and state management types.
pub mod workflow;

pub use auth::{ApiOpaContext, Claims, EngineOpaContext, OpaClient};
pub use dsl::{ApprovalSpec, WaitEventSpec};
pub use event::{ApprovalRegistry, EventCorrelation};
pub use event_rules::{EventRule, WebhookConfig};
pub use logging::LogBackend;
pub use outbox::OutboxMessage;
pub use runner::{Runner, RunnerStatus, StepDefinition};
pub use step::{StepInstance, StepOutput, StepStatus};
pub use storage::{BackendType, StorageBackend};
pub use test_report::{TestCase, TestCaseStatus, TestReport, TestSummary};
pub use workflow::{AuditLog, RunContext, RunQuotas, RunStatus, WorkflowRun};

/// Engine trait for scheduling and managing workflow runs.
pub trait WorkflowEngine {
    /// Schedules a new workflow run by name.
    fn schedule_run(&self, name: &str) -> anyhow::Result<WorkflowRun>;
}
