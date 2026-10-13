use serde_json::Value;
use std::collections::HashMap;
use stormchaser_model::workflow::RunStatus;
/// Module for auth.
pub mod auth;
/// Module for storage.
pub mod connections;
/// Module for cron.
pub mod cron;
/// Module for event rule.
pub mod event_rule;
/// Module for MCP (Model Context Protocol).
#[cfg(feature = "mcp")]
pub mod mcp;
/// Module for schema.
pub mod schema;
/// Module for step.
pub mod step;
/// Module for webhook.
pub mod webhook;
/// Module for workflow.
pub mod workflow;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use stormchaser_model::connections::{ArtifactRegistry, ConnectionType};
use stormchaser_model::step::{StepInstance, StepOutput, StepStatusHistory};
use stormchaser_model::test_report;

#[derive(Debug, Deserialize, ToSchema)]
/// Authexchangerequest.
pub struct AuthExchangeRequest {
    /// The sso token.
    pub sso_token: String,
    /// The callback url.
    pub callback_url: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
/// Authexchangeresponse.
pub struct AuthExchangeResponse {
    /// The access token.
    pub access_token: String,
    /// The refresh token.
    pub refresh_token: Option<String>,
    /// The token type.
    pub token_type: String,
    /// The expires in.
    pub expires_in: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Authrefreshrequest.
pub struct AuthRefreshRequest {
    /// The refresh token.
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ParseGitRequest {
    /// The connection (ID or name) to the repository.
    pub connection: String,
    /// The workflow path.
    pub workflow_path: String,
    /// The git ref.
    pub git_ref: String,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Enqueuerequest.
pub struct EnqueueRequest {
    /// The workflow name.
    pub workflow_name: String,
    /// The connection (ID or name) to the repository.
    pub connection: String,
    /// The workflow path.
    pub workflow_path: String,
    /// The git ref.
    pub git_ref: String,
    /// The inputs.
    #[schema(value_type = Object)]
    pub inputs: Value,
    /// The overrides.
    pub overrides: Option<RunOverrides>,
    /// Optional custom SOPS file name.
    pub sops_file: Option<String>,
    /// Optional AWS IAM Role ARN to assume for SOPS decryption.
    pub sops_role_arn: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Runoverrides.
pub struct RunOverrides {
    /// The timeout.
    pub timeout: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
/// Enqueueresponse.
pub struct EnqueueResponse {
    /// The run id.
    pub run_id: stormchaser_model::RunId,
    /// The status.
    pub status: RunStatus,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams, Default)]
#[into_params(parameter_in = Query)]
/// Listrunsquery.
pub struct ListRunsQuery {
    /// The workflow name.
    pub workflow_name: Option<String>,
    #[schema(value_type = Option<String>)]
    #[param(value_type = Option<String>)]
    /// The status.
    pub status: Option<RunStatus>,
    /// The initiating user.
    pub initiating_user: Option<String>,
    /// The repo url.
    pub repo_url: Option<String>,
    /// The workflow path.
    pub workflow_path: Option<String>,
    /// The created after.
    pub created_after: Option<DateTime<Utc>>,
    /// The created before.
    pub created_before: Option<DateTime<Utc>>,
    /// The limit.
    pub limit: Option<usize>,
    /// The offset.
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
/// Workflowrundetail.
pub struct WorkflowRunDetail {
    /// The id.
    pub id: stormchaser_model::RunId,
    /// The workflow name.
    pub workflow_name: String,
    /// The initiating user.
    pub initiating_user: String,
    /// The repo url.
    pub repo_url: String,
    /// The workflow path.
    pub workflow_path: String,
    /// The git ref.
    pub git_ref: String,
    /// The status.
    pub status: RunStatus,
    /// The version.
    pub version: i32,
    /// The created at.
    pub created_at: DateTime<Utc>,
    /// The updated at.
    pub updated_at: DateTime<Utc>,
    /// The started resolving at.
    pub started_resolving_at: Option<DateTime<Utc>>,
    /// The started at.
    pub started_at: Option<DateTime<Utc>>,
    /// The finished at.
    pub finished_at: Option<DateTime<Utc>>,
    /// The error.
    pub error: Option<String>,
    /// The inputs. Values of inputs the workflow declares `secret = true` are masked.
    #[schema(value_type = Object)]
    pub inputs: Value,
    /// The workflow's input schema, used only to find which inputs to mask.
    #[serde(skip)]
    #[sqlx(default)]
    #[schema(ignore)]
    pub inputs_schema: Option<Value>,
    /// The source code.
    pub source_code: String,
    /// The dsl version.
    pub dsl_version: String,
}

/// Shown in place of a secret input's value.
pub const MASKED_INPUT: &str = "********";

impl WorkflowRunDetail {
    /// Masks the value of every input the workflow's schema marks `secret = true`.
    ///
    /// Run details are readable by any authenticated user, so secret inputs (passwords,
    /// tokens) must not be echoed back in them.
    pub fn mask_secret_inputs(mut self) -> Self {
        let secret_names: Vec<String> = self
            .inputs_schema
            .as_ref()
            .and_then(|schema| schema.get("properties"))
            .and_then(|props| props.as_object())
            .map(|props| {
                props
                    .iter()
                    .filter(|(_, prop)| prop.get("secret").and_then(Value::as_bool) == Some(true))
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(inputs) = self.inputs.as_object_mut() {
            for name in secret_names {
                if let Some(value) = inputs.get_mut(&name) {
                    *value = Value::String(MASKED_INPUT.to_string());
                }
            }
        }
        self
    }
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
/// Workflowrunfulldetail.
pub struct WorkflowRunFullDetail {
    /// The detail.
    pub detail: WorkflowRunDetail,
    /// The steps.
    pub steps: Vec<StepDetail>,
    #[schema(value_type = Vec<Object>)]
    /// The artifacts.
    pub artifacts: Vec<ArtifactRegistry>,
    /// The test summaries.
    pub test_summaries: Vec<TestSummaryResponse>,
    #[schema(value_type = Vec<Object>)]
    /// The test cases.
    pub test_cases: Vec<test_report::TestCase>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
/// Stepdetail.
pub struct StepDetail {
    /// The instance.
    pub instance: StepInstance,
    /// The outputs.
    pub outputs: Vec<StepOutput>,
    /// The history.
    pub history: Vec<StepStatusHistory>,
    /// The logs.
    pub logs: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Directrunrequest.
pub struct DirectRunRequest {
    /// The dsl.
    pub dsl: String,
    /// The inputs.
    #[schema(value_type = Object)]
    pub inputs: Value,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Createwebhookrequest.
pub struct CreateWebhookRequest {
    /// The name.
    pub name: String,
    /// The description.
    pub description: Option<String>,
    /// The source type.
    pub source_type: String, // e.g. "github", "generic"
    /// The secret token.
    pub secret_token: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Updatewebhookrequest.
pub struct UpdateWebhookRequest {
    /// The name.
    pub name: Option<String>,
    /// The description.
    pub description: Option<String>,
    /// The source type.
    pub source_type: Option<String>, // e.g. "github", "generic"
    /// The secret token.
    pub secret_token: Option<String>,
    /// Is active.
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Createeventrulerequest.
pub struct CreateEventRuleRequest {
    /// The name.
    pub name: String,
    /// The description.
    pub description: Option<String>,
    /// The webhook id.
    pub webhook_id: stormchaser_model::WebhookId,
    /// The event type pattern.
    pub event_type_pattern: String,
    /// The condition expr.
    pub condition_expr: Option<String>,
    /// The workflow name.
    pub workflow_name: String,
    /// The connection UUID.
    pub connection: String,
    /// The workflow path.
    pub workflow_path: String,
    /// The git ref.
    pub git_ref: String,
    /// The input mappings.
    pub input_mappings: HashMap<String, String>,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Createcronworkflowrequest.
pub struct CreateCronWorkflowRequest {
    /// The name.
    pub name: String,
    /// The description.
    pub description: Option<String>,
    /// The cronspec.
    pub cronspec: String,
    /// The workflow name.
    pub workflow_name: String,
    /// The connection UUID.
    pub connection: String,
    /// The workflow path.
    pub workflow_path: String,
    /// The git ref.
    pub git_ref: String,
    /// The inputs.
    #[schema(value_type = Object)]
    pub inputs: Value,
}

#[derive(Debug, Serialize, ToSchema)]
/// Cronworkflowresponse.
pub struct CronWorkflowResponse {
    /// The id.
    pub id: stormchaser_model::CronWorkflowId,
    /// The secret token.
    pub secret_token: String,
    /// The external job id.
    pub external_job_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Test connection request.
pub struct TestConnectionRequest {
    /// The backend type.
    pub connection_type: ConnectionType,
    /// The config.
    #[schema(value_type = Object)]
    pub config: Value,
    /// Optional AWS role ARN.
    pub aws_assume_role_arn: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
/// Test connection response.
pub struct TestConnectionResponse {
    /// Whether the connection was successful.
    pub success: bool,
    /// Message detailing the result or error.
    pub message: String,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Createstoragebackendrequest.
pub struct CreateStorageBackendRequest {
    /// The name.
    pub name: String,
    /// The description.
    pub description: Option<String>,
    /// The backend type.
    pub connection_type: ConnectionType,
    /// The config.
    #[schema(value_type = Object)]
    pub config: Value,
    /// Optional AWS role ARN.
    pub aws_assume_role_arn: Option<String>,
    /// Optional encrypted credentials (password/token).
    pub encrypted_credentials: Option<String>,
    /// The is default sfs.
    pub is_default_sfs: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
/// Updatestoragebackendrequest.
pub struct UpdateStorageBackendRequest {
    /// The name.
    pub name: Option<String>,
    /// The description.
    pub description: Option<String>,
    /// The backend type.
    pub connection_type: Option<ConnectionType>,
    /// The config.
    #[schema(value_type = Object)]
    pub config: Option<Value>,
    /// Optional AWS role ARN. Set to an empty string to clear an existing ARN.
    pub aws_assume_role_arn: Option<String>,
    /// Optional encrypted credentials (password/token). Set to an empty string to clear.
    pub encrypted_credentials: Option<String>,
    /// The is default sfs.
    pub is_default_sfs: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
/// Testreportsummary.
pub struct TestReportSummary {
    /// The id.
    pub id: stormchaser_model::TestReportId,
    /// The report name.
    pub report_name: String,
    /// The file name.
    pub file_name: String,
    /// The format.
    pub format: String,
    /// The checksum.
    pub checksum: String,
    /// The backend id.
    pub connection_id: Option<stormchaser_model::ConnectionId>,
    /// The remote path.
    pub remote_path: Option<String>,
    /// The created at.
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
/// Testsummaryresponse.
pub struct TestSummaryResponse {
    /// The id.
    pub id: stormchaser_model::TestReportId,
    /// The run id.
    pub run_id: stormchaser_model::RunId,
    /// The step instance id.
    pub step_instance_id: stormchaser_model::StepInstanceId,
    /// The report name.
    pub report_name: String,
    /// The total tests.
    pub total_tests: i32,
    /// The passed.
    pub passed: i32,
    /// The failed.
    pub failed: i32,
    /// The skipped.
    pub skipped: i32,
    /// The errors.
    pub errors: i32,
    /// The duration ms.
    pub duration_ms: i64,
    /// The created at.
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod run_detail_tests {
    use super::*;
    use serde_json::json;

    fn detail(inputs: Value, inputs_schema: Option<Value>) -> WorkflowRunDetail {
        WorkflowRunDetail {
            id: stormchaser_model::RunId::new_v4(),
            workflow_name: "wf".into(),
            initiating_user: "user".into(),
            repo_url: String::new(),
            workflow_path: String::new(),
            git_ref: String::new(),
            status: RunStatus::Running,
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_resolving_at: None,
            started_at: None,
            finished_at: None,
            error: None,
            inputs,
            inputs_schema,
            source_code: String::new(),
            dsl_version: "1.0".into(),
        }
    }

    #[test]
    fn secret_inputs_are_masked_and_secrets_are_not_serialized() {
        let run = detail(
            json!({ "env": "prod", "db_password": "hunter2" }),
            Some(json!({ "properties": {
                "env": { "type": "string" },
                "db_password": { "type": "string", "secret": true }
            }})),
        )
        .mask_secret_inputs();

        let body = serde_json::to_value(&run).unwrap();
        assert_eq!(body["inputs"]["env"], "prod");
        assert_eq!(body["inputs"]["db_password"], MASKED_INPUT);
        assert!(body.get("secrets").is_none());
        assert!(body.get("inputs_schema").is_none());
        assert!(!body.to_string().contains("hunter2"));
    }

    #[test]
    fn inputs_without_a_schema_are_left_alone() {
        let run = detail(json!({ "env": "prod" }), None).mask_secret_inputs();
        assert_eq!(run.inputs, json!({ "env": "prod" }));
    }
}
