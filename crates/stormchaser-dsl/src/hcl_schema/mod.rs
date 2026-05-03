pub mod hcl_to_json;
pub mod json_to_hcl;

pub use hcl_to_json::*;
pub use json_to_hcl::*;

#[cfg(test)]
mod tests {
    use super::*;
    use hcl::Body;
    use serde_json::{json, Value};

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
