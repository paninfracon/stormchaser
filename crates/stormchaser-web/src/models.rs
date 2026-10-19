use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use stormchaser_model::api;

pub use stormchaser_model::connections::ArtifactRegistry;
pub use stormchaser_model::cron::CronWorkflow;
pub use stormchaser_model::step::{StepInstance, StepOutput, StepStatusHistory};
pub use stormchaser_model::test_report::{TestCase, TestCaseStatus};
pub use stormchaser_model::{
    Connection, ConnectionType, EventRule, RunId, RunStatus, StepInstanceId, StepStatus,
    WebhookConfig,
};

// What the browser is sent of a run: the API's run types without the workflow's
// source code and the fields only the API uses, so lists stay small.

/// A summary of a workflow run, used for list views.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WorkflowRunDetail {
    pub id: RunId,
    pub workflow_name: String,
    pub initiating_user: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub status: RunStatus,
    pub version: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub started_resolving_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub inputs: Value,
}

impl From<api::WorkflowRunDetail> for WorkflowRunDetail {
    fn from(run: api::WorkflowRunDetail) -> Self {
        WorkflowRunDetail {
            id: run.id,
            workflow_name: run.workflow_name,
            initiating_user: run.initiating_user,
            repo_url: run.repo_url,
            workflow_path: run.workflow_path,
            git_ref: run.git_ref,
            status: run.status,
            version: run.version,
            created_at: run.created_at,
            updated_at: run.updated_at,
            started_resolving_at: run.started_resolving_at,
            started_at: run.started_at,
            finished_at: run.finished_at,
            error: run.error,
            inputs: run.inputs,
        }
    }
}

/// A step of a workflow run, with its outputs, history and logs.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StepDetail {
    pub instance: StepInstance,
    pub outputs: Vec<StepOutput>,
    pub history: Vec<StepStatusHistory>,
    pub logs: Vec<String>,
}

impl From<api::StepDetail> for StepDetail {
    fn from(step: api::StepDetail) -> Self {
        StepDetail {
            instance: step.instance,
            outputs: step.outputs,
            history: step.history,
            logs: step.logs,
        }
    }
}

/// A workflow run with its steps, artifacts and test cases.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkflowRunFullDetail {
    pub detail: WorkflowRunDetail,
    pub steps: Vec<StepDetail>,
    pub artifacts: Vec<ArtifactRegistry>,
    pub test_cases: Vec<TestCase>,
}

impl From<api::WorkflowRunFullDetail> for WorkflowRunFullDetail {
    fn from(run: api::WorkflowRunFullDetail) -> Self {
        WorkflowRunFullDetail {
            detail: run.detail.into(),
            steps: run.steps.into_iter().map(Into::into).collect(),
            artifacts: run.artifacts,
            test_cases: run.test_cases,
        }
    }
}

/// A step status as the API writes it, such as `waiting_for_event`.
pub fn step_status_name(status: &StepStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// What a connection type is called in the UI.
pub fn connection_type_label(connection_type: &ConnectionType) -> &'static str {
    match connection_type {
        ConnectionType::S3 => "S3",
        ConnectionType::Oci => "OCI Registry",
        ConnectionType::Jfrog => "JFrog",
        ConnectionType::Gcs => "GCS",
        ConnectionType::Azure => "Azure Blob",
        ConnectionType::Postgres => "PostgreSQL",
        ConnectionType::Mysql => "MySQL",
        ConnectionType::HttpApi => "HTTP API",
        ConnectionType::Git => "Git",
    }
}

/// Where the workflow behind a run form comes from. Hydration sends this to the query
/// service, which reads the workflow's schema and queries from there itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HydrationSource {
    /// A workflow file in a Git connection's repository.
    Git {
        connection: String,
        workflow_path: String,
        git_ref: String,
    },
    /// Workflow text the user supplied for a direct run.
    Direct { dsl: String },
}
