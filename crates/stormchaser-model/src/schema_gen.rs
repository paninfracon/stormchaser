use crate::dsl::*;
use crate::events::{
    RunnerHeartbeatEvent, RunnerOfflineEvent, RunnerRegisterEvent, StepCompletedEvent,
    StepFailedEvent, StepQueryEvent, StepQueryResponseEvent, StepRunningEvent, StepScheduledEvent,
    WorkflowAbortedEvent, WorkflowCompletedEvent, WorkflowFailedEvent, WorkflowQueuedEvent,
    WorkflowRunningEvent, WorkflowStartPendingEvent,
};
use schemars::schema::{ObjectValidation, RootSchema, Schema, SchemaObject, SubschemaValidation};
use schemars::{schema_for, Map};

/// Generates a map of all event schemas.
pub fn generate_event_schemas() -> HashMap<String, RootSchema> {
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
pub fn apply_step_extensibility(
    root_schema: &mut RootSchema,
    spec_schemas: &HashMap<String, Schema>,
) {
    if let Some(Schema::Object(step_obj)) = root_schema.definitions.get_mut("Step") {
        let mut all_of = Vec::new();

        // Preserve any existing allOf logic
        if let Some(subschemas) = &mut step_obj.subschemas {
            if let Some(existing_all_of) = &subschemas.all_of {
                all_of.extend(existing_all_of.clone());
            }
        }

        for (type_name, spec_schema) in spec_schemas {
            let mut if_schema = SchemaObject::default();
            let mut properties = Map::new();

            let type_schema = SchemaObject {
                const_value: Some(Value::String(type_name.clone())),
                ..Default::default()
            };

            properties.insert("type".to_string(), Schema::Object(type_schema));
            if_schema.object = Some(Box::new(ObjectValidation {
                properties,
                required: std::collections::BTreeSet::from(["type".to_string()]),
                ..Default::default()
            }));

            let mut then_schema = SchemaObject::default();
            let mut then_props = Map::new();
            then_props.insert("spec".to_string(), spec_schema.clone());
            then_schema.object = Some(Box::new(ObjectValidation {
                properties: then_props,
                required: std::collections::BTreeSet::from(["spec".to_string()]),
                ..Default::default()
            }));

            let condition = SchemaObject {
                subschemas: Some(Box::new(SubschemaValidation {
                    if_schema: Some(Box::new(Schema::Object(if_schema))),
                    then_schema: Some(Box::new(Schema::Object(then_schema))),
                    ..Default::default()
                })),
                ..Default::default()
            };
            all_of.push(Schema::Object(condition));
        }

        if let Some(subschemas) = &mut step_obj.subschemas {
            subschemas.all_of = Some(all_of);
        } else {
            step_obj.subschemas = Some(Box::new(SubschemaValidation {
                all_of: Some(all_of),
                ..Default::default()
            }));
        }

        if let Some(obj) = step_obj.object.as_mut() {
            obj.additional_properties = Some(Box::new(Schema::Bool(true)));
        }
    }
}

/// Dynamically generates the complete JSON Schema Draft-07 for the Stormchaser DSL.
///
/// This compiles the `Workflow` AST struct into an OpenAPI/JSON Schema compatible
/// tree, embeds all standard `spec` mappings for known intrinsic step types, and
/// strips out incompatible OpenAPI meta-schemas to ensure local JSON Schema validation runs cleanly.
pub fn generate_dsl_schema() -> RootSchema {
    let mut generator = schemars::gen::SchemaSettings::draft07()
        .with(|s| s.option_nullable = true)
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
        "WaitEvent".to_string(),
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
    spec_schemas.insert("Email".to_string(), generator.subschema_for::<EmailSpec>());
    spec_schemas.insert(
        "JinjaRender".to_string(),
        generator.subschema_for::<JinjaRenderSpec>(),
    );
    spec_schemas.insert(
        "TestReportEmail".to_string(),
        generator.subschema_for::<TestReportEmailSpec>(),
    );
    spec_schemas.insert("Jq".to_string(), generator.subschema_for::<JqSpec>());

    let mut root_schema = generator.root_schema_for::<Workflow>();

    apply_step_extensibility(&mut root_schema, &spec_schemas);

    // Remove the OpenAPI meta-schema to avoid jsonschema validation errors on unrecognized drafts
    root_schema.meta_schema = None;
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
                let mut merged_props = serde_json::Map::new();
                let mut merged_required = Vec::new();

                for condition in all_of {
                    // Check if condition matches current inputs.
                    // This is a naive evaluation: if all const properties in `if` match `inputs`.
                    let mut matches = true;
                    if let Some(if_props) = condition
                        .get("if")
                        .and_then(|i| i.get("properties"))
                        .and_then(|p| p.as_object())
                    {
                        for (k, v) in if_props {
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
                            for (k, v) in then_props {
                                merged_props.insert(k.clone(), v.clone());
                            }
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

                if !merged_props.is_empty() {
                    if let Some(props) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
                        for (k, v) in merged_props {
                            props.insert(k, v);
                        }
                    } else {
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
    fn test_generate_dsl_schema_registers_rest_api_spec() {
        let schema = generate_dsl_schema();
        let rest_api_schema = schema
            .definitions
            .get("RestApiSpec")
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
            .definitions
            .get("SqlExecuteSpec")
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
}
