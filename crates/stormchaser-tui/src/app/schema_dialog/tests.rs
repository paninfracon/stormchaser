use super::*;
use serde_json::json;

#[test]
fn test_apply_hydrated_schema_adds_new_fields() {
    let base_schema = json!({
        "type": "object",
        "properties": {
            "type": { "type": "string" }
        }
    });

    let mut dialog = SchemaDialog::new(base_schema, "".to_string(), json!({}), None);
    assert_eq!(dialog.fields.len(), 1);
    assert_eq!(dialog.fields[0].name, "type");

    let hydrated_schema = json!({
        "type": "object",
        "properties": {
            "type": { "type": "string" },
            "spec": { "type": "object", "description": "The spec" }
        },
        "required": ["spec"]
    });

    dialog.apply_hydrated_schema(hydrated_schema, vec![], "Completed".to_string());

    assert_eq!(dialog.fields.len(), 2);
    assert_eq!(dialog.fields[0].name, "type");
    assert_eq!(dialog.fields[1].name, "spec");
    assert_eq!(dialog.fields[1].description, "The spec");
    assert!(dialog.fields[1].required);
}

fn create_dummy_field(name: &str) -> SchemaField<'static> {
    SchemaField {
        name: name.to_string(),
        description: "".to_string(),
        required: false,
        input: TextArea::default(),
        options: vec![],
        error: None,
        is_enum: false,
        schema_type: "string".to_string(),
        list_state: ListState::default(),
    }
}

#[test]
fn test_sort_fields_with_ui_order_and_wildcard() {
    let mut fields = vec![
        create_dummy_field("field_a"),
        create_dummy_field("field_b"),
        create_dummy_field("field_c"),
        create_dummy_field("field_d"),
    ];

    let inputs_view = Some(stormchaser_model::dsl::InputView {
        ui_order: vec![
            "field_c".to_string(),
            "field_a".to_string(),
            "*".to_string(),
            "field_d".to_string(),
        ],
    });

    sort_fields(&mut fields, &inputs_view);

    assert_eq!(fields.len(), 4);
    assert_eq!(fields[0].name, "field_c");
    assert_eq!(fields[1].name, "field_a");
    assert_eq!(fields[2].name, "field_b"); // wildcard picks up remaining
    assert_eq!(fields[3].name, "field_d");
}

#[test]
fn test_sort_fields_with_ui_order_no_wildcard() {
    let mut fields = vec![
        create_dummy_field("field_a"),
        create_dummy_field("field_b"),
        create_dummy_field("field_c"),
    ];

    let inputs_view = Some(stormchaser_model::dsl::InputView {
        ui_order: vec!["field_b".to_string()],
    });

    sort_fields(&mut fields, &inputs_view);

    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].name, "field_b");
    // Remaining appended to the end when there is no wildcard
    assert_eq!(fields[1].name, "field_a");
    assert_eq!(fields[2].name, "field_c");
}
