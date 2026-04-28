pub mod auth;
pub mod cron;
pub mod step;
pub mod storage;
pub mod webhook;
pub mod workflow;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use stormchaser_model::workflow::RunStatus;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use stormchaser_model::step::{StepInstance, StepOutput, StepStatusHistory};
use stormchaser_model::storage::{ArtifactRegistry, BackendType};
use stormchaser_model::test_report;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthExchangeRequest {
    pub sso_token: String,
    pub callback_url: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthExchangeResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub expires_in: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthRefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EnqueueRequest {
    pub workflow_name: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    #[schema(value_type = Object)]
    pub inputs: serde_json::Value,
    pub overrides: Option<RunOverrides>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RunOverrides {
    pub timeout: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct EnqueueResponse {
    pub run_id: Uuid,
    pub status: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListRunsQuery {
    pub workflow_name: Option<String>,
    #[schema(value_type = Option<String>)]
    #[param(value_type = Option<String>)]
    pub status: Option<RunStatus>,
    pub initiating_user: Option<String>,
    pub repo_url: Option<String>,
    pub workflow_path: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct WorkflowRunDetail {
    pub id: Uuid,
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
    pub inputs: serde_json::Value,
    pub secrets: serde_json::Value,
    pub source_code: String,
    pub dsl_version: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkflowRunFullDetail {
    pub detail: WorkflowRunDetail,
    pub steps: Vec<StepDetail>,
    #[schema(value_type = Vec<Object>)]
    pub artifacts: Vec<ArtifactRegistry>,
    pub test_summaries: Vec<TestSummaryResponse>,
    #[schema(value_type = Vec<Object>)]
    pub test_cases: Vec<test_report::TestCase>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct StepDetail {
    pub instance: StepInstance,
    pub outputs: Vec<StepOutput>,
    pub history: Vec<StepStatusHistory>,
    pub logs: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct DirectRunRequest {
    pub dsl: String,
    pub inputs: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub description: Option<String>,
    pub source_type: String, // e.g. "github", "generic"
    pub secret_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateEventRuleRequest {
    pub name: String,
    pub description: Option<String>,
    pub webhook_id: Uuid,
    pub event_type_pattern: String,
    pub condition_expr: Option<String>,
    pub workflow_name: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub input_mappings: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCronWorkflowRequest {
    pub name: String,
    pub description: Option<String>,
    pub cronspec: String,
    pub workflow_name: String,
    pub repo_url: String,
    pub workflow_path: String,
    pub git_ref: String,
    pub inputs: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct CronWorkflowResponse {
    pub id: Uuid,
    pub secret_token: String,
    pub external_job_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateStorageBackendRequest {
    pub name: String,
    pub description: Option<String>,
    pub backend_type: BackendType,
    pub config: serde_json::Value,
    pub is_default_sfs: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateStorageBackendRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub backend_type: Option<BackendType>,
    pub config: Option<serde_json::Value>,
    pub is_default_sfs: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct TestReportSummary {
    pub id: Uuid,
    pub report_name: String,
    pub file_name: String,
    pub format: String,
    pub checksum: String,
    pub backend_id: Option<Uuid>,
    pub remote_path: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct TestSummaryResponse {
    pub id: Uuid,
    pub run_id: Uuid,
    pub step_instance_id: Uuid,
    pub report_name: String,
    pub total_tests: i32,
    pub passed: i32,
    pub failed: i32,
    pub skipped: i32,
    pub errors: i32,
    pub duration_ms: i64,
    pub created_at: DateTime<Utc>,
}
