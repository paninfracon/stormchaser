use serde_json::json;
use stormchaser_dsl::StormchaserParser;

fn main() {
    let source = std::fs::read_to_string("tests/demo/complex-inputs.storm").unwrap();
    let wf = StormchaserParser::new().parse(&source).unwrap();
    let schema = wf.inputs_schema.unwrap();
    println!(
        "Schema BEFORE: {}",
        serde_json::to_string_pretty(&schema).unwrap()
    );

    let inputs = json!({ "role": "developer" });
    let flat = stormchaser_model::schema_gen::flatten_schema_for_ui(&schema, &inputs);

    println!(
        "Schema AFTER: {}",
        serde_json::to_string_pretty(&flat).unwrap()
    );
}
