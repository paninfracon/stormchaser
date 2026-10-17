use async_nats::jetstream;
use async_nats::jetstream::kv::Config;
use async_nats::Client;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use chrono::Utc;
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

/// Comma-separated addresses or CIDR ranges of the reverse proxies in front of the API.
///
/// Only a request arriving from one of these has its `X-Forwarded-For` header believed.
/// Empty by default: every request is counted against the address it connected from.
pub const TRUSTED_PROXIES_ENV: &str = "STORMCHASER_TRUSTED_PROXIES";

/// An address range, as an address and a prefix length.
pub type Cidr = (IpAddr, u8);

/// State for the NATS-backed rate limiter
#[derive(Clone)]
pub struct RateLimitState {
    /// NATS client connection
    pub nats: Client,
    /// Lazy initialized Key-Value store for rate limiting
    pub store: Arc<OnceCell<jetstream::kv::Store>>,
    /// Tokens a client's bucket regains per second
    pub per_second: u64,
    /// Size of a client's bucket: the most requests it can make at once
    pub burst_size: u64,
    /// Proxies whose `X-Forwarded-For` header is believed
    pub trusted_proxies: Vec<Cidr>,
}

/// Parses [`TRUSTED_PROXIES_ENV`]. Entries that are not an address or a CIDR are skipped.
pub fn parse_trusted_proxies(value: &str) -> Vec<Cidr> {
    value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .filter_map(|entry| {
            let parsed = match entry.split_once('/') {
                Some((ip, len)) => ip
                    .parse::<IpAddr>()
                    .ok()
                    .zip(len.parse::<u8>().ok())
                    .filter(|(ip, len)| *len <= if ip.is_ipv4() { 32 } else { 128 }),
                None => entry
                    .parse::<IpAddr>()
                    .ok()
                    .map(|ip| (ip, if ip.is_ipv4() { 32 } else { 128 })),
            };
            if parsed.is_none() {
                tracing::warn!("Ignoring invalid {} entry '{}'", TRUSTED_PROXIES_ENV, entry);
            }
            parsed
        })
        .collect()
}

fn in_cidr(ip: IpAddr, (net, len): Cidr) -> bool {
    fn bits(ip: IpAddr) -> Option<(u128, u8)> {
        match ip {
            IpAddr::V4(v4) => Some((u32::from(v4) as u128, 32)),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => Some((u32::from(v4) as u128, 32)),
                None => Some((u128::from(v6), 128)),
            },
        }
    }
    let (Some((ip, ip_width)), Some((net, net_width))) = (bits(ip), bits(net)) else {
        return false;
    };
    if ip_width != net_width {
        return false;
    }
    let len = len.min(ip_width);
    if len == 0 {
        return true;
    }
    let shift = ip_width - len;
    (ip >> shift) == (net >> shift)
}

fn is_trusted(ip: IpAddr, trusted: &[Cidr]) -> bool {
    trusted.iter().any(|cidr| in_cidr(ip, *cidr))
}

/// The address a request is counted against.
///
/// That is the peer, unless the peer is a trusted proxy: then it is the last address in
/// `X-Forwarded-For` that is not itself a trusted proxy. Reading from the right means a
/// client cannot choose its own key by sending the header, because whatever it sends
/// ends up to the left of the address the proxy appended.
fn client_ip(peer: IpAddr, headers: &HeaderMap, trusted: &[Cidr]) -> IpAddr {
    if !is_trusted(peer, trusted) {
        return peer;
    }
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .rev()
        .map(|hop| hop.parse::<IpAddr>())
        // An entry that is not an address ends the walk: nothing left of it is reliable
        .take_while(Result::is_ok)
        .flatten()
        .find(|hop| !is_trusted(*hop, trusted))
        .unwrap_or(peer)
}

