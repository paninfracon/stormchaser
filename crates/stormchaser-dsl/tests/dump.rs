use stormchaser_dsl::StormchaserParser;
#[test]
fn dump_schema() {
    let dsl = std::fs::read_to_string("../../tests/demo/complex-inputs.storm").unwrap();
    let workflow = StormchaserParser.parse(&dsl).unwrap();
    println!(
        "INPUTS SCHEMA:\n{}",
        serde_json::to_string_pretty(&workflow.inputs_schema).unwrap()
    );
}
