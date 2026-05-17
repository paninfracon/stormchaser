use anyhow::{Context, Result};
use schemars::schema::Schema;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use stormchaser_dsl::StormchaserParser;
use stormchaser_model::schema_gen::{apply_step_extensibility, generate_dsl_schema};

/// CLI command to lint a workflow file against the JSON schema.
#[derive(clap::Parser)]
pub struct LintCommand {
    /// Path to the .storm workflow file to lint
    pub file: Option<PathBuf>,

    /// Additional step schemas to include in validation.
    /// Format: TYPE=PATH
    /// Where PATH can be a local file (e.g. MyStep=schema.json)
    /// Or a local git repository path using git-local scheme
    /// (e.g. MyStep=git-local:///path/to/repo?ref=main&file=schema.json)
    #[arg(long, value_parser = parse_step_schema)]
    pub step_schema: Vec<(String, String)>,

    /// Fetch the base schema from the remote server instead of using the local schema
    #[arg(long)]
    pub remote: bool,

    /// Download and store the server schemas locally for offline validation
    #[arg(long)]
    pub prepare: bool,
}

/// Parses the `TYPE=PATH` argument into a tuple for `step_schema`.
fn parse_step_schema(s: &str) -> Result<(String, String)> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid step-schema format. Expected TYPE=PATH");
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
}

/// Handles the `lint` command logic.
pub async fn handle(
    url: &str,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: LintCommand,
) -> Result<()> {
    if command.prepare && command.file.is_none() {
        let req_url = format!("{}/api/v1/schema", url);
        let resp = http_client
            .get(&req_url)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("Failed to fetch remote schema from {}", req_url))?;
        let json: Value = resp.json().await?;
        std::fs::write(
            ".stormchaser-schema.json",
            serde_json::to_string_pretty(&json)?,
        )?;
        println!("✓ Downloaded and saved remote schema to .stormchaser-schema.json");
        return Ok(());
    }

    let file_path = command
        .file
        .clone()
        .context("No workflow file provided to lint")?;
    let content = std::fs::read_to_string(&file_path)
        .with_context(|| format!("Failed to read file: {:?}", file_path))?;

    // First, parse the HCL to ensure it's syntactically valid and maps to Workflow
    let parser = StormchaserParser::new();
    let workflow = parser
        .parse(&content)
        .with_context(|| format!("Failed to parse HCL in {:?}", file_path))?;

    // Serialize to JSON Value for schema validation
    let instance = serde_json::to_value(&workflow)?;

    // Generate base schema
    let mut root_schema = resolve_base_schema(&command, url, http_client).await?;

    let mut spec_schemas = HashMap::new();

    // Load additional schemas from arguments
    for (type_name, path) in command.step_schema {
        let schema_json = load_schema(&path)?;
        let schema_obj: Schema = serde_json::from_value(schema_json)
            .with_context(|| format!("Failed to parse schema for type {}", type_name))?;
        spec_schemas.insert(type_name, schema_obj);
    }

    // Apply extensibility logic
    apply_step_extensibility(&mut root_schema, &spec_schemas);

    // Compile and validate
    let schema_value = serde_json::to_value(&root_schema)?;
    let validator = jsonschema::Validator::new(&schema_value)
        .map_err(|e| anyhow::anyhow!("Failed to compile JSON schema: {}", e))?;

    if !validator.is_valid(&instance) {
        println!("Validation failed for {:?}", file_path);
        for error in validator.iter_errors(&instance) {
            println!("- {}: {}", error.instance_path(), error);
        }
        anyhow::bail!("Workflow failed schema validation");
    }

    println!("✓ Workflow successfully validated against schema.");
    Ok(())
}

async fn resolve_base_schema(
    command: &LintCommand,
    url: &str,
    http_client: &reqwest_middleware::ClientWithMiddleware,
) -> Result<schemars::schema::RootSchema> {
    if command.remote || command.prepare {
        let req_url = format!("{}/api/v1/schema", url);
        let resp = http_client
            .get(&req_url)
            .send()
            .await?
            .error_for_status()
            .with_context(|| format!("Failed to fetch remote schema from {}", req_url))?;
        let json: Value = resp.json().await?;

        if command.prepare {
            std::fs::write(
                ".stormchaser-schema.json",
                serde_json::to_string_pretty(&json)?,
            )?;
            println!("✓ Downloaded and saved remote schema to .stormchaser-schema.json");
        }

        serde_json::from_value(json).context("Failed to parse remote schema")
    } else if Path::new(".stormchaser-schema.json").exists() {
        let content = std::fs::read_to_string(".stormchaser-schema.json")
            .context("Failed to read .stormchaser-schema.json")?;
        serde_json::from_str(&content).context("Failed to parse .stormchaser-schema.json")
    } else {
        Ok(generate_dsl_schema())
    }
}

