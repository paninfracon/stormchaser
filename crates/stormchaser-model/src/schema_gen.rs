use crate::dsl::*;
use crate::events::{
    RunnerHeartbeatEvent, RunnerOfflineEvent, RunnerRegisterEvent, StepCompletedEvent,
    StepFailedEvent, StepInitializingEvent, StepQueryEvent, StepQueryResponseEvent,
    StepRunningEvent, StepScheduledEvent, WorkflowAbortedEvent, WorkflowCompletedEvent,
    WorkflowFailedEvent, WorkflowQueuedEvent, WorkflowRunningEvent, WorkflowStartPendingEvent,
};
use schemars::generate::SchemaSettings;
use schemars::transform::{transform_subschemas, Transform};
use schemars::{JsonSchema, Schema};

/// Generates a standalone JSON Schema Draft-07 document for `T`.
pub fn draft07_schema_for<T: JsonSchema + ?Sized>() -> Schema {
    SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

macro_rules! schema_for {
    ($t:ty) => {
        draft07_schema_for::<$t>()
    };
}

/// Generates a map of all event schemas.
pub fn generate_event_schemas() -> HashMap<String, Schema> {
    let mut schemas = HashMap::new();
    schemas.insert(
        "WorkflowQueuedEvent".to_string(),
        schema_for!(WorkflowQueuedEvent),
    );
    schemas.insert(
        "WorkflowStartPendingEvent".to_string(),
        schema_for!(WorkflowStartPendingEvent),
    );
    schemas.insert(
        "WorkflowRunningEvent".to_string(),
        schema_for!(WorkflowRunningEvent),
    );
    schemas.insert(
        "WorkflowCompletedEvent".to_string(),
        schema_for!(WorkflowCompletedEvent),
    );
    schemas.insert(
        "WorkflowFailedEvent".to_string(),
        schema_for!(WorkflowFailedEvent),
    );
    schemas.insert(
        "WorkflowAbortedEvent".to_string(),
        schema_for!(WorkflowAbortedEvent),
    );
    schemas.insert(
        "StepScheduledEvent".to_string(),
        schema_for!(StepScheduledEvent),
    );
    schemas.insert(
        "StepRunningEvent".to_string(),
        schema_for!(StepRunningEvent),
    );
    schemas.insert(
        "StepInitializingEvent".to_string(),
        schema_for!(StepInitializingEvent),
    );
    schemas.insert(
        "StepCompletedEvent".to_string(),
        schema_for!(StepCompletedEvent),
    );
    schemas.insert("StepFailedEvent".to_string(), schema_for!(StepFailedEvent));
    schemas.insert("StepQueryEvent".to_string(), schema_for!(StepQueryEvent));
    schemas.insert(
        "StepQueryResponseEvent".to_string(),
        schema_for!(StepQueryResponseEvent),
    );
    schemas.insert(
        "RunnerRegisterEvent".to_string(),
        schema_for!(RunnerRegisterEvent),
    );
    schemas.insert(
        "RunnerHeartbeatEvent".to_string(),
        schema_for!(RunnerHeartbeatEvent),
    );
    schemas.insert(
        "RunnerOfflineEvent".to_string(),
        schema_for!(RunnerOfflineEvent),
    );
    schemas
}

use serde_json::Value;
use std::collections::HashMap;

/// Injects dynamic conditional `spec` mapping into the base `Step` schema.
///
/// This applies an `allOf` JSON Schema constraint that forces the `spec` field of a step
/// to structurally match the required subschema mapping (e.g. `K8sJobSpec`) depending on
/// the literal string value of the `type` field (e.g. `"RunK8sJob"`). It also sets
/// `additionalProperties` to `true` on steps to allow for external plugins.
pub fn apply_step_extensibility(root_schema: &mut Schema, spec_schemas: &HashMap<String, Schema>) {
    let Some(step_obj) = root_schema
        .get_mut("definitions")
        .and_then(Value::as_object_mut)
        .and_then(|definitions| definitions.get_mut("Step"))
        .and_then(Value::as_object_mut)
    else {
        return;
    };

    // Preserve any existing allOf logic
    let mut all_of = step_obj
        .get("allOf")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    for (type_name, spec_schema) in spec_schemas {
        all_of.push(serde_json::json!({
            "if": {
                "required": ["type"],
                "properties": { "type": { "const": type_name } }
            },
            "then": {
                "required": ["spec"],
                "properties": { "spec": spec_schema }
            }
        }));
    }

    step_obj.insert("allOf".to_string(), Value::Array(all_of));

    if step_obj.contains_key("properties") || step_obj.contains_key("required") {
        step_obj.insert("additionalProperties".to_string(), Value::Bool(true));
    }
}

/// Marks schemas that accept `null` with the OpenAPI-style `nullable: true` annotation,
/// keeping the explicit `null` type so plain JSON Schema validators behave the same.
#[derive(Clone)]
struct MarkNullable;

impl Transform for MarkNullable {
    fn transform(&mut self, schema: &mut Schema) {
        if let Some(obj) = schema.as_object_mut() {
            let has_null_type = obj
                .get("type")
                .and_then(Value::as_array)
                .is_some_and(|types| types.iter().any(|t| t == "null"));
            let has_null_branch =
                obj.get("anyOf")
                    .and_then(Value::as_array)
                    .is_some_and(|branches| {
                        branches
                            .iter()
                            .any(|branch| branch.get("type").is_some_and(|t| t == "null"))
                    });
            if has_null_type || has_null_branch {
                obj.insert("nullable".to_string(), Value::Bool(true));
            }
        }
        transform_subschemas(self, schema);
    }
}

/// Dynamically generates the complete JSON Schema Draft-07 for the Stormchaser DSL.
///
/// This compiles the `Workflow` AST struct into an OpenAPI/JSON Schema compatible
/// tree, embeds all standard `spec` mappings for known intrinsic step types, and
/// strips out incompatible OpenAPI meta-schemas to ensure local JSON Schema validation runs cleanly.
pub fn generate_dsl_schema() -> Schema {
    let mut generator = SchemaSettings::draft07()
        .with_transform(MarkNullable)
        .into_generator();

    let mut spec_schemas = HashMap::new();
    spec_schemas.insert(
        "RunContainer".to_string(),
        generator.subschema_for::<CommonContainerSpec>(),
    );
    spec_schemas.insert(
        "RunK8sJob".to_string(),
        generator.subschema_for::<K8sJobSpec>(),
    );
    spec_schemas.insert(
        "Approval".to_string(),
        generator.subschema_for::<ApprovalSpec>(),
    );
    spec_schemas.insert(
        "Wait".to_string(),
        generator.subschema_for::<WaitEventSpec>(),
    );
    spec_schemas.insert(
        "LambdaInvoke".to_string(),
        generator.subschema_for::<LambdaInvokeSpec>(),
    );
    spec_schemas.insert(
        "GitCheckout".to_string(),
        generator.subschema_for::<GitCheckoutSpec>(),
    );
    spec_schemas.insert(
        "Wasm".to_string(),
        generator.subschema_for::<WasmStepSpec>(),
    );
    spec_schemas.insert(
        "SlackMessage".to_string(),
        generator.subschema_for::<SlackMessageSpec>(),
    );
    spec_schemas.insert(
        "TeamsMessage".to_string(),
        generator.subschema_for::<TeamsMessageSpec>(),
    );
    spec_schemas.insert(
        "WebhookInvoke".to_string(),
        generator.subschema_for::<WebhookInvokeSpec>(),
    );
    spec_schemas.insert(
        "RestApi".to_string(),
        generator.subschema_for::<RestApiSpec>(),
    );
    spec_schemas.insert(
        "SqlExecute".to_string(),
        generator.subschema_for::<SqlExecuteSpec>(),
    );
    spec_schemas.insert(
        "EmailSend".to_string(),
        generator.subschema_for::<EmailSpec>(),
    );
    spec_schemas.insert(
        "JinjaRender".to_string(),
        generator.subschema_for::<JinjaRenderSpec>(),
    );
    spec_schemas.insert(
        "TestReportEmail".to_string(),
        generator.subschema_for::<TestReportEmailSpec>(),
    );
    spec_schemas.insert("JQ".to_string(), generator.subschema_for::<JqSpec>());

    // Keyed by the names workflows use, which the step type list holds
    debug_assert!(spec_schemas
        .keys()
        .all(|name| crate::dsl::step_types::step_type(name).is_some()));

    let mut root_schema = generator.root_schema_for::<Workflow>();

    apply_step_extensibility(&mut root_schema, &spec_schemas);

    // Remove the OpenAPI meta-schema to avoid jsonschema validation errors on unrecognized drafts
    root_schema.remove("$schema");
    root_schema
}

/// Flattens conditional `allOf` blocks into the current node's properties based on current inputs.
/// This allows UI clients to easily render active conditional fields without complex parsing.
pub fn flatten_schema_for_ui(schema: &Value, inputs: &Value) -> Value {
    let mut flat = schema.clone();

    fn recursive_flatten(node: &mut Value, current_inputs: &Value) {
        if let Some(obj) = node.as_object_mut() {
            // First, recurse into properties
            if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
                for v in props.values_mut() {
                    recursive_flatten(v, current_inputs);
                }
            }

            // Now handle allOf at this level
            if let Some(all_of) = obj.get("allOf").and_then(|v| v.as_array()) {
                let mut splices: Vec<(Option<String>, serde_json::Map<String, Value>)> = Vec::new();
                let mut merged_required = Vec::new();

                for condition in all_of {
                    // Check if condition matches current inputs.
                    // This is a naive evaluation: if all const properties in `if` match `inputs`.
                    let mut matches = true;
                    let mut last_trigger_field = None;

                    if let Some(if_props) = condition
                        .get("if")
                        .and_then(|i| i.get("properties"))
                        .and_then(|p| p.as_object())
                    {
                        for (k, v) in if_props {
                            last_trigger_field = Some(k.clone());
                            if let Some(const_val) = v.get("const") {
                                if current_inputs.get(k) != Some(const_val) {
                                    matches = false;
                                    break;
                                }
                            } else {
                                // If it's not a const check, we conservatively don't match or assume false for this basic UI flattener.
                                matches = false;
                                break;
                            }
                        }
                    } else {
                        matches = false; // No if properties
                    }

                    if matches {
                        if let Some(then_props) = condition
                            .get("then")
                            .and_then(|t| t.get("properties"))
                            .and_then(|p| p.as_object())
                        {
                            splices.push((last_trigger_field, then_props.clone()));
                        }
                        if let Some(then_req) = condition
                            .get("then")
                            .and_then(|t| t.get("required"))
                            .and_then(|r| r.as_array())
                        {
                            for r in then_req {
                                if let Some(rs) = r.as_str() {
                                    merged_required.push(Value::String(rs.to_string()));
                                }
                            }
                        }
                    }
                }

                if !splices.is_empty() {
                    if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
                        let mut new_props = serde_json::Map::new();
                        let mut processed_splices = vec![false; splices.len()];

                        for (k, v) in props.iter() {
                            new_props.insert(k.clone(), v.clone());
                            for (i, (trigger, then_props)) in splices.iter().enumerate() {
                                if !processed_splices[i] && trigger.as_ref() == Some(k) {
                                    for (tk, tv) in then_props {
                                        new_props.insert(tk.clone(), tv.clone());
                                    }
                                    processed_splices[i] = true;
                                }
                            }
                        }

                        // Fallback: apply splices whose trigger field wasn't found
                        for (i, (_, then_props)) in splices.iter().enumerate() {
                            if !processed_splices[i] {
                                for (tk, tv) in then_props {
                                    new_props.insert(tk.clone(), tv.clone());
                                }
                            }
                        }
                        *props = new_props;
                    } else {
                        let mut merged_props = serde_json::Map::new();
                        for (_, then_props) in splices {
                            for (tk, tv) in then_props {
                                merged_props.insert(tk, tv);
                            }
                        }
                        obj.insert("properties".to_string(), Value::Object(merged_props));
                    }
                }

                if !merged_required.is_empty() {
                    if let Some(reqs) = obj.get_mut("required").and_then(|v| v.as_array_mut()) {
                        for r in merged_required {
                            if !reqs.contains(&r) {
                                reqs.push(r);
                            }
                        }
                    } else {
                        obj.insert("required".to_string(), Value::Array(merged_required));
                    }
                }
            }

            // Optionally, strip allOf to keep it completely flat for the UI
            // obj.remove("allOf");
        }
    }

    recursive_flatten(&mut flat, inputs);
    flat
}

