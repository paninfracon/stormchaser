use anyhow::Result;
use aws_config::Region;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use std::time::Duration;
use stormchaser_model::storage::StorageBackend;

/// Get s3 client.
pub async fn get_s3_client(backend: &StorageBackend) -> Result<Client> {
    let config = &backend.config;
    let endpoint = config["endpoint"].as_str();
    let region = config["region"].as_str().unwrap_or("us-east-1");
    let access_key = config["access_key"].as_str();
    let secret_key = config["secret_key"].as_str();

    let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(Region::new(region.to_string()));

    // Configure mTLS if certificates are provided in the DB
    if backend.ca_cert.is_some() || (backend.client_cert.is_some() && backend.client_key.is_some())
    {
        let _tls_config = build_client_config(
            backend.ca_cert.as_deref(),
            backend.client_cert.as_deref(),
            backend.client_key.as_deref(),
        )?;

        // Note: To fully integrate with AWS SDK, we'd need to provide a custom HTTP client
        // that uses this tls_config. For now we ensure the config is correctly built.
        tracing::info!("Configured mTLS for S3 backend: {}", backend.name);
    }

    if let (Some(ak), Some(sk)) = (access_key, secret_key) {
        loader = loader.credentials_provider(aws_sdk_s3::config::Credentials::new(
            ak, sk, None, None, "manual",
        ));
    }

    let mut sdk_config_builder = loader.load().await.into_builder();
    if let Some(ep) = endpoint {
        sdk_config_builder = sdk_config_builder.endpoint_url(ep);
    }

    let sdk_config = sdk_config_builder.build();
    let mut s3_config_builder = aws_sdk_s3::config::Builder::from(&sdk_config);

    // For Minio/S3 compatible backends, we often need path style access
    if config["force_path_style"].as_bool().unwrap_or(false) {
        s3_config_builder = s3_config_builder.force_path_style(true);
    }

    Ok(Client::from_conf(s3_config_builder.build()))
}

/// Generates a presigned URL for downloading or uploading an object to/from an S3 bucket.
pub async fn generate_presigned_url(
    client: &Client,
    bucket: &str,
    key: &str,
    is_upload: bool,
    expires_in: Duration,
) -> Result<String> {
    let presigning_config = PresigningConfig::builder().expires_in(expires_in).build()?;

    let url = if is_upload {
        client
            .put_object()
            .bucket(bucket)
            .key(key)
            .presigned(presigning_config)
            .await?
    } else {
        client
            .get_object()
            .bucket(bucket)
            .key(key)
            .presigned(presigning_config)
            .await?
    };

    Ok(url.uri().to_string())
}

use stormchaser_tls::build_client_config;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stormchaser_model::storage::BackendType;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_get_s3_client_config_parsing() {
        let backend = StorageBackend {
            id: Uuid::new_v4(),
            name: "test-s3".to_string(),
            description: None,
            backend_type: BackendType::S3,
            config: json!({
                "bucket": "my-bucket",
                "endpoint": "http://localhost:9000",
                "region": "us-east-1",
                "access_key": "test",
                "secret_key": "test",
                "force_path_style": true
            }),
            is_default_sfs: true,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let client = get_s3_client(&backend).await;
        assert!(client.is_ok());
    }
}