/// Loads a JSON schema from a local file or directly from a local git repository without checkout.
fn load_schema(path: &str) -> Result<Value> {
    if path.starts_with("git-local://") {
        let url = url::Url::parse(path)?;
        let repo_path = url.path();

        let mut ref_name = "HEAD".to_string();
        let mut file_path = String::new();

        for (k, v) in url.query_pairs() {
            if k == "ref" {
                ref_name = v.into_owned();
            } else if k == "file" {
                file_path = v.into_owned();
            }
        }

        if file_path.is_empty() {
            anyhow::bail!("Missing 'file' query parameter in git-local URL");
        }

        let repo = git2::Repository::open(repo_path)
            .with_context(|| format!("Failed to open git repository at {}", repo_path))?;

        let obj = repo
            .revparse_single(&ref_name)
            .with_context(|| format!("Failed to find ref {} in {}", ref_name, repo_path))?;

        let tree = obj
            .peel_to_tree()
            .with_context(|| format!("Failed to peel ref {} to a tree", ref_name))?;

        let entry = tree
            .get_path(std::path::Path::new(&file_path))
            .with_context(|| format!("Failed to find file {} at ref {}", file_path, ref_name))?;

        let blob = entry.to_object(&repo)?.peel_to_blob()?;
        let content = blob.content();

        let json: Value = serde_json::from_slice(content)
            .with_context(|| format!("Failed to parse JSON from {} in git repo", file_path))?;

        Ok(json)
    } else {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read schema file: {}", path))?;
        let json: Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse JSON from file: {}", path))?;
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use std::io::Write;
    use tempfile::NamedTempFile;

    static LINT_MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn test_parse_step_schema_valid() {
        let (t, p) = parse_step_schema("CustomType=local.json").unwrap();
        assert_eq!(t, "CustomType");
        assert_eq!(p, "local.json");

        let (t2, p2) =
            parse_step_schema("GitType=git-local:///foo?ref=main&file=schema.json").unwrap();
        assert_eq!(t2, "GitType");
        assert_eq!(p2, "git-local:///foo?ref=main&file=schema.json");
    }

    #[test]
    fn test_parse_step_schema_invalid() {
        parse_step_schema("InvalidFormatWithoutEquals").unwrap_err();
    }

    #[tokio::test]
    async fn test_lint_handle_valid_workflow() -> Result<()> {
        let _guard = LINT_MUTEX.lock().await;
        let mut file = NamedTempFile::new()?;
        writeln!(
            file,
            r#"
stormchaser_dsl_version = "0.1"
workflow "test_workflow" {{
  description = "A valid workflow"
  steps {{
    step "test_step" "RunContainer" {{
      image = "alpine"
    }}
  }}
}}
"#
        )?;

        let cmd = LintCommand {
            file: Some(file.path().to_path_buf()),
            step_schema: vec![],
            remote: false,
            prepare: false,
        };

        let http_client = ClientBuilder::new(reqwest::Client::new()).build();
        handle("http://localhost", &http_client, cmd)
            .await
            .expect("Valid workflow should lint successfully");
        Ok(())
    }

    #[tokio::test]
    async fn test_lint_handle_invalid_spec() -> Result<()> {
        let _guard = LINT_MUTEX.lock().await;
        let _ = std::fs::remove_file(".stormchaser-schema.json");
        let mut file = NamedTempFile::new()?;
        writeln!(
            file,
            r#"
stormchaser_dsl_version = "0.1"
workflow "test_workflow" {{
  description = "A workflow with an invalid spec type"
  steps {{
    step "test_step" "RunContainer" {{
      image = 123
    }}
  }}
}}
"#
        )?;

        let cmd = LintCommand {
            file: Some(file.path().to_path_buf()),
            step_schema: vec![],
            remote: false,
            prepare: false,
        };

        let http_client = ClientBuilder::new(reqwest::Client::new()).build();
        let result = handle("http://localhost", &http_client, cmd).await;
        assert!(
            result.is_err(),
            "Expected invalid schema to fail validation"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("schema validation"),
            "Error should indicate schema validation failure"
        );
        Ok(())
    }

    #[test]
    fn test_load_schema_local_file() -> Result<()> {
        let mut file = NamedTempFile::new()?;
        writeln!(file, r#"{{"type":"object"}}"#)?;
        let val = load_schema(file.path().to_str().unwrap())?;
        assert_eq!(val["type"], "object");
        Ok(())
    }

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_lint_handle_prepare() -> Result<()> {
        let _guard = LINT_MUTEX.lock().await;
        let _ = std::fs::remove_file(".stormchaser-schema.json");
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/schema"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"type": "object"})),
            )
            .mount(&server)
            .await;

        let cmd = LintCommand {
            file: None,
            step_schema: vec![],
            remote: false,
            prepare: true,
        };

        let http_client = ClientBuilder::new(reqwest::Client::new()).build();
        handle(&server.uri(), &http_client, cmd).await?;

        let saved = std::fs::read_to_string(".stormchaser-schema.json")?;
        assert!(saved.contains("\"type\": \"object\""));
        std::fs::remove_file(".stormchaser-schema.json")?;

        Ok(())
    }
}
