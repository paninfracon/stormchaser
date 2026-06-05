use super::TestConnectionRequest;
use aws_config::Region;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const HTTP_TEST_TIMEOUT: Duration = Duration::from_secs(10);

fn is_blocked_ip(ip: IpAddr) -> bool {
    if ip.is_loopback() || ip.is_multicast() {
        return true;
    }

    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            // Block IPv6 link-local (fe80::/10) and unique-local (fc00::/7).
            let octets = v6.octets();
            let is_link_local = octets[0] == 0xfe && (octets[1] & 0xc0) == 0x80;
            let is_unique_local = octets[0] & 0xfe == 0xfc;
            is_link_local || is_unique_local
        }
    }
}

/// Validate that a URL is safe to connect to (SSRF protection).
///
/// Allows only `http` and `https` schemes and blocks loopback, link-local, and
/// RFC-1918 private addresses to prevent Server-Side Request Forgery attacks.
async fn validate_url_safe(raw_url: &str) -> Result<(url::Url, String, u16, Vec<IpAddr>), String> {
    let parsed = url::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        s => {
            return Err(format!(
                "Disallowed URL scheme '{}': only http and https are permitted",
                s
            ))
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "URL has no known port for scheme".to_string())?;

    // Reject bare IP addresses that fall in restricted ranges.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err(format!(
                "Disallowed host '{}': non-public addresses are not permitted",
                host
            ));
        }
        return Ok((parsed, host, port, vec![ip]));
    }

    // Reject well-known loopback/link-local hostnames regardless of case.
    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err(format!(
            "Disallowed host '{}': localhost is not permitted",
            host
        ));
    }

    let resolved_ips = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("Failed to resolve host '{}': {}", host, e))?
        .map(|addr| addr.ip())
        .collect::<Vec<_>>();

    if resolved_ips.is_empty() {
        return Err(format!("Host '{}' did not resolve to any addresses", host));
    }

    for ip in &resolved_ips {
        if is_blocked_ip(*ip) {
            return Err(format!(
                "Disallowed host '{}': resolved to non-public address {}",
                host, ip
            ));
        }
    }

    Ok((parsed, host, port, resolved_ips))
}

pub async fn validate_connection(payload: &TestConnectionRequest) -> (bool, String) {
    match payload.connection_type {
        stormchaser_model::connections::ConnectionType::HttpApi => {
            if let Some(base_url) = payload.config.get("base_url").and_then(|v| v.as_str()) {
                let (parsed_url, host, port, resolved_ips) = match validate_url_safe(base_url).await
                {
                    Ok(validated) => validated,
                    Err(reason) => return (false, reason),
                };

                let mut client_builder = reqwest::Client::builder().timeout(HTTP_TEST_TIMEOUT);
                for ip in resolved_ips {
                    client_builder = client_builder.resolve(&host, SocketAddr::new(ip, port));
                }
                let client = match client_builder.build() {
                    Ok(c) => c,
                    Err(e) => return (false, format!("Failed to build client: {}", e)),
                };

                let mut req = client.get(parsed_url);
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

#[cfg(test)]
mod tests {
    use super::is_blocked_ip;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn blocks_ipv4_multicast() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))));
    }

    #[test]
    fn blocks_ipv6_multicast() {
        assert!(is_blocked_ip(IpAddr::V6(Ipv6Addr::new(
            0xff00, 0, 0, 0, 0, 0, 0, 1
        ))));
    }

    #[test]
    fn allows_public_ipv4() {
        assert!(!is_blocked_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }
}
