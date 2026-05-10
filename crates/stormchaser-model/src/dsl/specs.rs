use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::{Input, StorageMount};

/// Minimal common set of arguments for running containers (portable across runners)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommonContainerSpec {
    /// Container image to run.
    pub image: String,
    /// Override the container entrypoint.
    pub command: Option<Vec<String>>,
    /// Arguments passed to the command.
    pub args: Option<Vec<String>>,
    /// Environment variables.
    pub env: Option<Vec<EnvVar>>,
    /// Requested CPU.
    pub cpu: Option<String>,
    /// Requested memory.
    pub memory: Option<String>,
    /// Whether to run in privileged mode.
    pub privileged: Option<bool>,
    /// Storage volumes to mount.
    pub storage_mounts: Option<Vec<StorageMount>>,
}

/// Structured specification for a Kubernetes Job (RunK8sJob step)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct K8sJobSpec {
    /// Container image.
    pub image: String,
    /// Command.
    pub command: Option<Vec<String>>,
    /// Arguments.
    pub args: Option<Vec<String>>,
    /// Environment variables.
    pub env: Option<Vec<EnvVar>>,
    /// Resource requirements.
    pub resources: Option<Resources>,
    /// Secret mounts.
    pub secret_mounts: Option<Vec<SecretMount>>,
    /// Config map mounts.
    pub config_map_mounts: Option<Vec<ConfigMapMount>>,
    /// Storage mounts.
    pub storage_mounts: Option<Vec<StorageMount>>,

    /// Active deadline seconds.
    pub active_deadline_seconds: Option<i64>,
    /// Backoff limit.
    pub backoff_limit: Option<i32>,
    /// Completions.
    pub completions: Option<i32>,
    /// Parallelism.
    pub parallelism: Option<i32>,
    /// TTL seconds after finished.
    pub ttl_seconds_after_finished: Option<i32>,
    /// Privileged flag.
    pub privileged: Option<bool>,
    /// Minimum version of K8s node.
    pub minimum_version: Option<String>,
    /// Node selector labels.
    pub node_selector: Option<HashMap<String, String>>,
    /// Service account name.
    pub service_account_name: Option<String>,
    /// Restart policy.
    pub restart_policy: Option<String>, // "OnFailure" or "Never"
    /// Custom labels.
    pub labels: Option<HashMap<String, String>>,
    /// Custom annotations.
    pub annotations: Option<HashMap<String, String>>,
}

/// Key-value pair for environment variables.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvVar {
    /// Variable name.
    pub name: String,
    /// Variable value.
    pub value: String,
}

/// Resource specification for CPU and memory.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Resources {
    /// Minimum guaranteed resources.
    pub requests: Option<ResourceRequirements>,
    /// Maximum allowed resources.
    pub limits: Option<ResourceRequirements>,
}

/// Resource requirements structure.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResourceRequirements {
    /// CPU specification.
    pub cpu: Option<String>,
    /// Memory specification.
    pub memory: Option<String>,
}

/// Secret mount definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecretMount {
    /// Name of the secret.
    pub name: String,
    /// Mount path.
    pub mount_path: String,
}

/// ConfigMap mount definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigMapMount {
    /// Name of the ConfigMap.
    pub name: String,
    /// Mount path.
    pub mount_path: String,
}

/// Human-in-the-loop approval specification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalSpec {
    /// List of user IDs or groups allowed to approve.
    pub approvers: Option<Vec<String>>,
    /// Input fields required during approval.
    pub inputs: Option<Vec<Input>>,
    /// Notification settings.
    pub notify: Option<EmailSpec>,
    /// Timeout for waiting for approval.
    pub timeout: Option<String>,
}

/// Specification for waiting on a system event.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WaitEventSpec {
    /// Key used for correlation.
    pub correlation_key: String,
    /// Expected value for correlation.
    pub correlation_value: String,
    /// Timeout.
    pub timeout: Option<String>,
}

/// Specification for invoking an AWS Lambda function.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LambdaInvokeSpec {
    /// Name of the Lambda function.
    pub function_name: String,
    /// JSON payload.
    pub payload: Option<Value>,
    /// Invocation type.
    pub invocation_type: Option<String>, // "RequestResponse", "Event", "DryRun"
    /// Qualifier (alias/version).
    pub qualifier: Option<String>, // Alias or version
    /// AWS Region.
    pub region: Option<String>,
    /// IAM Role ARN to assume.
    pub assume_role_arn: Option<String>, // IAM Role ARN to assume
    /// Optional session name.
    pub role_session_name: Option<String>, // Optional session name for the assumed role
}

/// Specification for checking out a Git repository.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitCheckoutSpec {
    /// Repository URL.
    pub repo: String,
    /// Git reference (branch, tag, commit).
    pub r#ref: Option<String>,
    /// Clone depth.
    pub depth: Option<u32>,
    /// Sparse checkout paths.
    pub sparse_checkout: Option<Vec<String>>,
    /// Destination directory.
    pub destination: Option<String>,
    /// Storage mounts.
    pub storage_mounts: Option<Vec<StorageMount>>,
}

