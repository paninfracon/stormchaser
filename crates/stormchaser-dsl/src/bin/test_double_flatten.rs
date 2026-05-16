use serde_json::json;

fn main() {
    let source = std::fs::read_to_string("tests/demo/complex-inputs.storm").unwrap();
    let wf = stormchaser_dsl::StormchaserParser::new()
        .parse(&source)
        .unwrap();
    let raw_schema = wf.inputs_schema.unwrap();

    // First hydration (empty inputs)
    let inputs1 = json!({});
    let hydrated1 = stormchaser_model::schema_gen::flatten_schema_for_ui(&raw_schema, &inputs1);

    // Second hydration (developer inputs)
    let inputs2 = json!({ "role": "developer" });
    let hydrated2 = stormchaser_model::schema_gen::flatten_schema_for_ui(&hydrated1, &inputs2);

    let props = hydrated2.get("properties").unwrap().as_object().unwrap();
    println!("Keys:");
    for (k, _) in props {
        println!("{}", k);
    }
}
