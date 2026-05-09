use crate::utils::{handle_run_response, parse_key_val_list, require_token};
use anyhow::Result;
use serde_json::json;
use std::fs;
use std::path::PathBuf;

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    file: PathBuf,
    input: Vec<String>,
    tail: bool,
    watch: bool,
) -> Result<()> {
    let dsl = fs::read_to_string(file)?;
    let inputs = parse_key_val_list(input);
    let token = require_token(token)?;

    let res = http_client
        .post(format!("{}/api/v1/runs/direct", url))
        .header(reqwest::header::AUTHORIZATION, format!("Bearer {}", token))
        .json(&json!({ "dsl": dsl, "inputs": inputs }))
        .send()
        .await?;

    handle_run_response(http_client, url, token, res, tail, watch).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use std::io::Write;
    use stormchaser_model::RunStatus;
    use tempfile::NamedTempFile;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_run_handle() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/runs/direct"))
            .and(header(reqwest::header::AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "12345678-1234-1234-1234-123456789012",
                "status": RunStatus::Queued
            })))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();

        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "workflow \"test\" {{}}").unwrap();

        let input = vec!["param=value".to_string()];

        let result = handle(
            &server.uri(),
            Some("test-token"),
            &client,
            temp_file.path().to_path_buf(),
            input,
            false,
            false,
        )
        .await;

        assert!(result.is_ok());
    }
}
