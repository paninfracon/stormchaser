use stormchaser_dsl::StormchaserParser;

fn main() {
    let source = r#"
workflow "test" {
  inputs {
    ui_order = ["username", "role", "github_username", "region", "cluster", "dry_run", "age"]
    username = string(pattern("^[a-zA-Z0-9_-]{3,16}$"))
    age = integer(minimum(18), maximum(100))
    region = string(enum("${queries.regions_query}"))
    cluster = string(enum("${queries.clusters_query}"))
    dry_run = boolean(default(true))
    role = string(enum("${queries.roles_query}"))

    allOf = [
      {
        if = { properties = { role = { const = "developer" } } }
        then = { properties = { github_username = { type = "string" } }, required = ["github_username"] }
      }
    ]
    required = ["username", "role", "region", "cluster"]
  }
}
"#;
    let wf = StormchaserParser::new().parse(source).unwrap();
    println!(
        "{}",
        serde_json::to_string_pretty(&wf.inputs_schema).unwrap()
    );
}
