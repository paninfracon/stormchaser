use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::{Handler, Step, Storage};

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
    /// Aliases defined for the workflow.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Storage configurations to provision.
    pub storage: Vec<Storage>,
    /// Expected inputs for the workflow.
    pub inputs: Vec<Input>,
    /// Dynamic queries.
    #[serde(default)]
    pub queries: Vec<Query>,
    /// Compiled JSON schema for inputs.
    #[serde(default)]
    pub inputs_schema: Option<serde_json::Value>,
    /// View configuration for inputs.
    #[serde(default)]
    pub inputs_view: Option<InputView>,
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

use super::RetryPolicy;

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
}

/// Dynamic query definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct Query {
    /// Name of the query variable.
    pub name: String,
    /// Type of the query.
    pub r#type: String,
    /// Parameters for the query.
    #[serde(default)]
    pub params: HashMap<String, String>,
}

/// Workflow output value definition.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Output {
    /// Name of the output variable.
    pub name: String,
    /// HCL Expression to evaluate the output value.
    pub value: String, // HCL Expression
}

/// Describes the visual layout and presentation of inputs.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct InputView {
    /// Explicit ordering of input fields. Fields not listed are appended.
    #[serde(default)]
    pub ui_order: Vec<String>,
}
