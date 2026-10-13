//! SSRF protection for outbound HTTP requests to URLs that workflows or users supply.
//!
//! Every such request should go through [`guarded_client`], which refuses URLs that
//! lead to internal addresses and returns a client that can only connect to the
//! addresses that were checked.
//!
//! This lives in the model crate only because the engine and API both depend on it; it
//! is due to move to a shared utility crate (see docs/security-remediation-plan.md).

use anyhow::{anyhow, bail, Result};
use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// Comma-separated hosts that outbound requests may reach even though they resolve to
/// internal addresses. An entry is a hostname or IP literal, or `*.suffix` to match any
/// host under a domain.
pub const ALLOWED_HOSTS_ENV: &str = "STORMCHASER_SSRF_ALLOWED_HOSTS";

/// Default overall timeout for a guarded request; callers may set a shorter one per request.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest response body a workflow-driven request may read into memory.
pub const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

fn is_blocked_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || o[0] == 0 // "this network" 0.0.0.0/8
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // carrier-grade NAT 100.64.0.0/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments 192.0.0.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18.0.0/15
        || o[0] >= 240 // reserved 240.0.0.0/4
}

/// True if `ip` is in a range Stormchaser must never connect to on a workflow's behalf:
/// loopback, private, link-local (including the 169.254.169.254 cloud-metadata address),
/// carrier-grade NAT, reserved and multicast ranges, and the IPv6 equivalents.
///
/// IPv6 forms that carry an IPv4 address (IPv4-mapped, IPv4-compatible, NAT64, 6to4) are
/// unwrapped so they cannot smuggle a restricted v4 address past the v6 checks.
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_v4(v4);
            }
            let o = v6.octets();
            let embedded_v4 = |at: usize| Ipv4Addr::new(o[at], o[at + 1], o[at + 2], o[at + 3]);

            // IPv4-compatible ::a.b.c.d (also covers :: and ::1)
            if o[..12].iter().all(|b| *b == 0) {
                return true;
            }
            // NAT64 64:ff9b::/96 translates to the embedded v4 address
            if o[..12] == [0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
                return is_blocked_v4(embedded_v4(12));
            }
            // 6to4 2002::/16 routes to the embedded v4 address
            if o[0] == 0x20 && o[1] == 0x02 {
                return is_blocked_v4(embedded_v4(2));
            }

            v6.is_multicast()
                || (o[0] == 0xfe && (o[1] & 0xc0) == 0x80) // link-local fe80::/10
                || (o[0] == 0xfe && (o[1] & 0xc0) == 0xc0) // site-local fec0::/10
                || (o[0] & 0xfe == 0xfc) // unique-local fc00::/7
                || o[..6] == [0, 0x64, 0xff, 0x9b, 0, 1] // local-use NAT64 64:ff9b:1::/48
                || o[..4] == [0x20, 0x01, 0, 0] // Teredo 2001::/32
        }
    }
}

/// A URL that passed the SSRF checks, with the addresses it was checked against.
#[derive(Debug, Clone)]
pub struct ValidatedUrl {
    /// The parsed URL.
    pub url: Url,
    /// Host name to pin; `None` when the URL holds an IP literal.
    pub domain: Option<String>,
    /// The only addresses a request for this URL may connect to.
    pub addrs: Vec<SocketAddr>,
}