#[cfg(test)]
mod tests {
    use super::generate_dsl_schema;

    #[test]
    fn spec_schemas_are_keyed_by_step_types_workflows_use() {
        let schema = serde_json::to_value(generate_dsl_schema()).unwrap();
        let keys: Vec<String> = schema["definitions"]["Step"]["allOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|rule| rule["if"]["properties"]["type"]["const"].as_str())
            .map(str::to_string)
            .collect();
        assert!(keys.len() >= 15, "{keys:?}");
        for key in &keys {
            assert!(
                crate::dsl::step_types::step_type(key).is_some(),
                "{key} is not a step type"
            );
        }
        for name in ["EmailSend", "JQ", "Wait"] {
            assert!(keys.iter().any(|k| k == name), "{name} has no spec schema");
        }
    }

    #[test]
    fn test_generate_dsl_schema_registers_rest_api_spec() {
        let schema = generate_dsl_schema();
        let rest_api_schema = schema
            .pointer("/definitions/RestApiSpec")
            .expect("generated DSL schema should include the RestApiSpec definition");
        let rest_api_schema_json = serde_json::to_value(rest_api_schema)
            .expect("RestApiSpec schema should serialize to JSON value");
        let properties = rest_api_schema_json
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("RestApiSpec schema should expose object properties");

        assert!(
            properties.contains_key("url"),
            "RestApiSpec schema should include the url property"
        );
        assert!(
            properties.contains_key("extractors"),
            "RestApiSpec schema should include the extractors property"
        );
    }

    #[test]
    fn test_generate_dsl_schema_registers_sql_execute_spec() {
        let schema = generate_dsl_schema();
        let sql_execute_schema = schema
            .pointer("/definitions/SqlExecuteSpec")
            .expect("generated DSL schema should include the SqlExecuteSpec definition");
        let sql_execute_schema_json = serde_json::to_value(sql_execute_schema)
            .expect("SqlExecuteSpec schema should serialize to JSON value");
        let properties = sql_execute_schema_json
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("SqlExecuteSpec schema should expose object properties");

        assert!(
            properties.contains_key("connection"),
            "SqlExecuteSpec schema should include the connection property"
        );
        assert!(
            properties.contains_key("query"),
            "SqlExecuteSpec schema should include the query property"
        );
        assert!(
            properties.contains_key("params"),
            "SqlExecuteSpec schema should include the params property"
        );
    }

    #[test]
    fn test_flatten_schema_for_ui() {
        use super::flatten_schema_for_ui;
        use serde_json::json;

        let schema = json!({
            "properties": {
                "type": { "type": "string" }
            },
            "allOf": [
                {
                    "if": { "properties": { "type": { "const": "RunK8sJob" } } },
                    "then": { "properties": { "spec": { "type": "object" } }, "required": ["spec"] }
                }
            ]
        });

        let inputs = json!({
            "type": "RunK8sJob"
        });

        let flat = flatten_schema_for_ui(&schema, &inputs);
        let properties = flat.get("properties").unwrap().as_object().unwrap();
        assert!(properties.contains_key("spec"));

        let required = flat.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("spec")));

        let inputs_no_match = json!({
            "type": "Other"
        });

        let flat_no_match = flatten_schema_for_ui(&schema, &inputs_no_match);
        let properties_no_match = flat_no_match
            .get("properties")
            .unwrap()
            .as_object()
            .unwrap();
        assert!(!properties_no_match.contains_key("spec"));
    }

    #[test]
    fn test_flatten_schema_for_ui_ordering() {
        use super::flatten_schema_for_ui;
        use serde_json::json;

        let schema = json!({
            "properties": {
                "first_field": { "type": "string" },
                "trigger_field": { "type": "string" },
                "last_field": { "type": "string" }
            },
            "allOf": [
                {
                    "if": { "properties": { "trigger_field": { "const": "show_more" } } },
                    "then": { "properties": { "cond_field1": { "type": "string" } } }
                }
            ]
        });

        let inputs = json!({
            "trigger_field": "show_more"
        });

        let flat = flatten_schema_for_ui(&schema, &inputs);
        let props = flat.get("properties").unwrap().as_object().unwrap();
        let keys: Vec<_> = props.keys().map(|s| s.as_str()).collect();
        assert_eq!(
            keys,
            vec!["first_field", "trigger_field", "cond_field1", "last_field"]
        );
    }
}
