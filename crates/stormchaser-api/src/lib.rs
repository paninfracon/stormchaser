pub mod auth;
pub mod db;
pub mod hitl;
pub mod routes;
pub mod telemetry;

use auth::opa::opa_middleware;
pub use auth::{AuthClaims, Claims, JWT_SECRET};
use axum::{
    http::StatusCode,
    middleware,
    routing::{delete, get, post},
    Router,
};
use once_cell::sync::Lazy;
use opentelemetry::{global, metrics::Counter};
use sqlx::PgPool;
use std::sync::Arc;
use stormchaser_model::auth::OpaAuthorizer;
use stormchaser_model::LogBackend;
pub mod rate_limit;

use tower_http::trace::TraceLayer;
use utoipa::OpenApi;

use routes::auth::*;
use routes::cron::*;
use routes::step::*;
use routes::storage::*;
use routes::webhook::*;
use routes::workflow::*;
pub use routes::*;

#[derive(OpenApi)]
#[openapi(
    paths(
        routes::auth::login,
        routes::auth::exchange_token,
        routes::auth::refresh_token,
        routes::workflow::enqueue_workflow,
        routes::workflow::list_workflow_runs,
        routes::workflow::get_workflow_run,
        hitl::approve_step_link
    ),
    components(
        schemas(
            AuthExchangeRequest, AuthExchangeResponse, AuthRefreshRequest,
            EnqueueRequest, EnqueueResponse, RunOverrides,
            ListRunsQuery, WorkflowRunDetail,
            WorkflowRunFullDetail, StepDetail
        )
    ),
    tags(
        (name = "stormchaser", description = "Stormchaser API"),
        (name = "hitl", description = "Human-in-the-Loop")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
pub struct ApiDoc;

pub static RUNS_ENQUEUED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-api")
        .u64_counter("stormchaser.runs_enqueued")
        .with_description("Total number of runs enqueued")
        .build()
});

use tokio::sync::RwLock;

#[derive(Clone)]
pub struct OidcConfig {
    pub issuer: String,
    pub external_issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub jwks_url: String,
}

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub nats: async_nats::Client,
    pub opa: Arc<dyn OpaAuthorizer>,
    pub oidc_config: Option<OidcConfig>,
    pub jwks: Arc<RwLock<std::collections::HashMap<String, jsonwebtoken::jwk::Jwk>>>,
    pub log_backend: Option<LogBackend>,
}

pub async fn fetch_jwks(
    jwks_url: &str,
) -> std::collections::HashMap<String, jsonwebtoken::jwk::Jwk> {
    let mut jwks = std::collections::HashMap::new();
    let retry_policy =
        reqwest_retry::policies::ExponentialBackoff::builder().build_with_max_retries(3);
    let client = reqwest_middleware::ClientBuilder::new(reqwest::Client::new())
        .with(reqwest_retry::RetryTransientMiddleware::new_with_policy(
            retry_policy,
        ))
        .build();

    match client.get(jwks_url).send().await {
        Ok(resp) => {
            if let Ok(jwks_set) = resp.json::<jsonwebtoken::jwk::JwkSet>().await {
                for jwk in jwks_set.keys {
                    if let Some(kid) = &jwk.common.key_id {
                        jwks.insert(kid.clone(), jwk);
                    }
                }
                tracing::info!("Successfully fetched {} keys from JWKS", jwks.len());
            } else {
                tracing::error!("Failed to parse JWKS response from {}", jwks_url);
            }
        }
        Err(e) => {
            tracing::error!("Failed to fetch JWKS from {}: {:?}", jwks_url, e);
        }
    }
    jwks
}

pub fn app(state: AppState) -> Router {
    let per_second = std::env::var("API_RATE_LIMIT_PER_SECOND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let burst_size = std::env::var("API_RATE_LIMIT_BURST_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let rate_limit_state = Arc::new(rate_limit::RateLimitState {
        nats: state.nats.clone(),
        store: Arc::new(tokio::sync::OnceCell::new()),
        per_second,
        burst_size,
    });

    let authenticated_routes = Router::new()
        .route("/runs", get(list_workflow_runs).post(enqueue_workflow))
        .route("/runs/stream", get(stream_workflow_runs_api))
        .route(
            "/runs/:id",
            get(get_workflow_run).delete(delete_workflow_run_api),
        )
        .route("/runs/:id/steps/:step_id/approve", post(hitl::approve_step))
        .route("/runs/:id/steps/:step_id/reject", post(hitl::reject_step))
        .route("/events/correlate", post(hitl::correlate_event))
        .route("/runs/:id/artifacts", get(list_run_artifacts))
        .route("/runs/:id/reports", get(list_run_test_reports))
        .route("/runs/:id/summaries", get(list_run_test_summaries))
        .route("/runs/:id/reports/:report_id", get(get_test_report))
        .route(
            "/runs/:id/steps/:step_name/logs/stream",
            get(stream_step_logs_api),
        )
        .route("/runs/:id/logs/stream", get(stream_run_logs_api))
        .route("/runs/:id/status/stream", get(stream_run_status_api))
        .route("/runs/direct", post(direct_run))
        .route("/webhooks", get(list_webhooks).post(create_webhook))
        .route("/webhooks/:id", get(get_webhook).delete(delete_webhook))
        .route(
            "/cron-workflows",
            get(list_cron_workflows).post(create_cron_workflow),
        )
        .route("/cron-workflows/:id", delete(delete_cron_workflow))
        .route("/rules", get(list_event_rules).post(create_event_rule))
        .route("/rules/:id", delete(delete_event_rule))
        .route(
            "/storage-backends",
            get(list_storage_backends).post(create_storage_backend),
        )
        .route(
            "/storage-backends/:id",
            get(get_storage_backend)
                .patch(update_storage_backend)
                .delete(delete_storage_backend),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            opa_middleware,
        ));

    let api_v1 = Router::new()
        .merge(authenticated_routes)
        .route("/webhooks/:id", post(handle_webhook))
        .route("/auth/login", get(login))
        .route("/auth/exchange", post(exchange_token))
        .route("/auth/refresh", post(refresh_token))
        .route("/approve-link/:token", get(hitl::approve_step_link))
        .route("/cron-trigger/:id", post(trigger_cron_workflow))
        .layer(middleware::from_fn_with_state(
            rate_limit_state,
            rate_limit::nats_rate_limiter,
        ))
        .layer(middleware::from_fn(
            |mut req: axum::extract::Request, next: middleware::Next| async move {
                if req
                    .extensions()
                    .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                    .is_none()
                {
                    req.extensions_mut().insert(axum::extract::ConnectInfo(
                        std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
                    ));
                }
                Ok::<_, StatusCode>(next.run(req).await)
            },
        ));

    Router::new()
        .merge(
            utoipa_swagger_ui::SwaggerUi::new("/swagger-ui")
                .url("/api-docs/openapi.json", ApiDoc::openapi()),
        )
        .route("/", get(|| async { "Stormchaser API" }))
        .route("/healthz", get(|| async { "OK" }))
        .route("/api/health", get(|| async { "OK" }))
        .nest("/api/v1", api_v1)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