/// Specification for executing a WASM module.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WasmStepSpec {
    /// URI to WASM module.
    pub module: String, // URI to WASM module (e.g., file://, s3://, or registry name)
    /// Function to invoke.
    pub function: String,
    /// Arguments to pass.
    pub args: Option<Value>,
}

/// Specification for invoking a Webhook.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebhookInvokeSpec {
    /// Webhook URL.
    pub url: String,
    /// HTTP method.
    pub method: Option<String>, // Default to POST
    /// HTTP headers.
    pub headers: Option<HashMap<String, String>>,
    /// Request body template.
    pub body: Option<String>, // MiniJinja template
    /// Request timeout.
    pub timeout: Option<String>,
}

/// Specification for making a REST API call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestApiResponseExtractor {
    /// Output key name.
    pub name: String,
    /// Data format (e.g., 'json', 'regex').
    pub format: Option<String>,
    /// JSON Pointer (or dot-notation path) used for JSON response extraction.
    pub json_pointer: Option<String>,
    /// Regular expression to match against the response body.
    pub regex: Option<String>,
    /// Regex capture group index.
    pub group: Option<u32>,
    /// Whether the extracted output is sensitive.
    pub sensitive: Option<bool>,
}

/// Specification for making a REST API call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RestApiSpec {
    /// REST API URL.
    pub url: String,
    /// HTTP method.
    pub method: Option<String>, // Default to GET
    /// HTTP headers.
    pub headers: Option<HashMap<String, String>>,
    /// Request body template.
    pub body: Option<String>, // MiniJinja template
    /// Request timeout.
    pub timeout: Option<String>,
    /// Rules for extracting outputs from the response.
    pub extractors: Option<Vec<RestApiResponseExtractor>>,
}

/// Supported email backends.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailBackend {
    /// Standard SMTP protocol.
    Smtp,
    /// Amazon Simple Email Service.
    Ses,
}

/// Specification for sending an email notification.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EmailSpec {
    /// Sender address.
    pub from: String,
    /// List of recipient addresses.
    pub to: Vec<String>,
    /// CC addresses.
    pub cc: Option<Vec<String>>,
    /// BCC addresses.
    pub bcc: Option<Vec<String>>,
    /// Email subject.
    pub subject: String,
    /// Body template (MiniJinja).
    pub body: String, // MiniJinja template
    /// Send as HTML.
    pub html: Option<bool>,
    /// Email backend choice.
    pub backend: Option<EmailBackend>, // Defaults to SMTP
    /// SMTP Server address.
    pub smtp_server: Option<String>,
    /// SMTP Port.
    pub smtp_port: Option<u16>,
    /// SMTP username.
    pub smtp_username: Option<String>,
    /// SMTP password.
    pub smtp_password: Option<String>,
    /// Use TLS.
    pub smtp_use_tls: Option<bool>,
    /// Use mTLS.
    pub smtp_use_mtls: Option<bool>,
    /// SES region.
    pub ses_region: Option<String>,
    /// SES role ARN to assume.
    pub ses_role_arn: Option<String>,
    /// SES configuration set.
    pub ses_configuration_set_name: Option<String>,
}

/// Specification for rendering a Jinja template.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JinjaRenderSpec {
    /// The template string.
    pub template: String,
    /// Context variables for rendering.
    pub context: Option<Value>,
    /// Output key for the result.
    pub output_key: Option<String>, // Key to store the result in step outputs, defaults to "result"
}

/// Specification for emailing a test report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TestReportEmailSpec {
    /// Sender address.
    pub from: String,
    /// Recipient addresses.
    pub to: Vec<String>,
    /// Email subject.
    pub subject: String,
    /// Custom template to override default.
    pub template: Option<String>, // Override default template
    /// Specific report name to send, or all if None.
    pub report_name: Option<String>, // If None, send all reports for the run
    /// Backend to use.
    pub backend: Option<EmailBackend>, // Defaults to SMTP
    /// SMTP Server.
    pub smtp_server: Option<String>,
    /// SMTP Port.
    pub smtp_port: Option<u16>,
    /// SMTP username.
    pub smtp_username: Option<String>,
    /// SMTP password.
    pub smtp_password: Option<String>,
    /// Use TLS.
    pub smtp_use_tls: Option<bool>,
    /// Use mTLS.
    pub smtp_use_mtls: Option<bool>,
    /// SES region.
    pub ses_region: Option<String>,
    /// SES role ARN.
    pub ses_role_arn: Option<String>,
    /// SES config set.
    pub ses_configuration_set_name: Option<String>,
}

/// Specification for running a JQ query on data.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JqSpec {
    /// JQ program string.
    pub program: String,
    /// JSON input data.
    pub input: Option<Value>,
    /// Input file path.
    pub input_file: Option<String>,
    /// Output file path.
    pub output_file: Option<String>,
    /// Storage mounts.
    pub storage_mounts: Option<Vec<StorageMount>>,
}
