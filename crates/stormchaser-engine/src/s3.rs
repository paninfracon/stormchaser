use anyhow::Result;
use aws_config::Region;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::Client;
use std::time::Duration;
use stormchaser_model::connections::Connection;

/// Refuses a connection that would sign with the engine's own AWS identity, unless it is
/// the default storage backend.
///
/// A connection with no keys and no role falls back to whatever credentials the engine
/// runs with (an IRSA or instance role). Any user may create a connection naming any
/// bucket, so that would let them have the engine sign uploads into every bucket its
/// role can write. Only an admin chooses the default backend, so it alone may rely on
/// them; any other connection brings its own keys or an allow-listed role.
fn ensure_credentials_are_its_own(backend: &Connection) -> Result<()> {
    let has_keys =
        backend.config["access_key"].is_string() && backend.config["secret_key"].is_string();
    let has_role = cfg!(feature = "aws-s3-sts") && backend.aws_assume_role_arn.is_some();
    if backend.is_default_sfs || has_keys || has_role {
        Ok(())
    } else {
        anyhow::bail!(
            "S3 connection '{}' has no access_key and secret_key or role; only the default storage backend may use the engine's own credentials",
            backend.name
        )
    }
}

/// Get s3 client.
pub async fn get_s3_client(backend: &Connection) -> Result<Client> {
    let config = &backend.config;
    let endpoint = config["endpoint"].as_str();
    let region = config["region"].as_str().unwrap_or("us-east-1");
    let access_key = config["access_key"].as_str();
    let secret_key = config["secret_key"].as_str();

    ensure_credentials_are_its_own(backend)?;

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

    #[cfg(feature = "aws-s3-sts")]
    let sdk_config = if let Some(role_arn) = &backend.aws_assume_role_arn {
        crate::aws_guard::ensure_role_allowed(role_arn)?;
        let sts_client = aws_sdk_sts::Client::new(&sdk_config);
        let assume_role_res = sts_client
            .assume_role()
            .role_arn(role_arn)
            .role_session_name("StormchaserS3Backend")
            .send()
            .await?;

        if let Some(credentials) = assume_role_res.credentials() {
            let provider = aws_sdk_s3::config::Credentials::new(
                credentials.access_key_id(),
                credentials.secret_access_key(),
                Some(credentials.session_token().to_string()),
                None,
                "StsAssumedRole",
            );
            let shared_provider = aws_sdk_s3::config::SharedCredentialsProvider::new(provider);
            sdk_config
                .into_builder()
                .credentials_provider(shared_provider)
                .build()
        } else {
            return Err(anyhow::anyhow!("Missing credentials from assume_role"));
        }
    } else {
        sdk_config
    };

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
    use chrono::Utc;
    use serde_json::json;
    use stormchaser_model::connections::ConnectionType;
    use stormchaser_model::ConnectionId;

    #[tokio::test]
    async fn test_get_s3_client_config_parsing() {
        let backend = Connection {
            id: ConnectionId::new_v4(),
            name: "test-s3".to_string(),
            description: None,
            connection_type: ConnectionType::S3,
            config: json!({
                "bucket": "test",
                "endpoint": "http://localhost:9000",
                "region": "us-east-1",
                "access_key": "minio",
                "secret_key": "minio123",
                "force_path_style": true,
            }),
            aws_assume_role_arn: None,
            is_default_sfs: false,
            encrypted_credentials: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let client = get_s3_client(&backend).await;
        client.unwrap();
    }

    fn keyless(is_default_sfs: bool) -> Connection {
        Connection {
            id: ConnectionId::new_v4(),
            name: "keyless".to_string(),
            description: None,
            connection_type: ConnectionType::S3,
            config: json!({"bucket": "someone-elses-bucket"}),
            aws_assume_role_arn: None,
            is_default_sfs,
            encrypted_credentials: None,
            ca_cert: None,
            client_cert: None,
            client_key: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn only_the_default_backend_may_use_the_engines_own_credentials() {
        assert!(ensure_credentials_are_its_own(&keyless(true)).is_ok());
        let err = ensure_credentials_are_its_own(&keyless(false)).unwrap_err();
        assert!(err.to_string().contains("default storage backend"));

        let mut with_keys = keyless(false);
        with_keys.config = json!({"bucket": "b", "access_key": "a", "secret_key": "s"});
        assert!(ensure_credentials_are_its_own(&with_keys).is_ok());

        let mut half_keys = keyless(false);
        half_keys.config = json!({"bucket": "b", "access_key": "a"});
        assert!(ensure_credentials_are_its_own(&half_keys).is_err());

        let mut with_role = keyless(false);
        with_role.aws_assume_role_arn = Some("arn:aws:iam::1:role/r".to_string());
        assert_eq!(
            ensure_credentials_are_its_own(&with_role).is_ok(),
            cfg!(feature = "aws-s3-sts")
        );
    }
}
