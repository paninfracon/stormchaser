use serde_json::json;

fn main() {
    let schema = json!({
        "type": "object",
        "properties": {
            "github_username": { "type": "string" }
        },
        "required": ["github_username"]
    });

    let inputs = json!({});
    let compiled = jsonschema::validator_for(&schema).unwrap();
    if let Err(errors) = compiled.validate(&inputs) {
        println!("{}", errors);
    }
}
