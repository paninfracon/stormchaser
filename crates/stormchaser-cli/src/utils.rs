use anyhow::{Context, Result};
use eventsource_stream::Eventsource;
use futures::stream::StreamExt;
use serde_json::json;
use serde_json::Value;

pub fn parse_key_val_list(list: Vec<String>) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for i in list {
        if let Some((key, value)) = i.split_once('=') {
            // Try to parse as JSON first (for numbers, bools), fallback to string
            let val = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
            map.insert(key.to_string(), val);
        }
    }
    map
}

pub async fn handle_response(res: reqwest::Response) -> Result<()> {
    let status = res.status();
    if status.is_success() {
        let body = res.text().await?;
        if !body.is_empty() {
            if let Ok(val) = serde_json::from_str::<Value>(&body) {
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                println!("{}", body);
            }
        } else {
            println!("Success ({})", status);
        }
    } else {
        let error_text = res.text().await.unwrap_or_default();
        eprintln!("Error ({}): {}", status, error_text);
        std::process::exit(1);
    }
    Ok(())
}

pub async fn stream_run_logs(
    http_client: &reqwest_middleware::ClientWithMiddleware,
    cli_url: &str,
    token: &str,
    run_id: stormchaser_model::RunId,
) -> Result<()> {
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/logs/stream", cli_url, run_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        handle_response(res).await?;
        return Ok(());
    }

    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}", event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

pub async fn stream_run_status(
    http_client: &reqwest_middleware::ClientWithMiddleware,
    cli_url: &str,
    token: &str,
    run_id: stormchaser_model::RunId,
) -> Result<()> {
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/status/stream", cli_url, run_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        handle_response(res).await?;
        return Ok(());
    }

    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}: {}", event.event, event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

pub fn require_token(token: Option<&str>) -> Result<&str> {
    token.context("Authentication token required (use --token or STORMCHASER_TOKEN env var)")
}

pub async fn handle_run_response(
    http_client: &reqwest_middleware::ClientWithMiddleware,
    url: &str,
    token: &str,
    res: reqwest::Response,
    tail: bool,
    watch: bool,
) -> Result<()> {
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() {
        if let Ok(val) = serde_json::from_str::<Value>(&body) {
            println!("{}", serde_json::to_string_pretty(&val)?);
            if tail {
                if let Some(id_str) = val.get("run_id").and_then(|i| i.as_str()) {
                    if let Ok(run_id) = id_str.parse::<stormchaser_model::RunId>() {
                        println!("Streaming logs for run {}...", run_id);
                        stream_run_logs(http_client, url, token, run_id).await?;
                    }
                }
            } else if watch {
                if let Some(id_str) = val.get("run_id").and_then(|i| i.as_str()) {
                    if let Ok(run_id) = id_str.parse::<stormchaser_model::RunId>() {
                        println!("Watching status for run {}...", run_id);
                        stream_run_status(http_client, url, token, run_id).await?;
                    }
                }
            }
        } else {
            println!("{}", body);
        }
    } else {
        eprintln!("Error ({}): {}", status, body);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_val_list_strings() {
        let input = vec!["key1=value1".to_string(), "key2=value2".to_string()];
        let map = parse_key_val_list(input);
        assert_eq!(map.get("key1").unwrap().as_str().unwrap(), "value1");
        assert_eq!(map.get("key2").unwrap().as_str().unwrap(), "value2");
    }

    #[test]
    fn test_parse_key_val_list_json_types() {
        let input = vec!["num=42".to_string(), "bool=true".to_string()];
        let map = parse_key_val_list(input);
        assert_eq!(map.get("num").unwrap().as_i64().unwrap(), 42);
        assert!(map.get("bool").unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_require_token_missing() {
        let err = require_token(None).unwrap_err();
        assert!(err.to_string().contains("Authentication token required"));
    }

    #[test]
    fn test_require_token_present() {
        let token = require_token(Some("my-token")).unwrap();
        assert_eq!(token, "my-token");
    }
}
