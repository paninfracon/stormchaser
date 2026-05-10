fn main() {
    let schema = serde_json::json!({"type": "string"});
    let validator = jsonschema::options().build(&schema).unwrap();
    println!("Works");
}