fn parse_allowed_hosts(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn host_is_allowed(allowed: &[String], host: &str) -> bool {
    let host = host.to_lowercase();
    allowed.iter().any(|entry| match entry.strip_prefix("*.") {
        Some(suffix) => host
            .strip_suffix(suffix)
            .is_some_and(|rest| rest.ends_with('.')),
        None => *entry == host,
    })
}

/// Validates `raw_url` against the operator's allow-list in [`ALLOWED_HOSTS_ENV`].
pub async fn validate_url_safe(raw_url: &str) -> Result<ValidatedUrl, String> {
    let allowed = parse_allowed_hosts(std::env::var(ALLOWED_HOSTS_ENV).ok());
    validate_url_with(raw_url, &allowed).await
}

/// Validates that a URL is safe to connect to.
///
/// Allows only `http`/`https`, resolves the host, and rejects the URL if any resolved
/// address is restricted (see [`is_blocked_ip`]) unless the host is in `allowed_hosts`.
pub async fn validate_url_with(
    raw_url: &str,
    allowed_hosts: &[String],
) -> Result<ValidatedUrl, String> {
    let url = Url::parse(raw_url).map_err(|e| format!("Invalid URL: {}", e))?;

    match url.scheme() {
        "http" | "https" => {}
        s => {
            return Err(format!(
                "Disallowed URL scheme '{}': only http and https are permitted",
                s
            ))
        }
    }

    let port = url.port_or_known_default().unwrap_or(443);
    let (display_host, domain, resolved): (String, Option<String>, Vec<IpAddr>) = match url.host() {
        Some(url::Host::Ipv4(ip)) => (ip.to_string(), None, vec![IpAddr::V4(ip)]),
        Some(url::Host::Ipv6(ip)) => (ip.to_string(), None, vec![IpAddr::V6(ip)]),
        Some(url::Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.').to_lowercase();
            let ips = tokio::net::lookup_host((domain.as_str(), port))
                .await
                .map_err(|e| format!("Could not resolve host '{}': {}", domain, e))?
                .map(|addr| addr.ip())
                .collect::<Vec<_>>();
            (domain.clone(), Some(domain), ips)
        }
        None => return Err("URL has no host".to_string()),
    };

    if resolved.is_empty() {
        return Err(format!(
            "Host '{}' did not resolve to any address",
            display_host
        ));
    }

    if !host_is_allowed(allowed_hosts, &display_host) {
        if display_host == "localhost" || display_host.ends_with(".localhost") {
            return Err(format!(
                "Disallowed host '{}': localhost is not permitted",
                display_host
            ));
        }
        // Reject if ANY resolved address is restricted, so a name with one public and
        // one internal record cannot be used to reach the internal one.
        if let Some(ip) = resolved.iter().find(|ip| is_blocked_ip(**ip)) {
            return Err(format!(
                "Disallowed host '{}': resolves to a restricted address ({}). Operators can permit it with {}",
                display_host, ip, ALLOWED_HOSTS_ENV
            ));
        }
    }

    Ok(ValidatedUrl {
        url,
        domain,
        addrs: resolved
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect(),
    })
}

/// Builds an HTTP client for one request to `raw_url`, or fails if the URL is unsafe.
///
/// The client connects only to the addresses that were validated, which closes the
/// DNS-rebinding window between the check and the connection, and it does not follow
/// redirects, since a 30x to an internal host would otherwise bypass the check.
pub async fn guarded_client(raw_url: &str) -> Result<(reqwest::Client, Url)> {
    let validated = validate_url_safe(raw_url)
        .await
        .map_err(|reason| anyhow!("Refusing request to {}: {}", raw_url, reason))?;

    let mut builder = reqwest::Client::builder()
        .timeout(DEFAULT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if let Some(domain) = &validated.domain {
        builder = builder.resolve_to_addrs(domain, &validated.addrs);
    }

    let client = builder
        .build()
        .map_err(|e| anyhow!("Failed to build HTTP client: {}", e))?;
    Ok((client, validated.url))
}

/// Reads a response body, failing once it exceeds `max_bytes` instead of buffering it all.
pub async fn read_body_capped(mut res: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    if res
        .content_length()
        .is_some_and(|len| len > max_bytes as u64)
    {
        bail!("Response body exceeds the {} byte limit", max_bytes);
    }

    let mut body = Vec::new();
    while let Some(chunk) = res.chunk().await? {
        if body.len() + chunk.len() > max_bytes {
            bail!("Response body exceeds the {} byte limit", max_bytes);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

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
    fn blocks_reserved_ranges_and_v4_embedded_in_v6() {
        for s in [
            "0.1.2.3",             // "this network"
            "100.64.0.1",          // carrier-grade NAT
            "100.100.100.200",     // Alibaba metadata
            "192.0.0.8",           // IETF protocol assignments
            "198.18.0.1",          // benchmarking
            "240.0.0.1",           // reserved
            "255.255.255.255",     // broadcast
            "::127.0.0.1",         // IPv4-compatible
            "64:ff9b::a9fe:a9fe",  // NAT64 -> 169.254.169.254
            "64:ff9b:1::1",        // local-use NAT64
            "2002:a9fe:a9fe::1",   // 6to4 -> 169.254.169.254
            "2002:0a00:0001::1",   // 6to4 -> 10.0.0.1
            "2001:0:4136:e378::1", // Teredo
            "fec0::1",             // site-local
        ] {
            assert!(is_blocked_ip(ip(s)), "expected {} to be blocked", s);
        }
    }

    #[test]
    fn blocks_multicast() {
        assert!(is_blocked_ip(IpAddr::V4(Ipv4Addr::new(224, 0, 0, 1))));
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
            "100.63.255.255", // just below carrier-grade NAT
            "198.20.0.1",     // just above benchmarking
            "2606:4700:4700::1111",
            "64:ff9b::808:808", // NAT64 -> 8.8.8.8
            "2002:808:808::1",  // 6to4 -> 8.8.8.8
        ] {
            assert!(!is_blocked_ip(ip(s)), "expected {} to be allowed", s);
        }
    }

    #[test]
    fn allow_list_matches_exact_hosts_and_domain_suffixes() {
        let allowed = parse_allowed_hosts(Some(
            " Vault.internal , *.svc.cluster.local,10.0.0.5".into(),
        ));
        assert!(host_is_allowed(&allowed, "vault.internal"));
        assert!(host_is_allowed(&allowed, "api.prod.svc.cluster.local"));
        assert!(host_is_allowed(&allowed, "10.0.0.5"));

        assert!(!host_is_allowed(&allowed, "svc.cluster.local"));
        assert!(!host_is_allowed(&allowed, "evilsvc.cluster.local"));
        assert!(!host_is_allowed(
            &allowed,
            "vault.internal.attacker.example"
        ));
        assert!(!host_is_allowed(&[], "anything"));
    }

    #[tokio::test]
    async fn rejects_internal_literals_schemes_and_localhost() {
        for url in [
            "http://169.254.169.254/latest/meta-data/",
            "http://127.0.0.1:8181/v1/data",
            "http://[::1]:4222/",
            "http://[::ffff:10.0.0.1]/",
            "http://2130706433/", // decimal form of 127.0.0.1
            "http://0x7f.0.0.1/", // hex octet form
            "http://localhost:5432/",
            "file:///etc/passwd",
            "gopher://example.com/",
        ] {
            assert!(
                validate_url_with(url, &[]).await.is_err(),
                "expected {} to be rejected",
                url
            );
        }
    }

    #[tokio::test]
    async fn allow_listed_internal_host_is_permitted_and_pinned() {
        let allowed = vec!["127.0.0.1".to_string()];
        let validated = validate_url_with("http://127.0.0.1:9000/x", &allowed)
            .await
            .unwrap();
        assert_eq!(validated.addrs, vec!["127.0.0.1:9000".parse().unwrap()]);
        assert!(validated.domain.is_none());

        // Being allow-listed does not widen the permitted schemes.
        assert!(validate_url_with("ftp://127.0.0.1/", &allowed)
            .await
            .is_err());
    }
}
