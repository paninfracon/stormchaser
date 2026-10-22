use crate::utils::{follow_run, parse_inputs};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use stormchaser_client::Client;
use stormchaser_model::api::DirectRunRequest;

pub async fn handle(
    client: &Client,
    file: PathBuf,
    input: Vec<String>,
    tail: bool,
    watch: bool,
) -> Result<()> {
    let dsl = fs::read_to_string(file)?;
    let inputs = parse_inputs(input)?;

    let run = client
        .run_direct(&DirectRunRequest {
            dsl,
            inputs: inputs.into(),
        })
        .await;

    follow_run(client, run, tail, watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_run_handle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs/direct"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_json(json!({
                "dsl": "workflow \"test\" {}\n",
                "inputs": { "param": "value" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "run_id": "12345678-1234-1234-1234-123456789012",
                "status": "queued"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "workflow \"test\" {{}}").unwrap();

        let input = vec!["param=value".to_string()];

        let result = handle(&client, temp_file.path().to_path_buf(), input, false, false).await;

        result.unwrap();
    }
}
