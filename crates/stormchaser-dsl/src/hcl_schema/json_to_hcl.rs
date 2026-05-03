use anyhow::Result;
use hcl::expr::{Expression, FuncCall, FuncName, Object, ObjectKey, Traversal, Variable};
use hcl::{Block, Body, Identifier, Number as HclNumber};
use serde_json::Value;

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
                                block = block.add_attribute((dk.as_str(), json_to_hcl_expr(dv)?));
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
