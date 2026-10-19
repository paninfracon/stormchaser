use super::TestConnectionRequest;
use aws_config::Region;
use std::time::Duration;

const HTTP_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Host and port a repository URL connects to. The URL has already passed
/// `validate_repo_url`, so it is `scheme://…` or scp-like `user@host:path`.
fn git_host_port(repo_url: &str) -> Result<(String, u16), String> {
    if repo_url.contains("://") {
        let parsed = url::Url::parse(repo_url).map_err(|e| format!("Invalid URL: {}", e))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| "Repository URL has no host".to_string())?
            .to_string();
        let port = parsed
            .port_or_known_default()
            .unwrap_or(if parsed.scheme() == "ssh" { 22 } else { 443 });
        return Ok((host, port));
    }
    let (user_host, _path) = repo_url
        .split_once(':')
        .ok_or_else(|| "Repository URL has no host".to_string())?;
    let host = user_host.rsplit_once('@').map_or(user_host, |(_, h)| h);
    Ok((host.to_string(), 22))
}

/// Refuses a repository the engine would not clone, or one on an internal address.
async fn check_git_target(repo_url: &str) -> Result<(), String> {
    stormchaser_core::git_cache::validate_repo_url(repo_url).map_err(|e| e.to_string())?;
    let (host, port) = git_host_port(repo_url)?;
    stormchaser_core::net_guard::validate_host_safe(&host, port)
        .await
        .map(|_| ())
}

/// Parses a Postgres URL and refuses Unix sockets and internal addresses. The returned
/// options connect to the address that was checked, except under `verify-full`, where
/// the server must prove the name anyway.
async fn check_postgres_target(url: &str) -> Result<sqlx::postgres::PgConnectOptions, String> {
    use sqlx::postgres::{PgConnectOptions, PgSslMode};
    use std::str::FromStr;

    // Not echoed back: the parse error can quote the URL, password included
    let options =
        PgConnectOptions::from_str(url).map_err(|_| "Invalid Postgres URL".to_string())?;
    // A URL without a host makes the driver look for a local socket or use PGHOST
    let names_host = url::Url::parse(url)
        .ok()
        .is_some_and(|u| u.host_str().is_some_and(|h| !h.is_empty()));
    if !names_host || options.get_socket().is_some() || options.get_host().starts_with('/') {
        return Err("Postgres URL must name a network host".to_string());
    }
    let addrs =
        stormchaser_core::net_guard::validate_host_safe(options.get_host(), options.get_port())
            .await?;
    if matches!(options.get_ssl_mode(), PgSslMode::VerifyFull) {
        return Ok(options);
    }
    match addrs.first() {
        Some(addr) => Ok(options.host(&addr.ip().to_string())),
        None => Err("Host did not resolve to any address".to_string()),
    }
}

/// Refuses an S3 test that would use the API's own AWS identity, or an endpoint on an
/// internal address.
///
/// Without keys of its own the test would run as the API: any user could probe buckets
/// the API can reach, or have it assume any role that trusts it.
async fn check_s3_target(
    has_keys: bool,
    role_arn: Option<&str>,
    endpoint: Option<&str>,
) -> Result<(), String> {
    match role_arn {
        Some(role_arn) => {
            stormchaser_core::aws_guard::ensure_role_allowed(role_arn).map_err(|e| e.to_string())?
        }
        None if !has_keys => {
            return Err(
                "An S3 connection test needs access_key and secret_key, or an allow-listed role"
                    .to_string(),
            )
        }
        None => {}
    }
    if let Some(endpoint) = endpoint {
        stormchaser_core::net_guard::validate_url_safe(endpoint).await?;
    }
    Ok(())
}

