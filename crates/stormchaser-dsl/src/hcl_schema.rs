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

    /// Verify that the JSON→HCL→JSON round-trip preserves all important schema constraints.
    ///
    /// Note: JSON Schema keys that start with `$` (e.g. `$schema`, `$id`) are not valid
    /// bare HCL identifiers and cannot be represented in the HCL format; they are silently
    /// dropped when serializing.  All other constraints survive the round-trip faithfully.
    #[test]
    fn test_roundtrip_json_to_hcl_to_json() {
        let original = json!({
            "title": "Person",
            "type": "object",
            "required": ["email"],
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
                    }
                },
                "address": {
                    "$ref": "#/definitions/Address"
                }
            },
            "definitions": {
                "Address": {
                    "type": "object",
                    "required": ["street"],
                    "properties": {
                        "street": {
                            "type": "string"
                        }
                    }
                }
            }
        });

        // JSON → HCL
        let hcl_body = json_schema_to_hcl(&original).unwrap();
        let hcl_str = hcl::to_string(&hcl_body).unwrap();

        // HCL → JSON
        let parsed_body: Body = hcl::from_str(&hcl_str).unwrap();
        let recovered = hcl_to_json_schema(&parsed_body).unwrap();

        let root = recovered.as_object().expect("root must be an object");

        // Root-level schema keywords
        assert_eq!(
            root.get("title").and_then(Value::as_str),
            Some("Person"),
            "root title preserved"
        );
        assert_eq!(
            root.get("type").and_then(Value::as_str),
            Some("object"),
            "root type preserved"
        );
        let required = root
            .get("required")
            .and_then(Value::as_array)
            .expect("required must be an array");
        assert!(
            required.iter().any(|v| v.as_str() == Some("email")),
            "required contains email"
        );

        // Properties
        let props = root
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");
        assert!(props.contains_key("age"), "age property preserved");
        assert!(props.contains_key("email"), "email property preserved");
        assert!(props.contains_key("tags"), "tags property preserved");
        assert!(props.contains_key("address"), "address property preserved");

        let age = props["age"].as_object().expect("age must be an object");
        assert_eq!(age.get("type").and_then(Value::as_str), Some("integer"));

        let email_prop = props["email"].as_object().expect("email must be an object");
        assert_eq!(
            email_prop.get("type").and_then(Value::as_str),
            Some("string")
        );

        let tags = props["tags"].as_object().expect("tags must be an object");
        assert_eq!(tags.get("type").and_then(Value::as_str), Some("array"));

        // $ref preserved
        let address_prop = props["address"]
            .as_object()
            .expect("address must be an object");
        assert_eq!(
            address_prop.get("$ref").and_then(Value::as_str),
            Some("#/definitions/Address"),
            "$ref preserved through round-trip"
        );

        // Definitions: full definition object (type + required + properties) preserved
        let defs = root
            .get("definitions")
            .and_then(Value::as_object)
            .expect("definitions must be an object");
        let addr_def = defs
            .get("Address")
            .and_then(Value::as_object)
            .expect("Address definition must exist");
        assert_eq!(
            addr_def.get("type").and_then(Value::as_str),
            Some("object"),
            "Address definition type preserved"
        );
        let addr_required = addr_def
            .get("required")
            .and_then(Value::as_array)
            .expect("Address required must be an array");
        assert!(
            addr_required.iter().any(|v| v.as_str() == Some("street")),
            "Address required contains street"
        );
        let addr_props = addr_def
            .get("properties")
            .and_then(Value::as_object)
            .expect("Address properties must be an object");
        assert!(
            addr_props.contains_key("street"),
            "Address.street property preserved"
        );
    }

    /// Verify that an array whose items schema has constraints round-trips correctly.
    ///
    /// The HCL encoding is `array(string(format("email")))` and the expected JSON is
    /// `{"type": "array", "items": {"type": "string", "format": "email"}}`.
    #[test]
    fn test_array_items_with_constraints_roundtrip() {
        let original = json!({
            "type": "object",
            "properties": {
                "emails": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "format": "email"
                    }
                },
                "counts": {
                    "type": "array",
                    "items": {
                        "type": "integer"
                    }
                }
            }
        });

        // JSON → HCL
        let hcl_body = json_schema_to_hcl(&original).unwrap();
        let hcl_str = hcl::to_string(&hcl_body).unwrap();

        // HCL → JSON
        let parsed_body: Body = hcl::from_str(&hcl_str).unwrap();
        let recovered = hcl_to_json_schema(&parsed_body).unwrap();

        let root = recovered.as_object().expect("root must be object");
        let props = root
            .get("properties")
            .and_then(Value::as_object)
            .expect("properties must be an object");

        // emails: array of strings with format constraint
        let emails = props["emails"].as_object().expect("emails must be object");
        assert_eq!(emails.get("type").and_then(Value::as_str), Some("array"));
        let email_items = emails
            .get("items")
            .and_then(Value::as_object)
            .expect("emails.items must be object");
        assert_eq!(
            email_items.get("type").and_then(Value::as_str),
            Some("string"),
            "emails.items type preserved"
        );
        assert_eq!(
            email_items.get("format").and_then(Value::as_str),
            Some("email"),
            "emails.items format constraint preserved"
        );

        // counts: array of integers (simple case)
        let counts = props["counts"].as_object().expect("counts must be object");
        assert_eq!(counts.get("type").and_then(Value::as_str), Some("array"));
        let count_items = counts
            .get("items")
            .and_then(Value::as_object)
            .expect("counts.items must be object");
        assert_eq!(
            count_items.get("type").and_then(Value::as_str),
            Some("integer"),
            "counts.items type preserved"
        );
    }

    /// written HCL body (not produced by json_schema_to_hcl).
    #[test]
    fn test_hcl_to_json_schema_direct() {
        let hcl_src = r#"
            title = "Minimal"
            type = "object"
            required = ["name"]

            name = string()
            count = integer()
        "#;
        let body: Body = hcl::from_str(hcl_src).unwrap();
        let schema = hcl_to_json_schema(&body).unwrap();

        let root = schema.as_object().unwrap();
        assert_eq!(root.get("title").and_then(Value::as_str), Some("Minimal"));
        assert_eq!(root.get("type").and_then(Value::as_str), Some("object"));

        let req = root.get("required").and_then(Value::as_array).unwrap();
        assert_eq!(req.len(), 1);
        assert_eq!(req[0].as_str(), Some("name"));

        let props = root.get("properties").and_then(Value::as_object).unwrap();
        assert!(props.contains_key("name"));
        assert!(props.contains_key("count"));
        assert_eq!(
            props["name"].as_object().unwrap().get("type"),
            Some(&Value::String("string".into()))
        );
    }
}
