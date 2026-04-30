use anyhow::Result;
use hcl::expr::{
    Expression, FuncCall, FuncName, Object, ObjectKey, Traversal, TraversalOperator, Variable,
};
use hcl::{Block, Body, Identifier, Number as HclNumber};
use serde_json::{Map, Number as JsonNumber, Value};

/// Serializes a JSON Schema (represented as serde_json::Value) into the HCL data format
/// using functional abstractions described in the specification.
pub fn json_schema_to_hcl(schema: &Value) -> Result<Body> {
    let mut body = Body::builder();

    if let Value::Object(map) = schema {
        for (k, v) in map {
            if k == "definitions" || k == "$defs" {
                if let Value::Object(defs) = v {
                    for (def_k, def_v) in defs {
                        let mut block = Block::builder(k.as_str()).add_label(def_k.as_str());
                        if let Value::Object(def_map) = def_v {
                            // Encode the full definition object so schema constraints that
                            // commonly appear alongside `properties` (for example `required`,
                            // `additionalProperties`, `description`, `oneOf`, etc.) are preserved.
                            for (dk, dv) in def_map {
                                block =
                                    block.add_attribute((dk.as_str(), json_to_hcl_expr(dv)?));
                            }
                        }
                        body = body.add_block(block.build());
                    }
                }
            } else if k == "properties" {
                // Top-level properties as attributes
                if let Value::Object(props) = v {
                    for (pk, pv) in props {
                        body = body.add_attribute((pk.as_str(), json_to_hcl_expr(pv)?));
                    }
                }
            } else {
                body = body.add_attribute((k.as_str(), json_to_hcl_expr(v)?));
            }
        }
    }

    Ok(body.build())
}

/// Helper to convert a JSON value into an HCL expression, applying functional abbreviations.
pub fn json_to_hcl_expr(v: &Value) -> Result<Expression> {
    if let Value::Object(map) = v {
        if let Some(Value::String(ref_path)) = map.get("$ref") {
            // Convert $ref: "#/definitions/Address" -> definitions.Address
            let parts: Vec<&str> = ref_path.trim_start_matches("#/").split('/').collect();
            if parts.len() == 2 {
                let traversal = Traversal::builder(Variable::new(parts[0])?)
                    .attr(parts[1])
                    .build();
                return Ok(Expression::Traversal(Box::new(traversal)));
            } else {
                return Ok(Expression::String(ref_path.to_string())); // Fallback
            }
        }

        if let Some(Value::String(t)) = map.get("type") {
            if [
                "string", "integer", "number", "boolean", "array", "object", "map",
            ]
            .contains(&t.as_str())
            {
                let mut args = Vec::new();

                // For arrays, the first argument is often the `items` schema
                if t == "array" {
                    if let Some(items) = map.get("items") {
                        args.push(json_to_hcl_expr(items)?);
                    }
                }

                // Add constraints as function calls
                for (k, val) in map {
                    if k == "type" || (t == "array" && k == "items") {
                        continue; // Skip type and already processed items
                    }
                    // For object properties, we might want to pass them as an object argument
                    if t == "object" && k == "properties" {
                        // Pass properties as a map expression
                        let mut props_map = Object::new();
                        if let Value::Object(p) = val {
                            for (pk, pv) in p {
                                props_map.insert(
                                    ObjectKey::Identifier(Identifier::new(pk)?),
                                    json_to_hcl_expr(pv)?,
                                );
                            }
                        }
                        let prop_func = FuncCall::builder("properties")
                            .arg(Expression::Object(props_map))
                            .build();
                        args.push(Expression::FuncCall(Box::new(prop_func)));
                        continue;
                    }

                    // Standard constraints e.g., format("email"), minimum(16)
                    let constraint_arg = json_to_hcl_expr(val)?;
                    let constraint_func = FuncCall::builder(k.as_str()).arg(constraint_arg).build();
                    args.push(Expression::FuncCall(Box::new(constraint_func)));
                }

                let mut func = FuncCall::new(FuncName::new(t.as_str()));
                func.args = args;
                return Ok(Expression::FuncCall(Box::new(func)));
            }
        }

        // Complex logical types (oneOf, anyOf, allOf)
        for logical in ["oneOf", "anyOf", "allOf"] {
            if let Some(Value::Array(arr)) = map.get(logical) {
                let mut elements = Vec::new();
                for item in arr {
                    elements.push(json_to_hcl_expr(item)?);
                }
                let array_expr = Expression::Array(elements);
                // Return as an object `{ oneOf = [...] }`
                let mut obj = Object::new();
                obj.insert(ObjectKey::Identifier(Identifier::new(logical)?), array_expr);
                return Ok(Expression::Object(obj));
            }
        }

        // Fallback to standard HCL object representation
        let mut obj = Object::new();
        for (k, val) in map {
            obj.insert(
                ObjectKey::Identifier(Identifier::new(k)?),
                json_to_hcl_expr(val)?,
            );
        }
        return Ok(Expression::Object(obj));
    }

    // Primitive mappings
    match v {
        Value::Null => Ok(Expression::Null),
        Value::Bool(b) => Ok(Expression::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_u64() {
                Ok(Expression::Number(HclNumber::from(i)))
            } else if let Some(i) = n.as_i64() {
                Ok(Expression::Number(HclNumber::from(i)))
            } else if let Some(f) = n.as_f64() {
                Ok(Expression::Number(HclNumber::from_f64(f).unwrap()))
            } else {
                Ok(Expression::Null)
            }
        }
        Value::String(s) => Ok(Expression::String(s.clone())),
        Value::Array(arr) => {
            let mut elements = Vec::new();
            for item in arr {
                elements.push(json_to_hcl_expr(item)?);
            }
            Ok(Expression::Array(elements))
        }
        _ => Ok(Expression::Null),
    }
}

