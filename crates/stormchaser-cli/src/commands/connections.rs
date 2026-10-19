use crate::utils::print_json;
use anyhow::{Context, Result};
use clap::Subcommand;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use stormchaser_client::Client;
use stormchaser_model::api::{
    CreateStorageBackendRequest, TestConnectionRequest, UpdateStorageBackendRequest,
};
use stormchaser_model::connections::ConnectionType;

#[derive(Subcommand)]
pub enum ConnectionCommands {
    /// List configured connections
    List,
    /// Create a connection
    Create {
        name: String,
        /// The type of connection (e.g., s3, oci, postgres, http_api)
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
        credentials: Option<String>,
        /// Validate the connection before creating it
        #[arg(long)]
        test: bool,
    },
    /// Get connection details
    Get { id: stormchaser_model::ConnectionId },
    /// Update a connection
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
        credentials: Option<String>,
    },
    /// Delete a connection
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

/// A connection type as the API spells it (`s3`, `http_api`).
fn parse_connection_type(text: &str) -> Result<ConnectionType> {
    serde_json::from_value(Value::String(text.to_string()))
        .with_context(|| format!("Unknown connection type '{}'", text))
}

fn read_config(path: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}

pub async fn handle(client: &Client, command: ConnectionCommands) -> Result<()> {
    match command {
        ConnectionCommands::List => {
            print_json(&client.list_connections().await?)?;
        }
        ConnectionCommands::Create {
            name,
            connection_type,
            config,
            default_sfs,
            description,
            aws_assume_role_arn,
            credentials,
            test,
        } => {
            let config = read_config(&config)?;
            let connection_type = parse_connection_type(&connection_type)?;

            if test {
                let result = client
                    .test_connection(&TestConnectionRequest {
                        connection_type: connection_type.clone(),
                        config: config.clone(),
                        aws_assume_role_arn: aws_assume_role_arn.clone(),
                    })
                    .await;
                let result = match result {
                    Ok(result) => result,
                    Err(e) => match e.status() {
                        Some(status) => anyhow::bail!("Test request failed: {}", status),
                        None => return Err(e.into()),
                    },
                };
                // Can quote what the tested server answered
                let msg = crate::utils::strip_control(&result.message);

                if !result.success {
                    anyhow::bail!("Connection test failed: {}. Aborting creation.", msg);
                } else {
                    println!("Connection test passed: {}", msg);
                }
            }

            let created = client
                .create_connection(&CreateStorageBackendRequest {
                    name,
                    description,
                    connection_type,
                    config,
                    aws_assume_role_arn,
                    credentials,
                    is_default_sfs: default_sfs,
                })
                .await?;
            print_json(&created)?;
        }
        ConnectionCommands::Get { id } => {
            print_json(&client.get_connection(id).await?)?;
        }
        ConnectionCommands::Update {
            id,
            name,
            config,
            default_sfs,
            description,
            aws_assume_role_arn,
            credentials,
        } => {
            let config = config.as_deref().map(read_config).transpose()?;
            client
                .update_connection(
                    id,
                    &UpdateStorageBackendRequest {
                        name,
                        description,
                        connection_type: None,
                        config,
                        aws_assume_role_arn,
                        credentials,
                        is_default_sfs: default_sfs,
                    },
                )
                .await?;
            println!("Success");
        }
        ConnectionCommands::Delete { id } => {
            client.delete_connection(id).await?;
            println!("Success");
        }
        ConnectionCommands::Test {
            connection_type,
            config,
            aws_assume_role_arn,
        } => {
            let result = client
                .test_connection(&TestConnectionRequest {
                    connection_type: parse_connection_type(&connection_type)?,
                    config: read_config(&config)?,
                    aws_assume_role_arn,
                })
                .await?;
            print_json(&result)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_storage_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/connections"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = ConnectionCommands::List;

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_storage_create() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/connections"))
            .and(header("authorization", "Bearer test-token"))
            .and(body_partial_json(json!({
                "connection_type": "s3",
                "config": { "bucket": "my-bucket" },
                "is_default_sfs": true
            })))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"id": uuid::Uuid::new_v4()})),
            )
            .expect(1)
            .mount(&server)
            .await;

        use std::io::Write;
        use tempfile::NamedTempFile;
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "{{\"bucket\":\"my-bucket\"}}").unwrap();

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = ConnectionCommands::Create {
            name: "test-storage".to_string(),
            connection_type: "s3".to_string(),
            config: temp_file.path().to_path_buf(),
            default_sfs: true,
            description: None,
            aws_assume_role_arn: None,
            credentials: None,
            test: false,
        };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[tokio::test]
    async fn test_storage_delete() {
        let server = MockServer::start().await;
        let id = stormchaser_model::ConnectionId::new_v4();
        Mock::given(method("DELETE"))
            .and(path(format!("/api/v1/connections/{}", id)))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "deleted"})))
            .mount(&server)
            .await;

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = ConnectionCommands::Delete { id };

        let result = handle(&client, cmd).await;
        result.unwrap();
    }

    #[test]
    fn an_unknown_connection_type_is_refused() {
        assert_eq!(
            parse_connection_type("http_api").unwrap(),
            ConnectionType::HttpApi
        );
        parse_connection_type("ftp").unwrap_err();
    }

    #[tokio::test]
    async fn a_failed_test_stops_the_creation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/connections/test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": false,
                "message": "bucket not found"
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/connections"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config.json");
        std::fs::write(&config, r#"{"bucket":"b"}"#).unwrap();

        let client = Client::builder(server.uri())
            .token("test-token")
            .build()
            .unwrap();
        let cmd = ConnectionCommands::Create {
            name: "s".to_string(),
            connection_type: "s3".to_string(),
            config,
            default_sfs: false,
            description: None,
            aws_assume_role_arn: None,
            credentials: None,
            test: true,
        };

        let err = handle(&client, cmd).await.unwrap_err();
        assert!(err.to_string().contains("bucket not found"), "{err}");
    }
}
