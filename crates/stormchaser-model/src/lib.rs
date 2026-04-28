pub mod auth;
pub mod cron;
pub mod dsl;
pub mod event;
pub mod event_rules;
pub mod hcl_eval;
pub mod logging;
pub mod outbox;
pub mod runner;
pub mod step;
pub mod storage;
pub mod test_report;
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

pub trait WorkflowEngine {
    fn schedule_run(&self, name: &str) -> anyhow::Result<WorkflowRun>;
}
