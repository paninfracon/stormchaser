//! The input form a workflow presents to whoever runs it.

use serde_json::{json, Map, Value};

use crate::ast::Workflow;

/// The JSON Schema of a workflow's inputs, in the shape the run forms and the query
/// service's hydration work on, or `None` if the workflow takes no inputs.
///
/// A workflow declares its inputs either as a typed `inputs` schema or as a list of
/// plain `input` blocks; both come out as an object schema with `properties`.
pub fn inputs_form_schema(workflow: &Workflow) -> Option<Value> {
    if let Some(mut schema) = workflow.inputs_schema.clone() {
        if schema.get("properties").is_none() && schema.get("input").is_some() {
            let properties: Map<String, Value> = schema
                .get("input")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            if let Some(obj) = schema.as_object_mut() {
                obj.insert("properties".to_string(), Value::Object(properties));
                obj.insert("type".to_string(), json!("object"));
            }
        }
        return Some(schema);
    }

    if workflow.inputs.is_empty() {
        return None;
    }
    let mut properties = Map::new();
    for input in &workflow.inputs {
        let mut prop = Map::new();
        prop.insert("type".to_string(), json!("string"));
        prop.insert(
            "description".to_string(),
            json!(input.description.as_deref().unwrap_or(&input.name)),
        );
        if let Some(default) = &input.default {
            let default = match default.as_str() {
                Some(s) => s.to_string(),
                None => default.to_string(),
            };
            prop.insert("default".to_string(), json!(default));
        }
        properties.insert(input.name.clone(), Value::Object(prop));
    }
    Some(json!({
        "type": "object",
        "title": "Workflow Inputs",
        "properties": properties
    }))
}
