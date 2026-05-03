//! Domain-Specific Language (DSL) abstract syntax tree and types.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Root structure representing a parsed workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Workflow {
    /// Indicates if this workflow is a reusable template.
    #[serde(default)]
    pub is_template: bool,
    /// The version of the Stormchaser DSL used.
    pub dsl_version: String,
    /// The name of the workflow.
    pub name: String,
    /// Optional description of the workflow.
    pub description: Option<String>,
    /// Optional cron schedule for triggering the workflow.
    pub cron: Option<String>,
    /// External reusable workflows or templates imported.
    pub libraries: Vec<Library>,
    /// Custom step definitions imported for this workflow.
    #[serde(default)]
    pub step_libraries: Vec<StepLibrary>,
    /// Sub-workflows included for execution.
    #[serde(default)]
    pub includes: Vec<Include>,
    /// Execution strategy and constraints for the workflow.
    pub strategy: Option<Strategy>,
    /// Resource quotas applied to the entire workflow run.
    pub quotas: Option<Quotas>,
    /// Storage configurations to provision.
    pub storage: Vec<Storage>,
    /// Expected inputs for the workflow.
    pub inputs: Vec<Input>,
    /// Outputs to be produced by the workflow.
    pub outputs: Vec<Output>,
    /// Event handlers for system or custom events.
    pub handlers: Vec<Handler>,
    /// The sequential or parallel steps to execute.
    pub steps: Vec<Step>,
    /// Steps to execute if the main workflow fails.
    pub on_failure: Option<Vec<Step>>,
    /// Steps to execute regardless of success or failure.
    pub finally: Option<Vec<Step>>,
}

/// Import definition for external workflow templates.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Library {
    /// Alias to use when referencing this library.
    pub name: String,
    /// Source URI of the library.
    pub source: String,
    /// Specific version or reference of the library.
    pub version: String,
    /// Checksum to verify the library's integrity.
    pub checksum: String,
}

/// Import definition for external step types.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepLibrary {
    /// Alias to use for the imported step type.
    pub name: String,
    /// The base step type being imported.
    pub r#type: String,
    /// Default parameters applied to the step.
    #[serde(default)]
    pub params: HashMap<String, String>,
    /// Step specification override.
    #[serde(default)]
    pub spec: Value,
    /// Default timeout for the step.
    pub timeout: Option<String>,
    /// Whether failures are ignored by default.
    pub allow_failure: Option<bool>,
    /// Default retry policy.
    pub retry: Option<RetryPolicy>,
}

/// Defines an inclusion of another workflow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Include {
    /// Name of the inclusion block.
    pub name: String,
    /// Reference to the workflow to include.
    pub workflow: String,
    /// Input variables to pass to the included workflow.
    #[serde(default)]
    pub inputs: HashMap<String, String>,
}

/// Execution strategy settings (e.g., parallelism, affinity).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Strategy {
    /// Runner affinity rules.
    pub affinity: Option<String>,
    /// Whether to abort all parallel steps if one fails.
    pub fail_fast: Option<bool>,
    /// Maximum number of parallel executions.
    pub max_parallel: Option<u32>,
    /// List of allowed processes or capabilities.
    pub process_allow_list: Option<Vec<String>>,
}

/// Resource quotas for a run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Quotas {
    /// Maximum concurrent steps.
    pub max_concurrency: Option<u32>,
    /// Maximum CPU limit.
    pub max_cpu: Option<String>,
    /// Maximum memory limit.
    pub max_memory: Option<String>,
    /// Maximum storage limit.
    pub max_storage: Option<String>,
    /// Maximum duration.
    pub timeout: Option<String>,
}

/// Storage volume definition to be provisioned.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Storage {
    /// Logical name of the storage volume.
    pub name: String,
    /// Type of backend (e.g., 's3', 'gcs').
    pub backend: Option<String>,
    /// Size of the storage requested.
    pub size: String,
    /// Pre-provisioning instructions (e.g., download an artifact into it).
    pub provision: Vec<Provision>,
    /// Paths to preserve after execution.
    pub preserve: Vec<String>,
    /// Artifacts to upload.
    pub artifacts: Vec<Artifact>,
    /// Retention policy for the storage.
    pub retainment: Option<String>,
}

/// Instructions for provisioning data into storage.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Provision {
    /// Type of resource to provision.
    pub resource_type: String, // "secret", "config", "download", "artifact"
    /// Name of the provision block.
    pub name: String,
    /// Source reference.
    pub source: Option<String>,
    /// URL to download from.
    pub url: Option<String>,
    /// Destination path.
    pub destination: String,
    /// File mode/permissions.
    pub mode: Option<String>,
    /// Verification checksum.
    pub checksum: Option<String>,
    /// Optional specific origin.
    pub from: Option<String>,
}