/// The key a client's bucket is stored under. An IPv6 client is counted by its /64, since
/// one host can use every address in it.
fn bucket_key(ip: IpAddr) -> String {
    let ip = match ip {
        IpAddr::V6(v6) if v6.to_ipv4_mapped().is_none() => {
            IpAddr::V6(Ipv6Addr::from(u128::from(v6) & !u128::from(u64::MAX)))
        }
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    };
    ip.to_string().replace(['.', ':'], "_")
}

/// A bucket as stored: tokens left, and when they were last topped up.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Bucket {
    tokens: f64,
    updated_ms: i64,
}

impl Bucket {
    fn parse(raw: &[u8]) -> Option<Self> {
        let (tokens, updated_ms) = std::str::from_utf8(raw).ok()?.split_once(':')?;
        let tokens: f64 = tokens.parse().ok()?;
        tokens.is_finite().then_some(Self {
            tokens,
            updated_ms: updated_ms.parse().ok()?,
        })
    }

    fn encode(&self) -> Bytes {
        format!("{:.3}:{}", self.tokens, self.updated_ms).into()
    }
}

/// Tops a bucket up for the time that has passed and takes one token from it.
///
/// Returns the bucket to store and whether the request may proceed. A client with no
/// bucket yet starts with a full one.
fn take_token(bucket: Option<Bucket>, now_ms: i64, per_second: u64, burst: u64) -> (Bucket, bool) {
    let burst = burst as f64;
    let tokens = match bucket {
        Some(bucket) => {
            let elapsed = (now_ms - bucket.updated_ms).max(0) as f64 / 1000.0;
            (bucket.tokens + elapsed * per_second as f64).min(burst)
        }
        None => burst,
    };
    let allowed = tokens >= 1.0;
    let bucket = Bucket {
        tokens: if allowed { tokens - 1.0 } else { tokens },
        updated_ms: now_ms,
    };
    (bucket, allowed)
}

