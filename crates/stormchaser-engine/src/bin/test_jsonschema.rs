use stormchaser_dsl::StormchaserParser;

fn main() {
    let source = std::fs::read_to_string("tests/hello-world.storm").unwrap();
    let wf = StormchaserParser::new().parse(&source).unwrap();
    let schema = wf.inputs_schema.unwrap();
    match jsonschema::validator_for(&schema) {
        Ok(_) => println!(
            "Valid schema: {}",
            serde_json::to_string_pretty(&schema).unwrap()
        ),
        Err(e) => println!("Invalid schema: {}", e),
    }
}
