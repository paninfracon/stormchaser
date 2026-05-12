use crate::utils::{handle_response, require_token};
use anyhow::Result;
use clap::Subcommand;
use reqwest::header::AUTHORIZATION;
use serde_json::json;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

#[derive(Subcommand)]
pub enum ConnectionCommands {
    /// List storage backends
    List,
    /// Create a storage backend
    Create {
        name: String,
        /// The type of storage backend (e.g., s3, oci)
        #[arg(long)]
        connection_type: String,
        /// Path to JSON configuration file
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        default_sfs: bool,
        #[arg(long)]
        description: Option<String>,
        /// Optional AWS role ARN to assume
        #[arg(long)]
        aws_assume_role_arn: Option<String>,
        /// Optional encrypted credentials (password/token)
        #[arg(long)]
        encrypted_credentials: Option<String>,
        /// Validate the connection before creating it
        #[arg(long)]
        test: bool,
    },
    /// Get storage backend details
    Get { id: stormchaser_model::ConnectionId },
    /// Update a storage backend
    Update {
        id: stormchaser_model::ConnectionId,
        #[arg(long)]
        name: Option<String>,
        /// Path to JSON configuration file
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        default_sfs: Option<bool>,
        #[arg(long)]
        description: Option<String>,
        /// Optional AWS role ARN to assume
        #[arg(long)]
        aws_assume_role_arn: Option<String>,
        /// Optional encrypted credentials (password/token)
        #[arg(long)]
        encrypted_credentials: Option<String>,
    },
    /// Delete a storage backend
    Delete { id: stormchaser_model::ConnectionId },
    /// Test a connection payload without saving it
    Test {
        /// The type of connection (e.g., http_api, s3, postgres)
        #[arg(long)]
        connection_type: String,
        /// Path to JSON configuration file
        #[arg(long)]
        config: PathBuf,
        /// Optional AWS role ARN to assume
        #[arg(long)]
        aws_assume_role_arn: Option<String>,
    },
}

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: ConnectionCommands,
) -> Result<()> {
    match command {
        ConnectionCommands::List => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/connections", url))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        ConnectionCommands::Create {
            name,
            connection_type,
            config,
            default_sfs,
            description,
            aws_assume_role_arn,
            encrypted_credentials,
            test,
        } => {
            let config_json: Value = serde_json::from_str(&fs::read_to_string(config)?)?;
            let token = require_token(token)?;

            if test {
                let test_res = http_client
                    .post(format!("{}/api/v1/connections/test", url))
                    .header(AUTHORIZATION, format!("Bearer {}", token))
                    .json(&json!({
                        "connection_type": connection_type,
                        "config": config_json,
                        "aws_assume_role_arn": aws_assume_role_arn,
                    }))
                    .send()
                    .await?;

                if !test_res.status().is_success() {
                    anyhow::bail!("Test request failed: {}", test_res.status());
                }

                let body: Value = test_res.json().await?;
                let success = body
                    .get("success")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let msg = body
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown");

                if !success {
                    anyhow::bail!("Connection test failed: {}. Aborting creation.", msg);
                } else {
                    println!("Connection test passed: {}", msg);
                }
            }

            let res = http_client
                .post(format!("{}/api/v1/connections", url))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .json(&json!({
                    "name": name,
                    "connection_type": connection_type,
                    "config": config_json,
                    "is_default_sfs": default_sfs,
                    "description": description,
                    "aws_assume_role_arn": aws_assume_role_arn,
                    "encrypted_credentials": encrypted_credentials,
                }))
                .send()
                .await?;
            handle_response(res).await?;
        }
        ConnectionCommands::Get { id } => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/connections/{}", url, id))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        ConnectionCommands::Update {
            id,
            name,
            config,
            default_sfs,
            description,
            aws_assume_role_arn,
            encrypted_credentials,
        } => {
            let mut body = json!({});
            if let Some(n) = name {
                body["name"] = json!(n);
            }
            if let Some(c) = config {
                body["config"] = serde_json::from_str(&fs::read_to_string(c)?)?;
            }
            if let Some(d) = default_sfs {
                body["is_default_sfs"] = json!(d);
            }
            if let Some(desc) = description {
                body["description"] = json!(desc);
            }
            if let Some(arn) = aws_assume_role_arn {
                body["aws_assume_role_arn"] = json!(arn);
            }
            if let Some(credentials) = encrypted_credentials {
                body["encrypted_credentials"] = json!(credentials);
            }

            let token = require_token(token)?;
            let res = http_client
                .patch(format!("{}/api/v1/connections/{}", url, id))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .json(&body)
                .send()
                .await?;
            handle_response(res).await?;
        }
        ConnectionCommands::Delete { id } => {
            let token = require_token(token)?;
            let res = http_client
                .delete(format!("{}/api/v1/connections/{}", url, id))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        ConnectionCommands::Test {
            connection_type,
            config,
            aws_assume_role_arn,
        } => {
            let config_json: Value = serde_json::from_str(&fs::read_to_string(config)?)?;
            let token = require_token(token)?;
            let res = http_client
                .post(format!("{}/api/v1/connections/test", url))
                .header(AUTHORIZATION, format!("Bearer {}", token))
                .json(&json!({
                    "connection_type": connection_type,
                    "config": config_json,
                    "aws_assume_role_arn": aws_assume_role_arn,
                }))
                .send()
                .await?;
            handle_response(res).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest_middleware::ClientBuilder;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_storage_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = ConnectionCommands::List;

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_storage_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/connections"))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "created"})))
            .mount(&server)
            .await;

        use std::io::Write;
        use tempfile::NamedTempFile;
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "{{\"bucket\":\"my-bucket\"}}").unwrap();

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = ConnectionCommands::Create {
            name: "test-storage".to_string(),
            connection_type: "s3".to_string(),
            config: temp_file.path().to_path_buf(),
            default_sfs: true,
            description: None,
            aws_assume_role_arn: None,
            encrypted_credentials: None,
            test: false,
        };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_storage_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::ConnectionId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/connections/{}", id)))
            .and(header(AUTHORIZATION, "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = ClientBuilder::new(reqwest::Client::new()).build();
        let cmd = ConnectionCommands::Delete { id };

        let result = handle(&server.uri(), Some("test-token"), &client, cmd).await;
        assert!(result.is_ok());
    }
}
