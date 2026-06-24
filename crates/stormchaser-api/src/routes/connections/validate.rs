use super::TestConnectionRequest;
use aws_config::Region;
use std::net::IpAddr;
use std::time::Duration;

const HTTP_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// True if `ip` is in a range Stormchaser must never connect to — loopback,
/// private (RFC-1918), link-local (169.254.0.0/16, incl. the 169.254.169.254
/// cloud-metadata address), unspecified/broadcast, multicast (IPv4 224.0.0.0/4,
/// IPv6 ff00::/8), and the IPv6 equivalents.
/// IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is unwrapped so it can't smuggle a
/// restricted v4 address past the v6 checks.
fn is_blocked_ip(ip: IpAddr) -> bool {
    if ip.is_multicast() {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(v4));
            }
            let o = v6.octets();
            v6.is_loopback()
                || v6.is_unspecified()
                || (o[0] == 0xfe && (o[1] & 0xc0) == 0x80) // link-local fe80::/10
                || (o[0] & 0xfe == 0xfc) // unique-local fc00::/7
        }
    }
}

/// Validate that a URL is safe to connect to (SSRF protection).
///
/// Allows only `http`/`https`, then **resolves the host via DNS** and rejects
/// the URL if ANY resolved address is restricted (see `is_blocked_ip`). This
/// prevents connecting to internal IP addresses.
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

    let port = parsed.port_or_known_default().unwrap_or(443);

    // Reject well-known loopback hostnames up front (before resolution).
    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err(format!(
            "Disallowed host '{}': localhost is not permitted",
            host
        ));
    }

    // Resolve the host and reject if ANY resolved IP is restricted. This is the
    // key SSRF gate: it catches hostnames that resolve to internal/metadata IPs
    // (incl. DNS-rebinding-style inputs) and bare IPs alike.
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("Could not resolve host '{}': {}", host, e))?;

    let mut resolved_any = false;
    let mut resolved_ips = Vec::new();
    for addr in addrs {
        resolved_any = true;
        let ip = addr.ip();
        if is_blocked_ip(ip) {
            return Err(format!(
                "Disallowed host '{}': resolves to a restricted address ({})",
                host, ip
            ));
        }
        resolved_ips.push(ip);
    }
    if !resolved_any {
        return Err(format!("Host '{}' did not resolve to any address", host));
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

                let resolved_addrs = resolved_ips
                    .into_iter()
                    .map(|ip| std::net::SocketAddr::new(ip, port))
                    .collect::<Vec<_>>();

                let client = match reqwest::Client::builder()
                    .timeout(HTTP_TEST_TIMEOUT)
                    // Do not follow redirects: a 30x to an internal host would
                    // otherwise bypass validate_url_safe (SSRF via redirect).
                    .redirect(reqwest::redirect::Policy::none())
                    // Resolve DNS queries to the exact IPs we just validated to
                    // prevent TOCTOU DNS rebinding attacks.
                    .resolve_to_addrs(&host, &resolved_addrs)
                    .build()
                {
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

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocks_loopback_private_linklocal_metadata_and_mapped() {
        for s in [
            "127.0.0.1",
            "10.0.0.1",
            "192.168.1.1",
            "172.16.0.1",
            "169.254.169.254", // cloud metadata (link-local)
            "0.0.0.0",
            "::1",
            "fe80::1",                // IPv6 link-local
            "fc00::1",                // IPv6 unique-local
            "::ffff:127.0.0.1",       // IPv4-mapped loopback
            "::ffff:169.254.169.254", // IPv4-mapped metadata
        ] {
            assert!(is_blocked_ip(ip(s)), "expected {} to be blocked", s);
        }
    }

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
    fn allows_public_addresses() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(!is_blocked_ip(ip(s)), "expected {} to be allowed", s);
        }
    }
}