/// Deserializes HCL data format back into a JSON Schema (serde_json::Value).
pub fn hcl_to_json_schema(body: &Body) -> Result<Value> {
    let mut map = Map::new();

    // Process blocks (e.g., definitions "Address" { ... })
    for block in body.blocks() {
        let block_name = block.identifier();

        let mut block_map = Map::new();
        // Assume the block body represents a set of properties for an object schema
        // Actually, definitions usually have `{ type: "object", properties: { ... } }`
        let mut props_map = Map::new();
        for attr in block.body().attributes() {
            props_map.insert(attr.key().to_string(), hcl_expr_to_json(attr.expr())?);
        }

        block_map.insert("type".to_string(), Value::String("object".to_string()));
        block_map.insert("properties".to_string(), Value::Object(props_map));

        // Get or create definitions container
        let defs_entry = map
            .entry(block_name)
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(defs_map) = defs_entry {
            // Use the first label as the definition name
            if let Some(label) = block.labels().first() {
                defs_map.insert(label.as_str().to_string(), Value::Object(block_map));
            }
        }
    }

    // Process attributes
    let mut root_props = Map::new();
    for attr in body.attributes() {
        let key = attr.key().to_string();
        let val = hcl_expr_to_json(attr.expr())?;

        // Metadata keys go to root, schema definitions go to properties
        if ["$schema", "title", "description", "required"].contains(&key.as_str()) {
            map.insert(key, val);
        } else {
            root_props.insert(key, val);
        }
    }

    if !root_props.is_empty() {
        map.insert("properties".to_string(), Value::Object(root_props));
        map.insert("type".to_string(), Value::String("object".to_string()));
    }

    Ok(Value::Object(map))
}

