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
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({ "dsl": dsl, "inputs": inputs }))
        .send()
        .await?;

    handle_run_response(http_client, url, token, res, tail, watch).await
}
