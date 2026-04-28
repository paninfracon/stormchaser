use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    #[serde(default)]
    pub is_template: bool,
    pub dsl_version: String,
    pub name: String,
    pub description: Option<String>,
    pub cron: Option<String>,
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub step_libraries: Vec<StepLibrary>,
    #[serde(default)]
    pub includes: Vec<Include>,
    pub strategy: Option<Strategy>,
    pub quotas: Option<Quotas>,
    pub storage: Vec<Storage>,
    pub inputs: Vec<Input>,
    pub outputs: Vec<Output>,
    pub handlers: Vec<Handler>,
    pub steps: Vec<Step>,
    pub on_failure: Option<Vec<Step>>,
    pub finally: Option<Vec<Step>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Library {
    pub name: String,
    pub source: String,
    pub version: String,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepLibrary {
    pub name: String,
    pub r#type: String,
    #[serde(default)]
    pub params: HashMap<String, String>,
    #[serde(default)]
    pub spec: Value,
    pub timeout: Option<String>,
    pub allow_failure: Option<bool>,
    pub retry: Option<RetryPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Include {
    pub name: String,
    pub workflow: String,
    #[serde(default)]
    pub inputs: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Strategy {
    pub affinity: Option<String>,
    pub fail_fast: Option<bool>,
    pub max_parallel: Option<u32>,
    pub process_allow_list: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quotas {
    pub max_concurrency: Option<u32>,
    pub max_cpu: Option<String>,
    pub max_memory: Option<String>,
    pub max_storage: Option<String>,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storage {
    pub name: String,
    pub backend: Option<String>,
    pub size: String,
    pub provision: Vec<Provision>,
    pub preserve: Vec<String>,
    pub artifacts: Vec<Artifact>,
    pub retainment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provision {
    pub resource_type: String, // "secret", "config", "download", "artifact"
    pub name: String,
    pub source: Option<String>,
    pub url: Option<String>,
    pub destination: String,
    pub mode: Option<String>,
    pub checksum: Option<String>,
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: String,
    pub path: String,
    pub retention: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Input {
    pub name: String,
    pub r#type: String,
    pub description: Option<String>,
    pub default: Option<Value>,
    pub validation: Option<String>,
    pub options: Option<Vec<String>>,
    pub query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub name: String,
    pub value: String, // CEL Expression
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Handler {
    pub name: String,
    pub event_type: String,
    pub condition: Option<String>,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub name: String,
    pub r#type: String, // e.g., "GitCheckout", "RunContainer", "Parallel", etc.
    pub condition: Option<String>,
    pub params: HashMap<String, String>, // CEL Expressions

    /// Step-specific specification.
    /// This allows each step type to define its own structured parameters.
    /// For "RunContainer" on K8s, this would be a K8sJobSpec.
    #[serde(default)]
    pub spec: Value,

    pub strategy: Option<Strategy>,
    pub aggregation: Vec<Aggregation>,
    pub iterate: Option<String>,
    pub iterate_as: Option<String>,
    pub steps: Option<Vec<Step>>, // For Parallel/Sequential
    pub next: Vec<String>,
    pub on_failure: Option<Box<Step>>,
    pub retry: Option<RetryPolicy>,
    pub timeout: Option<String>,
    pub allow_failure: Option<bool>,
    pub start_marker: Option<String>,
    pub end_marker: Option<String>,
    pub outputs: Vec<OutputExtraction>,
    #[serde(default)]
    pub reports: Vec<TestReport>,
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestReport {
    pub name: String,
    pub path: String,
    pub format: String, // e.g., "junit"
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageMount {
    pub name: String,
    pub mount_path: String,
    pub read_only: Option<bool>,
}

/// Minimal common set of arguments for running containers (portable across runners)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommonContainerSpec {
    pub image: String,
    pub command: Option<Vec<String>>,
    pub args: Option<Vec<String>>,
    pub env: Option<Vec<EnvVar>>,
    pub cpu: Option<String>,
    pub memory: Option<String>,
    pub privileged: Option<bool>,
    pub storage_mounts: Option<Vec<StorageMount>>,
}

/// Structured specification for a Kubernetes Job (RunK8sJob step)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct K8sJobSpec {
    pub image: String,
    pub command: Option<Vec<String>>,
    pub args: Option<Vec<String>>,
    pub env: Option<Vec<EnvVar>>,
    pub resources: Option<Resources>,
    pub secret_mounts: Option<Vec<SecretMount>>,
    pub config_map_mounts: Option<Vec<ConfigMapMount>>,
    pub storage_mounts: Option<Vec<StorageMount>>,

    // Kubernetes Job specific fields
    pub active_deadline_seconds: Option<i64>,
    pub backoff_limit: Option<i32>,
    pub completions: Option<i32>,
    pub parallelism: Option<i32>,
    pub ttl_seconds_after_finished: Option<i32>,
    pub privileged: Option<bool>,
    pub minimum_version: Option<String>,
    pub node_selector: Option<HashMap<String, String>>,
    pub service_account_name: Option<String>,
    pub restart_policy: Option<String>, // "OnFailure" or "Never"
    pub labels: Option<HashMap<String, String>>,
    pub annotations: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Resources {
    pub requests: Option<ResourceRequirements>,
    pub limits: Option<ResourceRequirements>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ResourceRequirements {
    pub cpu: Option<String>,
    pub memory: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecretMount {
    pub name: String,
    pub mount_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigMapMount {
    pub name: String,
    pub mount_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Aggregation {
    pub name: String,
    pub description: Option<String>,
    pub value: String, // CEL Expression
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputExtraction {
    pub name: String,
    pub source: String,
    pub marker: Option<String>,
    pub format: Option<String>,
    pub regex: Option<String>,
    pub group: Option<u32>,
    pub sensitive: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub count: u32,
    pub backoff: String,
    pub max_delay: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalSpec {
    pub approvers: Option<Vec<String>>,
    pub inputs: Option<Vec<Input>>,
    pub notify: Option<EmailSpec>,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WaitEventSpec {
    pub correlation_key: String,
    pub correlation_value: String,
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LambdaInvokeSpec {
    pub function_name: String,
    pub payload: Option<Value>,
    pub invocation_type: Option<String>, // "RequestResponse", "Event", "DryRun"
    pub qualifier: Option<String>,       // Alias or version
    pub region: Option<String>,
    pub assume_role_arn: Option<String>, // IAM Role ARN to assume
    pub role_session_name: Option<String>, // Optional session name for the assumed role
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitCheckoutSpec {
    pub repo: String,
    pub r#ref: Option<String>,
    pub depth: Option<u32>,
    pub sparse_checkout: Option<Vec<String>>,
    pub destination: Option<String>,
    pub storage_mounts: Option<Vec<StorageMount>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WasmStepSpec {
    pub module: String, // URI to WASM module (e.g., file://, s3://, or registry name)
    pub function: String,
    pub args: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebhookInvokeSpec {
    pub url: String,
    pub method: Option<String>, // Default to POST
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>, // MiniJinja template
    pub timeout: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EmailBackend {
    Smtp,
    Ses,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EmailSpec {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub subject: String,
    pub body: String, // MiniJinja template
    pub html: Option<bool>,
    pub backend: Option<EmailBackend>, // Defaults to SMTP
    pub smtp_server: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_use_tls: Option<bool>,
    pub smtp_use_mtls: Option<bool>,
    pub ses_region: Option<String>,
    pub ses_role_arn: Option<String>,
    pub ses_configuration_set_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JinjaRenderSpec {
    pub template: String,
    pub context: Option<Value>,
    pub output_key: Option<String>, // Key to store the result in step outputs, defaults to "result"
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TestReportEmailSpec {
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub template: Option<String>,      // Override default template
    pub report_name: Option<String>,   // If None, send all reports for the run
    pub backend: Option<EmailBackend>, // Defaults to SMTP
    pub smtp_server: Option<String>,
    pub smtp_port: Option<u16>,
    pub smtp_username: Option<String>,
    pub smtp_password: Option<String>,
    pub smtp_use_tls: Option<bool>,
    pub smtp_use_mtls: Option<bool>,
    pub ses_region: Option<String>,
    pub ses_role_arn: Option<String>,
    pub ses_configuration_set_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JqSpec {
    pub program: String,
    pub input: Option<Value>,
    pub input_file: Option<String>,
    pub output_file: Option<String>,
    pub storage_mounts: Option<Vec<StorageMount>>,
}
