use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;
use stormchaser_model::connections::{Connection, ConnectionType};
use stormchaser_model::dsl::Artifact;
use uuid::Uuid;

/// Generate parking instructions.
pub async fn generate_parking_instructions(
    backend: &Connection,
    run_id: Uuid,
    storage_name: &str,
    artifact: &Artifact,
) -> Result<Value> {
    match backend.connection_type {
        ConnectionType::S3 => {
            let client = crate::s3::get_s3_client(backend).await?;
            let bucket = backend.config["bucket"]
                .as_str()
                .context("Missing bucket in SFS backend config")?;

            let artifact_key = format!("artifacts/{}/{}/{}", run_id, storage_name, artifact.name);
            let expires = Duration::from_secs(3600); // 1 hour

            let put_url =
                crate::s3::generate_presigned_url(&client, bucket, &artifact_key, true, expires)
                    .await?;

            Ok(serde_json::json!({
                "connection_type": ConnectionType::S3,
                "put_url": put_url,
                "path": artifact.path,
                "retention": artifact.retention,
                "remote_path": artifact_key,
            }))
        }
        ConnectionType::Oci => {
            let location = crate::registry_token::RegistryLocation::of(backend)?;
            let repository = location.artifact_repository(run_id, storage_name);
            let remote_path = format!("{}/{}:{}", location.host, repository, artifact.name);
            let has_credentials = backend.config["username"].as_str().is_some()
                && backend.config["password"].as_str().is_some();

            // The registry's username and password stay in the engine. A runner asks for
            // a short-lived token for this run's repository when the step is ready to push.
            Ok(serde_json::json!({
                "connection_type": "oci",
                "registry": location.host,
                "remote_path": remote_path,
                "path": artifact.path,
                "retention": artifact.retention,
                "plain_http": location.plain_http,
                "needs_auth": has_credentials,
            }))
        }
        _ => anyhow::bail!(
            "Unsupported artifact backend type: {:?}",
            backend.connection_type
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use stormchaser_model::ConnectionId;

    #[tokio::test]
    async fn test_generate_parking_instructions_oci() {
        let backend = Connection {
            id: ConnectionId::new_v4(),
            name: "oci-registry".into(),
            description: None,
            connection_type: ConnectionType::Oci,
            config: json!({
                "registry": "registry.paninfracon.net",
                "username": "user",
                "password": "pass"
            }),
            aws_assume_role_arn: None,
            is_default_sfs: false,
            credentials: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
        };

        let run_id = Uuid::new_v4();
        let artifact = Artifact {
            name: "build-bin".into(),
            path: "bin/".into(),
            retention: "7d".into(),
        };

        let instructions = generate_parking_instructions(&backend, run_id, "my-storage", &artifact)
            .await
            .unwrap();

        assert_eq!(instructions["connection_type"], "oci");
        assert_eq!(instructions["registry"], "registry.paninfracon.net");
        // The stored credentials stay in the engine
        assert_eq!(instructions["needs_auth"], true);
        assert!(instructions.get("username").is_none());
        assert!(!instructions.to_string().contains("pass"));
        assert!(instructions["remote_path"]
            .as_str()
            .unwrap()
            .contains("registry.paninfracon.net"));
    }

    #[tokio::test]
    async fn test_generate_parking_instructions_unsupported() {
        let backend = Connection {
            id: ConnectionId::new_v4(),
            name: "fs-backend".into(),
            description: None,
            connection_type: ConnectionType::Jfrog,
            config: json!({}),
            aws_assume_role_arn: None,
            is_default_sfs: false,
            credentials: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
        };

        let result = generate_parking_instructions(
            &backend,
            Uuid::new_v4(),
            "st",
            &Artifact {
                name: "n".into(),
                path: "p".into(),
                retention: "1d".into(),
            },
        )
        .await;

        result.unwrap_err();
    }
}
