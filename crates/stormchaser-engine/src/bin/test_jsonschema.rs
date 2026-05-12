use stormchaser_dsl::StormchaserParser;

fn main() {
    let source = std::fs::read_to_string("tests/demo/complex-inputs.storm").unwrap();
    let wf = StormchaserParser::new().parse(&source).unwrap();
    let schema = wf.inputs_schema.unwrap();
    match jsonschema::validator_for(&schema) {
        Ok(_) => println!("Valid schema"),
        Err(e) => println!("Invalid schema: {}", e),
    }
}
