use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Hydrate the input form of a workflow held in a Git connection's repository.
///
/// The request names the workflow; it does not carry its schema or queries. Those are
/// read from the repository, so a caller cannot have the service run queries of their
/// own choosing by putting them in the request.
#[derive(Deserialize)]
pub struct HydrateSchemaRequest {
    /// The Git connection (id or name).
    pub connection: String,
    pub workflow_path: String,
    pub git_ref: String,
    #[serde(default)]
    pub inputs: Value,
}

/// Hydrate the input form of a workflow supplied as text, for a direct run.
///
/// Whoever sends this is the workflow's author, queries included. It has its own
/// endpoint so that policy can refuse it to users who may run workflows but not write
/// them, as it can refuse direct runs.
#[derive(Deserialize)]
pub struct HydrateDirectRequest {
    pub dsl: String,
    #[serde(default)]
    pub inputs: Value,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub enum HydrationStatus {
    #[serde(rename = "Update pending")]
    UpdatePending,
    #[serde(rename = "Incomplete Input")]
    IncompleteInput,
    #[serde(rename = "Schema validation failed")]
    SchemaValidationFailed,
    #[serde(rename = "Completed")]
    Completed,
}

#[derive(Serialize, Clone, Debug)]
pub struct HydrationEvent {
    pub status: HydrationStatus,
    pub hydrated_schema: Value,
    pub query_status: HashMap<String, String>,
    pub validation_errors: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct QueryTask {
    pub field_name: String,
    pub query_type: String,
    pub params: HashMap<String, String>,
    pub dependencies: Vec<String>,
    pub status: String,
}
