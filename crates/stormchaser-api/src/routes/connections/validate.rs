use super::TestConnectionRequest;
use aws_config::Region;
use std::time::Duration;

const HTTP_TEST_TIMEOUT: Duration = Duration::from_secs(10);

pub async fn validate_connection(payload: &TestConnectionRequest) -> (bool, String) {
    match payload.connection_type {
        stormchaser_model::connections::ConnectionType::HttpApi => {
            if let Some(base_url) = payload.config.get("base_url").and_then(|v| v.as_str()) {
                let client = match reqwest::Client::builder()
                    .timeout(HTTP_TEST_TIMEOUT)
                    .build()
                {
                    Ok(c) => c,
                    Err(e) => return (false, format!("Failed to build client: {}", e)),
                };
                let mut req = client.get(base_url);
                if let Some(headers) = payload.config.get("headers").and_then(|v| v.as_object()) {
                    for (k, v) in headers {
                        if let Some(s) = v.as_str() {
                            req = req.header(k, s);
                        }
                    }
                }
                match req.send().await {
                    Ok(res) => {
                        if res.status().is_success() || res.status().is_redirection() {
                            (
                                true,
                                format!("Successfully connected: HTTP {}", res.status()),
                            )
                        } else {
                            (
                                false,
                                format!(
                                    "Connected but received error status: HTTP {}",
                                    res.status()
                                ),
                            )
                        }
                    }
                    Err(e) => (false, format!("Failed to connect: {}", e)),
                }
            } else {
                (false, "Missing base_url".to_string())
            }
        }
        stormchaser_model::connections::ConnectionType::Git => {
            if let Some(url) = payload
                .config
                .get("url")
                .and_then(|v| v.as_str())
                .or_else(|| payload.config.get("repo").and_then(|v| v.as_str()))
                .or_else(|| payload.config.get("repo_url").and_then(|v| v.as_str()))
            {
                // We'll just test if the repo is reachable.
                match git2::Remote::create_detached(url) {
                    Ok(mut remote) => {
                        let cb = git2::RemoteCallbacks::new();
                        match remote.connect_auth(git2::Direction::Fetch, Some(cb), None) {
                            Ok(_) => (true, "Successfully reached Git repository".to_string()),
                            Err(e) => {
                                (false, format!("Failed to connect to Git repository: {}", e))
                            }
                        }
                    }
                    Err(e) => (
                        false,
                        format!("Failed to create detached Git remote: {}", e),
                    ),
                }
            } else {
                (false, "Missing repo url".to_string())
            }
        }
        stormchaser_model::connections::ConnectionType::Postgres => {
            if let Some(url) = payload.config.get("url").and_then(|v| v.as_str()) {
                match sqlx::postgres::PgPoolOptions::new()
                    .acquire_timeout(std::time::Duration::from_secs(5))
                    .connect(url)
                    .await
                {
                    Ok(pool) => {
                        pool.close().await;
                        (true, "Successfully connected to Postgres".to_string())
                    }
                    Err(e) => (false, format!("Failed to connect to Postgres: {}", e)),
                }
            } else {
                (false, "Missing url".to_string())
            }
        }
        stormchaser_model::connections::ConnectionType::Mysql => {
            if payload.config.get("url").is_some() {
                (
                    false,
                    "MySQL connectivity validation is not implemented in this build".to_string(),
                )
            } else {
                (false, "Missing url".to_string())
            }
        }
        stormchaser_model::connections::ConnectionType::S3 => {
            if let Some(bucket) = payload.config.get("bucket").and_then(|v| v.as_str()) {
                let access_key = payload.config.get("access_key").and_then(|v| v.as_str());
                let secret_key = payload.config.get("secret_key").and_then(|v| v.as_str());
                let region = payload
                    .config
                    .get("region")
                    .and_then(|v| v.as_str())
                    .unwrap_or("us-east-1");
                let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
                    .region(Region::new(region.to_string()));
                if let (Some(access_key), Some(secret_key)) = (access_key, secret_key) {
                    loader = loader.credentials_provider(aws_sdk_s3::config::Credentials::new(
                        access_key,
                        secret_key,
                        None,
                        None,
                        "stormchaser",
                    ));
                }

                let mut sdk_config = loader.load().await;
                if let Some(role_arn) = payload
                    .aws_assume_role_arn
                    .as_deref()
                    .filter(|v| !v.is_empty())
                {
                    let sts_client = aws_sdk_sts::Client::new(&sdk_config);
                    match sts_client
                        .assume_role()
                        .role_arn(role_arn)
                        .role_session_name("StormchaserConnectionTest")
                        .send()
                        .await
                    {
                        Ok(assume) => {
                            if let Some(credentials) = assume.credentials() {
                                sdk_config = sdk_config
                                    .into_builder()
                                    .credentials_provider(
                                        aws_sdk_s3::config::SharedCredentialsProvider::new(
                                            aws_sdk_s3::config::Credentials::new(
                                                credentials.access_key_id(),
                                                credentials.secret_access_key(),
                                                Some(credentials.session_token().to_string()),
                                                None,
                                                "StsAssumedRole",
                                            ),
                                        ),
                                    )
                                    .build();
                            } else {
                                return (
                                    false,
                                    "AssumeRole succeeded but returned no credentials".to_string(),
                                );
                            }
                        }
                        Err(e) => {
                            return (false, format!("Failed to assume role: {}", e));
                        }
                    }
                }

                let mut config = aws_sdk_s3::config::Builder::from(&sdk_config);
                if let Some(endpoint) = payload.config.get("endpoint").and_then(|v| v.as_str()) {
                    config = config.endpoint_url(endpoint);
                }
                if payload
                    .config
                    .get("force_path_style")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    config = config.force_path_style(true);
                }
                let client = aws_sdk_s3::Client::from_conf(config.build());
                match client.head_bucket().bucket(bucket).send().await {
                    Ok(_) => (true, "Successfully connected to S3 bucket".to_string()),
                    Err(e) => (false, format!("Failed to access S3 bucket: {}", e)),
                }
            } else {
                (false, "Missing bucket".to_string())
            }
        }
        _ => (
            true,
            "Connection type validation not implemented".to_string(),
        ),
    }
}
