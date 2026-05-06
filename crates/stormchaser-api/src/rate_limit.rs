use async_nats::jetstream;
use async_nats::jetstream::kv::Config;
use async_nats::Client;
use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use chrono::Utc;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;

/// State for the NATS-backed rate limiter
#[derive(Clone)]
pub struct RateLimitState {
    /// NATS client connection
    pub nats: Client,
    /// Lazy initialized Key-Value store for rate limiting
    pub store: Arc<OnceCell<jetstream::kv::Store>>,
    /// Allowed requests per second
    pub per_second: u64,
    /// Maximum burst size for requests
    pub burst_size: u64,
}

/// Middleware that limits request rates using NATS KV
pub async fn nats_rate_limiter(
    State(state): State<Arc<RateLimitState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request,
    next: Next,
) -> Response {
    if env::var("TEST_BYPASS_RATE_LIMIT").is_ok() {
        return next.run(req).await;
    }

    let ip = addr.ip().to_string();
    let current_second = Utc::now().timestamp();
    let ip_safe = ip.replace(['.', ':'], "_");
    let key = format!("{}_{}", ip_safe, current_second);

    let store = match state
        .store
        .get_or_try_init(|| async {
            let js = jetstream::new(state.nats.clone());
            js.create_key_value(Config {
                bucket: "api_rate_limits".to_string(),
                max_age: Duration::from_secs(60),
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

        match store.entry(&key).await {
            Ok(Some(entry)) => {
                let count: u64 = std::str::from_utf8(&entry.value)
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);
                if count >= state.burst_size {
                    break false;
                }
                let next_count = count + 1;
                let next_val: Bytes = next_count.to_string().into();
                match store.update(&key, next_val, entry.revision).await {
                    Ok(_) => break true,
                    Err(_) => {
                        retries += 1;
                        continue;
                    }
                }
            }
            Ok(None) => {
                let next_val: Bytes = "1".into();
                match store.update(&key, next_val, 0).await {
                    Ok(_) => break true,
                    Err(_) => {
                        retries += 1;
                        continue;
                    }
                }
            }
            Err(e) => {
                tracing::error!("NATS KV rate limit error: {:?}", e);
                break true; // fail open
            }
        }
    };

    if !allowed {
        return (StatusCode::TOO_MANY_REQUESTS, "Too Many Requests").into_response();
    }

    next.run(req).await
}