pub async fn validate_connection(payload: &TestConnectionRequest) -> (bool, String) {
    match payload.connection_type {
        stormchaser_model::connections::ConnectionType::HttpApi => {
            if let Some(base_url) = payload.config.get("base_url").and_then(|v| v.as_str()) {
                // Refuses internal addresses, pins the validated ones and does not follow
                // redirects (see `net_guard`).
                let (client, parsed_url) =
                    match stormchaser_core::net_guard::guarded_client(base_url).await {
                        Ok(guarded) => guarded,
                        Err(reason) => return (false, reason.to_string()),
                    };
                let mut req = client.get(parsed_url).timeout(HTTP_TEST_TIMEOUT);
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
                if let Err(reason) = check_git_target(url).await {
                    return (false, reason);
                }
                let url = url.to_string();
                // git2 network I/O is blocking; run it on a dedicated thread pool so it
                // does not block the async Tokio runtime.
                tokio::task::spawn_blocking(move || match git2::Remote::create_detached(&*url) {
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
                })
                .await
                .unwrap_or_else(|e| (false, format!("Git validation task panicked: {}", e)))
            } else {
                (false, "Missing repo url".to_string())
            }
        }
        stormchaser_model::connections::ConnectionType::Postgres => {
            if let Some(url) = payload.config.get("url").and_then(|v| v.as_str()) {
                let options = match check_postgres_target(url).await {
                    Ok(options) => options,
                    Err(reason) => return (false, reason),
                };
                match sqlx::postgres::PgPoolOptions::new()
                    .max_connections(1)
                    .acquire_timeout(std::time::Duration::from_secs(5))
                    .connect_with(options)
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
                let role_arn = payload
                    .aws_assume_role_arn
                    .as_deref()
                    .filter(|v| !v.is_empty());
                let endpoint = payload.config.get("endpoint").and_then(|v| v.as_str());
                if let Err(reason) = check_s3_target(
                    access_key.is_some() && secret_key.is_some(),
                    role_arn,
                    endpoint,
                )
                .await
                {
                    return (false, reason);
                }
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
                if let Some(role_arn) = role_arn {
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
                if let Some(endpoint) = endpoint {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_host_a_repository_url_connects_to() {
        for (url, host, port) in [
            ("https://github.com/org/repo.git", "github.com", 443),
            (
                "https://user:pw@git.example.com:8443/r.git",
                "git.example.com",
                8443,
            ),
            (
                "ssh://git@git.example.com/org/repo.git",
                "git.example.com",
                22,
            ),
            (
                "ssh://git@git.example.com:2222/org/repo.git",
                "git.example.com",
                2222,
            ),
            ("git@github.com:org/repo.git", "github.com", 22),
            // The last `@` wins, so a user name cannot pose as the host
            ("public.example.com@10.0.0.1:repo.git", "10.0.0.1", 22),
        ] {
            assert_eq!(
                git_host_port(url).unwrap(),
                (host.to_string(), port),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn refuses_local_and_internal_repositories() {
        for url in [
            "file:///etc",
            "/var/lib/stormchaser/cache/repo",
            "https://169.254.169.254/latest/meta-data",
            "ssh://git@10.0.0.5/org/repo.git",
            "git@192.168.1.10:org/repo.git",
            "https://[::1]/repo.git",
        ] {
            assert!(check_git_target(url).await.is_err(), "{url} was accepted");
        }
    }

    #[tokio::test]
    async fn refuses_sockets_and_internal_postgres_hosts() {
        for url in [
            "postgres://u:p@10.0.0.5/db",
            "postgres://u:p@169.254.169.254:5432/db",
            "postgres://u:p@[fd00::1]/db",
            "postgres:///db",
            "postgres:///db?host=/var/run/postgresql",
            "postgres://u:p@%2Fvar%2Frun%2Fpostgresql/db",
            "not a url",
        ] {
            assert!(
                check_postgres_target(url).await.is_err(),
                "{url} was accepted"
            );
        }
    }

    #[tokio::test]
    async fn a_public_postgres_host_is_pinned_to_the_checked_address() {
        let options = check_postgres_target("postgres://u:p@93.184.216.34:6543/db")
            .await
            .unwrap();
        assert_eq!(options.get_host(), "93.184.216.34");
        assert_eq!(options.get_port(), 6543);
    }

    #[tokio::test]
    async fn s3_tests_never_run_as_the_api() {
        // No keys and no role: the API's own identity would be used
        assert!(check_s3_target(false, None, None).await.is_err());
        // A role the operator has not listed
        assert!(check_s3_target(
            true,
            Some("arn:aws:iam::111111111111:role/unlisted-m6"),
            None
        )
        .await
        .is_err());
        // Explicit keys, public endpoint
        assert!(check_s3_target(true, None, None).await.is_ok());
        // Explicit keys, internal endpoint
        assert!(check_s3_target(true, None, Some("http://169.254.169.254/"))
            .await
            .is_err());
    }
}
