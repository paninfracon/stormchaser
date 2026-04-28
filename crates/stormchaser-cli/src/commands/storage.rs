use crate::utils::{handle_response, require_token};
use anyhow::Result;
use clap::Subcommand;
use serde_json::json;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Subcommand)]
pub enum StorageCommands {
    /// List storage backends
    List,
    /// Create a storage backend
    Create {
        name: String,
        /// The type of storage backend (e.g., s3, oci)
        #[arg(long)]
        backend_type: String,
        /// Path to JSON configuration file
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        default_sfs: bool,
        #[arg(long)]
        description: Option<String>,
    },
    /// Get storage backend details
    Get { id: Uuid },
    /// Update a storage backend
    Update {
        id: Uuid,
        #[arg(long)]
        name: Option<String>,
        /// Path to JSON configuration file
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        default_sfs: Option<bool>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Delete a storage backend
    Delete { id: Uuid },
}

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: StorageCommands,
) -> Result<()> {
    match command {
        StorageCommands::List => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/storage-backends", url))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        StorageCommands::Create {
            name,
            backend_type,
            config,
            default_sfs,
            description,
        } => {
            let config_json: Value = serde_json::from_str(&fs::read_to_string(config)?)?;
            let token = require_token(token)?;
            let res = http_client
                .post(format!("{}/api/v1/storage-backends", url))
                .header("Authorization", format!("Bearer {}", token))
                .json(&json!({
                    "name": name,
                    "backend_type": backend_type,
                    "config": config_json,
                    "is_default_sfs": default_sfs,
                    "description": description,
                }))
                .send()
                .await?;
            handle_response(res).await?;
        }
        StorageCommands::Get { id } => {
            let token = require_token(token)?;
            let res = http_client
                .get(format!("{}/api/v1/storage-backends/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
        StorageCommands::Update {
            id,
            name,
            config,
            default_sfs,
            description,
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

            let token = require_token(token)?;
            let res = http_client
                .patch(format!("{}/api/v1/storage-backends/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .json(&body)
                .send()
                .await?;
            handle_response(res).await?;
        }
        StorageCommands::Delete { id } => {
            let token = require_token(token)?;
            let res = http_client
                .delete(format!("{}/api/v1/storage-backends/{}", url, id))
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await?;
            handle_response(res).await?;
        }
    }
    Ok(())
}
