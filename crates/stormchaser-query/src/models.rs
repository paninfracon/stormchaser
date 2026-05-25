use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct HydrateSchemaRequest {
    pub schema: Value,
    #[serde(default)]
    pub inputs: Value,
    #[serde(default)]
    pub queries: Vec<Value>,
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
