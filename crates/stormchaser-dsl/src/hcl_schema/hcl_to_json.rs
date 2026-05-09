use anyhow::Result;
use hcl::expr::{Expression, ObjectKey, TraversalOperator};
use hcl::Body;
use serde_json::{Map, Number as JsonNumber, Value};

/// Deserializes HCL data format back into a JSON Schema (serde_json::Value).
///
/// This is the inverse of [`json_schema_to_hcl`] and the two functions form a
/// symmetric pair.  The following conventions are used:
///
/// * **Blocks** – a `definitions "Name" { … }` block is decoded by mapping each
///   attribute directly onto the corresponding definition key, preserving all
///   constraints (`type`, `required`, `additionalProperties`, etc.).
/// * **Top-level attributes** – attributes whose value is a *type-function call*
///   (e.g. `integer(…)`) or a `$ref`-style *traversal* are recognised as property
///   schemas and collected under `properties`.  All other attributes (strings,
///   arrays, plain objects) are treated as root-level schema keywords and placed
///   directly on the schema object.
pub fn hcl_to_json_schema(body: &Body) -> Result<Value> {
    let mut map = Map::new();

    // Process blocks (e.g., definitions "Address" { … }).
    // Each attribute in the block body is a direct key of the definition object,
    // which matches exactly what json_schema_to_hcl emits.
    for block in body.blocks() {
        let block_name = block.identifier();
        let mut definition_map = Map::new();
        for attr in block.body().attributes() {
            definition_map.insert(attr.key().to_string(), hcl_expr_to_json(attr.expr())?);
        }

        let defs_entry = map
            .entry(block_name)
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(defs_map) = defs_entry {
            if let Some(label) = block.labels().first() {
                defs_map.insert(label.as_str().to_string(), Value::Object(definition_map));
            }
        }
    }

    // Process top-level attributes.
    // Attributes whose values look like type-function calls or $ref traversals
    // are property definitions; everything else is a root-level schema keyword.
    let mut root_props = Map::new();
    for attr in body.attributes() {
        let key = attr.key().to_string();
        let val = hcl_expr_to_json(attr.expr())?;

        if is_property_definition(attr.expr()) {
            root_props.insert(key, val);
        } else {
            map.insert(key, val);
        }
    }

    if !root_props.is_empty() {
        map.insert("properties".to_string(), Value::Object(root_props));
        if !map.contains_key("type") {
            map.insert("type".to_string(), Value::String("object".to_string()));
        }
    }

    Ok(Value::Object(map))
}

/// Returns `true` when the expression represents a property schema definition
/// (a type-function call such as `string()` or a `$ref`-style traversal) as
/// opposed to a root-level schema keyword (string literal, array, plain object).
fn is_property_definition(expr: &Expression) -> bool {
    const TYPE_FUNCS: [&str; 7] = [
        "string", "integer", "number", "boolean", "array", "object", "map",
    ];
    match expr {
        Expression::FuncCall(func) => TYPE_FUNCS.contains(&func.name.name.as_str()),
        Expression::Traversal(_) => true,
        _ => false,
    }
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
        Expression::FuncCall(func) => func_call_to_json(func),
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

fn func_call_to_json(func: &hcl::expr::FuncCall) -> Result<Value> {
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
            // Args are inner constraint or type functions, e.g. format("email"),
            // maxItems(5), or — when outer type is "array" — a nested type function
            // like string() or string(format("email")) that describes array items.
            if let Expression::FuncCall(inner_func) = arg {
                let inner_name = inner_func.name.name.as_str();
                let is_type_func = [
                    "string", "integer", "number", "boolean", "array", "object", "map",
                ]
                .contains(&inner_name);

                if name == "array" && is_type_func {
                    // A type function nested inside array(...) describes the item schema,
                    // e.g. array(string()) or array(string(format("email"))).
                    map.insert("items".to_string(), hcl_expr_to_json(arg)?);
                } else if let Some(first_arg) = inner_func.args.first() {
                    // Constraint function with a value argument, e.g. format("email"),
                    // maxItems(5), minimum(0).
                    map.insert(inner_name.to_string(), hcl_expr_to_json(first_arg)?);
                }
                // else: zero-arg non-type function — nothing to map.
            } else if name == "array" {
                // Non-function argument to array() — treat as the items definition.
                map.insert("items".to_string(), hcl_expr_to_json(arg)?);
            }
        }
        return Ok(Value::Object(map));
    }

    // Otherwise, just a generic function call mapping, shouldn't be reached if valid schema
    Ok(Value::Null)
}
