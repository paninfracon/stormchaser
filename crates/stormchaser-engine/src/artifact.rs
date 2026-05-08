use anyhow::{Context, Result};
use serde_json::Value;
use std::time::Duration;
use stormchaser_model::dsl::Artifact;
use stormchaser_model::storage::{BackendType, StorageBackend};
use uuid::Uuid;

/// Generate parking instructions.
pub async fn generate_parking_instructions(
    backend: &StorageBackend,
    run_id: Uuid,
    storage_name: &str,
    artifact: &Artifact,
) -> Result<Value> {
    match backend.backend_type {
        BackendType::S3 => {
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
                "backend_type": "s3",
                "put_url": put_url,
                "path": artifact.path,
                "retention": artifact.retention,
                "remote_path": artifact_key,
            }))
        }
        BackendType::Oci => {
            let registry = backend.config["registry"]
                .as_str()
                .context("Missing registry in OCI backend config")?;
            let username = backend.config["username"].as_str();
            let password = backend.config["password"].as_str();

            let remote_path = format!("{}/{}/{}:{}", registry, run_id, storage_name, artifact.name);

            let mut payload = serde_json::json!({
                "backend_type": "oci",
                "registry": registry,
                "remote_path": remote_path,
                "path": artifact.path,
                "retention": artifact.retention,
            });

            if let (Some(u), Some(p)) = (username, password) {
                if let Some(map) = payload.as_object_mut() {
                    map.insert("username".to_string(), Value::String(u.to_string()));
                    map.insert("password".to_string(), Value::String(p.to_string()));
                }
            }

            Ok(payload)
        }
        _ => anyhow::bail!(
            "Unsupported artifact backend type: {:?}",
            backend.backend_type
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::BackendId;

    #[tokio::test]
    async fn test_generate_parking_instructions_oci() {
        let backend = StorageBackend {
            id: BackendId::new_v4(),
            name: "oci-registry".into(),
            description: None,
            backend_type: BackendType::Oci,
            config: json!({
                "registry": "registry.paninfracon.net",
                "username": "user",
                "password": "pass"
            }),
            aws_assume_role_arn: None,
            is_default_sfs: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
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

        assert_eq!(instructions["backend_type"], "oci");
        assert_eq!(instructions["registry"], "registry.paninfracon.net");
        assert_eq!(instructions["username"], "user");
        assert_eq!(instructions["password"], "pass");
        assert!(instructions["remote_path"]
            .as_str()
            .unwrap()
            .contains("registry.paninfracon.net"));
    }

    #[tokio::test]
    async fn test_generate_parking_instructions_unsupported() {
        let backend = StorageBackend {
            id: BackendId::new_v4(),
            name: "fs-backend".into(),
            description: None,
            backend_type: BackendType::Jfrog,
            config: json!({}),
            aws_assume_role_arn: None,
            is_default_sfs: false,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
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

        assert!(result.is_err());
    }
}