/// Definition of an artifact to collect and upload.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Artifact {
    /// Logical name of the artifact.
    pub name: String,
    /// Local path to the artifact file or directory.
    pub path: String,
    /// Retention period.
    pub retention: String,
}

/// Workflow input parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Input {
    /// Name of the input variable.
    pub name: String,
    /// Data type of the input.
    pub r#type: String,
    /// Description of the input.
    pub description: Option<String>,
    /// Default value if not provided.
    pub default: Option<Value>,
    /// Regex or validation rule for the input.
    pub validation: Option<String>,
    /// Allowed options for enum-like inputs.
    pub options: Option<Vec<String>>,
    /// Dynamic query to fetch options.
    pub query: Option<String>,
}

/// Workflow output value definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Output {
    /// Name of the output variable.
    pub name: String,
    /// CEL Expression to evaluate the output value.
    pub value: String, // CEL Expression
}

/// Event handler mapping an event to an action.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Handler {
    /// Name of the handler.
    pub name: String,
    /// Type of event to match.
    pub event_type: String,
    /// Condition expression to filter events.
    pub condition: Option<String>,
    /// Action to execute when the event occurs.
    pub action: String,
}

/// Execution step definition within a workflow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Step {
    /// Name of the step.
    pub name: String,
    /// Type of the step.
    pub r#type: String, // e.g., "GitCheckout", "RunContainer", "Parallel", etc.
    /// CEL condition controlling whether the step runs.
    pub condition: Option<String>,
    /// Evaluated parameters for the step.
    pub params: HashMap<String, String>, // CEL Expressions

    /// Step-specific specification.
    /// This allows each step type to define its own structured parameters.
    /// For "RunContainer" on K8s, this would be a K8sJobSpec.
    #[serde(default)]
    pub spec: Value,

    /// Execution strategy overrides for this step.
    pub strategy: Option<Strategy>,
    /// Values to aggregate from iterated steps.
    pub aggregation: Vec<Aggregation>,
    /// Iteration expression (e.g., a list of items to map over).
    pub iterate: Option<String>,
    /// Variable name for the current iteration item.
    pub iterate_as: Option<String>,
    /// Child steps if this is a group (e.g., Parallel).
    pub steps: Option<Vec<Step>>, // For Parallel/Sequential
    /// Next steps to execute after this one.
    pub next: Vec<String>,
    /// Steps to execute if this step fails.
    pub on_failure: Option<Box<Step>>,
    /// Retry policy for this step.
    pub retry: Option<RetryPolicy>,
    /// Timeout duration.
    pub timeout: Option<String>,
    /// Whether failure of this step should be ignored.
    pub allow_failure: Option<bool>,
    /// Log marker indicating the start of a section.
    pub start_marker: Option<String>,
    /// Log marker indicating the end of a section.
    pub end_marker: Option<String>,
    /// Extraction rules to capture output values.
    pub outputs: Vec<OutputExtraction>,
    /// Test reports to collect.
    #[serde(default)]
    pub reports: Vec<TestReport>,
    /// Specific artifacts to collect for this step.
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
}

/// Definition for capturing a test report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TestReport {
    /// Name of the report.
    pub name: String,
    /// Path to the report file.
    pub path: String,
    /// Format of the report (e.g., 'junit').
    pub format: String, // e.g., "junit"
}

/// Mount definition for attaching storage to a container.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageMount {
    /// Name of the storage volume to mount.
    pub name: String,
    /// Path where the volume should be mounted inside the container.
    pub mount_path: String,
    /// Whether the mount should be read-only.
    pub read_only: Option<bool>,
    /// Step-specific paths to preserve when parking the volume, overriding the top-level storage config.
    #[serde(default)]
    pub preserve: Option<Vec<String>>,
}

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

/// Aggregation rule for map/reduce patterns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Aggregation {
    /// Name of the aggregation.
    pub name: String,
    /// Optional description.
    pub description: Option<String>,
    /// CEL Expression for aggregation.
    pub value: String, // CEL Expression
}

/// Rule for extracting outputs from logs or files.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OutputExtraction {
    /// Output key name.
    pub name: String,
    /// Source to extract from (e.g., 'stdout', 'file').
    pub source: String,
    /// Optional marker string.
    pub marker: Option<String>,
    /// Data format (e.g., 'json', 'regex').
    pub format: Option<String>,
    /// Regular expression to match.
    pub regex: Option<String>,
    /// Regex capture group index.
    pub group: Option<u32>,
    /// Whether the extracted output is sensitive.
    pub sensitive: Option<bool>,
}

/// Step retry policy definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryPolicy {
    /// Number of retry attempts.
    pub count: u32,
    /// Backoff strategy ('fixed', 'exponential').
    pub backoff: String,
    /// Maximum backoff delay.
    pub max_delay: Option<String>,
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