/// Middleware that limits request rates with a token bucket per client, kept in NATS KV
/// so that every API replica draws from the same bucket.
pub async fn nats_rate_limiter(
    State(state): State<Arc<RateLimitState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    let key = bucket_key(client_ip(addr.ip(), req.headers(), &state.trusted_proxies));

    let store = match state
        .store
        .get_or_try_init(|| async {
            let js = jetstream::new(state.nats.clone());
            js.create_key_value(Config {
                bucket: "api_rate_limit_buckets".to_string(),
                // An idle bucket is forgotten, which is the same as it being full again
                max_age: Duration::from_secs(3600),
                ..Default::default()
            })
            .await
        })
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Failed to init NATS KV for rate limiting: {:?}", e);
            // Fail open
            return next.run(req).await;
        }
    };

    let mut retries = 0;
    let allowed = loop {
        if retries > 5 {
            break true; // fail open under high contention
        }

        let (current, revision) = match store.entry(&key).await {
            Ok(Some(entry)) => (Bucket::parse(&entry.value), entry.revision),
            Ok(None) => (None, 0),
            Err(e) => {
                tracing::error!("NATS KV rate limit error: {:?}", e);
                break true; // fail open
            }
        };
        let (bucket, allowed) = take_token(
            current,
            Utc::now().timestamp_millis(),
            state.per_second,
            state.burst_size,
        );
        // A refused request still stores the top-up, so a write that loses the race is
        // retried either way
        match store.update(&key, bucket.encode(), revision).await {
            Ok(_) => break allowed,
            Err(_) => retries += 1,
        }
    };

    if !allowed {
        return (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests").into_response();
    }

    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn headers(xff: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in xff {
            headers.append("x-forwarded-for", value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn a_bucket_allows_a_burst_then_the_steady_rate() {
        // 2 per second, bucket of 3
        let (mut bucket, mut allowed) = take_token(None, 0, 2, 3);
        assert!(allowed);
        for _ in 0..2 {
            (bucket, allowed) = take_token(Some(bucket), 0, 2, 3);
            assert!(allowed);
        }
        (bucket, allowed) = take_token(Some(bucket), 0, 2, 3);
        assert!(!allowed, "the bucket is empty");

        // Half a second later one token is back, and only one
        (bucket, allowed) = take_token(Some(bucket), 500, 2, 3);
        assert!(allowed);
        (bucket, allowed) = take_token(Some(bucket), 500, 2, 3);
        assert!(!allowed);

        // A long pause never fills it beyond its size
        (bucket, _) = take_token(Some(bucket), 3_600_000, 2, 3);
        assert_eq!(bucket.tokens, 2.0);
    }

    #[test]
    fn a_stored_bucket_round_trips_and_garbage_is_a_fresh_bucket() {
        let bucket = Bucket {
            tokens: 1.5,
            updated_ms: 1_700_000_000_123,
        };
        assert_eq!(Bucket::parse(&bucket.encode()), Some(bucket));
        assert_eq!(Bucket::parse(b"7"), None);
        assert_eq!(Bucket::parse(b"NaN:1"), None);
    }

    #[test]
    fn the_forwarded_header_is_ignored_unless_the_peer_is_a_trusted_proxy() {
        let trusted = parse_trusted_proxies("10.0.0.0/8, 192.168.1.5");
        let spoofed = headers(&["1.2.3.4"]);
        // Direct connection: the header is the client's own claim
        assert_eq!(
            client_ip(ip("203.0.113.9"), &spoofed, &trusted),
            ip("203.0.113.9")
        );
        assert_eq!(
            client_ip(ip("203.0.113.9"), &spoofed, &[]),
            ip("203.0.113.9")
        );
        // Through the proxy: the address the proxy appended wins over what the client sent
        let through_proxy = headers(&["1.2.3.4, 198.51.100.7"]);
        assert_eq!(
            client_ip(ip("10.1.2.3"), &through_proxy, &trusted),
            ip("198.51.100.7")
        );
        // Two trusted hops, and the header split over two lines
        let two_hops = headers(&["1.2.3.4", "198.51.100.7, 192.168.1.5"]);
        assert_eq!(
            client_ip(ip("10.1.2.3"), &two_hops, &trusted),
            ip("198.51.100.7")
        );
        // No usable entry: fall back to the peer
        assert_eq!(
            client_ip(ip("10.1.2.3"), &headers(&["unknown"]), &trusted),
            ip("10.1.2.3")
        );
        assert_eq!(
            client_ip(ip("10.1.2.3"), &HeaderMap::new(), &trusted),
            ip("10.1.2.3")
        );
    }

    #[test]
    fn cidr_entries_match_by_prefix_and_bad_entries_are_dropped() {
        let trusted = parse_trusted_proxies("10.0.0.0/8, fd00::/8, nonsense, 10.0.0.0/40");
        assert_eq!(trusted.len(), 2);
        assert!(is_trusted(ip("10.255.0.1"), &trusted));
        assert!(!is_trusted(ip("11.0.0.1"), &trusted));
        assert!(is_trusted(ip("fd12::1"), &trusted));
        assert!(!is_trusted(ip("fe80::1"), &trusted));
        // An IPv4-mapped peer is matched as IPv4
        assert!(is_trusted(ip("::ffff:10.1.1.1"), &trusted));
    }

    #[test]
    fn ipv6_clients_share_a_bucket_per_64() {
        assert_eq!(
            bucket_key(ip("2001:db8:1:2:aaaa::1")),
            bucket_key(ip("2001:db8:1:2:bbbb::2"))
        );
        assert_ne!(
            bucket_key(ip("2001:db8:1:2::1")),
            bucket_key(ip("2001:db8:1:3::1"))
        );
        assert_eq!(bucket_key(ip("192.0.2.1")), "192_0_2_1");
        assert_eq!(bucket_key(ip("::ffff:192.0.2.1")), "192_0_2_1");
    }
}