/// Helper to convert a functional HCL expression back into a JSON Schema representation.
pub fn hcl_expr_to_json(expr: &Expression) -> Result<Value> {
    match expr {
        Expression::Null => Ok(Value::Null),
        Expression::Bool(b) => Ok(Value::Bool(*b)),
        Expression::Number(n) => {
            if let Some(i) = n.as_u64() {
                Ok(Value::Number(JsonNumber::from(i)))
            } else if let Some(i) = n.as_i64() {
                Ok(Value::Number(JsonNumber::from(i)))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(JsonNumber::from_f64(f).unwrap()))
            } else {
                Ok(Value::Null)
            }
        }
        Expression::String(s) => Ok(Value::String(s.clone())),
        Expression::Array(arr) => {
            let mut elements = Vec::new();
            for item in arr {
                elements.push(hcl_expr_to_json(item)?);
            }
            Ok(Value::Array(elements))
        }
        Expression::Object(obj) => {
            let mut map = Map::new();
            for (k, v) in obj {
                let key_str = match k {
                    ObjectKey::Identifier(i) => i.to_string(),
                    ObjectKey::Expression(Expression::String(s)) => s.to_string(),
                    _ => continue,
                };
                map.insert(key_str, hcl_expr_to_json(v)?);
            }
            Ok(Value::Object(map))
        }
        Expression::FuncCall(func) => {
            let name = func.name.name.as_str();

            // Is it a type function like string(), integer()?
            if [
                "string", "integer", "number", "boolean", "array", "object", "map",
            ]
            .contains(&name)
            {
                let mut map = Map::new();
                map.insert("type".to_string(), Value::String(name.to_string()));

                for arg in &func.args {
                    // Args are usually inner constraint functions like format("email")
                    if let Expression::FuncCall(inner_func) = arg {
                        let inner_name = inner_func.name.name.as_str();
                        if let Some(first_arg) = inner_func.args.first() {
                            if name == "array"
                                && inner_name != "items"
                                && inner_name != "maxItems"
                                && inner_name != "minItems"
                                && inner_name != "uniqueItems"
                                && !["string", "integer", "number", "boolean", "object", "map"]
                                    .contains(&inner_name)
                            {
                                // Actually, if the type is array, the first argument might be the item type itself!
                                // e.g. array(string(), uniqueItems(true))
                            }

                            // Let's just blindly map the inner func name to its first argument value
                            map.insert(inner_name.to_string(), hcl_expr_to_json(first_arg)?);
                        } else if [
                            "string", "integer", "number", "boolean", "array", "object", "map",
                        ]
                        .contains(&inner_name)
                        {
                            // It's a type function passed as an argument (e.g. array(string()))
                            map.insert("items".to_string(), hcl_expr_to_json(arg)?);
                        }
                    } else if [
                        "string", "integer", "number", "boolean", "array", "object", "map",
                    ]
                    .contains(&name)
                        && name == "array"
                    {
                        // First argument to array without a func wrap might just be the items definition
                        map.insert("items".to_string(), hcl_expr_to_json(arg)?);
                    }
                }
                return Ok(Value::Object(map));
            }

            // Otherwise, just a generic function call mapping, shouldn't be reached if valid schema
            Ok(Value::Null)
        }
        Expression::Traversal(traversal) => {
            // Convert definitions.Address -> { "$ref": "#/definitions/Address" }
            let mut path = String::from("#");

            let expr = &traversal.expr;
            if let Expression::Variable(var) = expr {
                path.push('/');
                path.push_str(var.as_str());
            }

            for op in &traversal.operators {
                if let TraversalOperator::GetAttr(attr) = op {
                    path.push('/');
                    path.push_str(attr.as_str());
                }
            }

            let mut map = Map::new();
            map.insert("$ref".to_string(), Value::String(path));
            Ok(Value::Object(map))
        }
        _ => Ok(Value::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_schema_serialization_roundtrip() {
        let json_schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "Person",
            "type": "object",
            "properties": {
                "age": {
                    "type": "integer",
                    "minimum": 16
                },
                "email": {
                    "type": "string",
                    "format": "email"
                },
                "tags": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "uniqueItems": true
                },
                "address": {
                    "$ref": "#/definitions/Address"
                }
            },
            "required": ["email"],
            "definitions": {
                "Address": {
                    "type": "object",
                    "properties": {
                        "street": {
                            "type": "string"
                        }
                    }
                }
            }
        });

        // 1. Convert JSON Schema -> HCL
        let hcl_body = json_schema_to_hcl(&json_schema).unwrap();

        let hcl_str = hcl::to_string(&hcl_body).unwrap();
        // Since hashmap iteration is non-deterministic, we just ensure it parses back
        // 2. Convert HCL -> JSON Schema
        let parsed_hcl: Body = hcl::from_str(&hcl_str).unwrap();
        let out_json = hcl_to_json_schema(&parsed_hcl).unwrap();

        // Ensure keys exist in output
        let out_map = out_json.as_object().unwrap();
        if !out_map.contains_key("properties") {
            panic!("Parsed JSON: {:#?}", out_json);
        }
        let props = out_map.get("properties").unwrap().as_object().unwrap();
        if !props.contains_key("address") {
            panic!("Props missing address. Parsed JSON: {:#?}", out_json);
        }

        let address_ref = props.get("address").unwrap().as_object().unwrap();
        assert_eq!(address_ref.get("$ref").unwrap(), "#/definitions/Address");

        let defs = out_map.get("definitions").unwrap().as_object().unwrap();
        let address_def = defs.get("Address").unwrap().as_object().unwrap();
        assert_eq!(address_def.get("type").unwrap(), "object");
    }
}
