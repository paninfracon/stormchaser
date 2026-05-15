//! Stormchaser API implementation.
//! This crate contains the REST API for the Stormchaser system.

/// Authentication and authorization module
pub mod auth;
/// Database access module
pub mod db;
/// Human-in-the-loop module
pub mod hitl;
/// API routes module
pub mod routes;
/// Telemetry and metrics module
pub mod telemetry;

use async_nats::Client;
use auth::opa::opa_middleware;
pub use auth::{AuthClaims, Claims, JWT_SECRET};
use axum::extract;
use axum::{
    http::StatusCode,
    middleware,
    routing::{delete, get, post},
    Router,
};
use once_cell::sync::Lazy;
use opentelemetry::{global, metrics::Counter};
use sqlx::PgPool;
use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use stormchaser_model::auth::OpaAuthorizer;
use stormchaser_model::connections::ArtifactRegistry;
use stormchaser_model::connections::Connection;
use stormchaser_model::connections::ConnectionType;
use stormchaser_model::cron::CronWorkflow;
use stormchaser_model::event_rules::EventRule;
use stormchaser_model::event_rules::WebhookConfig;
use stormchaser_model::test_report::TestCase;
use stormchaser_model::test_report::TestCaseStatus;
use stormchaser_model::test_report::TestReport;
use stormchaser_model::test_report::TestSummary;
use stormchaser_model::LogBackend;
use tokio::sync;
/// Rate limiting middleware and configuration
pub mod rate_limit;

use tower_http::trace::TraceLayer;
use utoipa::OpenApi;

use routes::auth::*;
use routes::connections::*;
use routes::cron::*;
use routes::event_rule::*;
#[cfg(feature = "mcp")]
use routes::mcp::*;
use routes::step::*;
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
        routes::workflow::delete_workflow_run_api,
        routes::workflow::direct_run,
        routes::workflow::stream_workflow_runs_api,
        routes::cron::create_cron_workflow,
        routes::cron::list_cron_workflows,
        routes::cron::delete_cron_workflow,
        routes::cron::trigger_cron_workflow,
        routes::connections::create_connection,
        routes::connections::list_connections,
        routes::connections::get_connection,
        routes::connections::update_connection,
        routes::connections::delete_connection,
        routes::connections::test_connection,
        routes::connections::list_run_artifacts,
        routes::connections::list_run_test_reports,
        routes::connections::list_run_test_summaries,
        routes::connections::get_test_report,
        routes::webhook::create_webhook,
        routes::webhook::list_webhooks,
        routes::webhook::get_webhook,
        routes::webhook::delete_webhook,
        routes::event_rule::create_event_rule,
        routes::event_rule::list_event_rules,
        routes::event_rule::delete_event_rule,
        routes::webhook::handle_webhook,
        routes::step::stream_step_logs_api,
        routes::step::get_step_logs_api,
        routes::step::stream_run_logs_api,
        routes::step::stream_run_status_api,
        routes::schema::get_schema,
        hitl::approve_step_link
    ),
    components(
        schemas(
            AuthExchangeRequest, AuthExchangeResponse, AuthRefreshRequest,
            EnqueueRequest, EnqueueResponse, RunOverrides,
            ListRunsQuery, WorkflowRunDetail,
            WorkflowRunFullDetail, StepDetail,
            CreateCronWorkflowRequest, CronWorkflowResponse,
            CronWorkflow,
            CreateStorageBackendRequest, UpdateStorageBackendRequest,
            Connection, ConnectionType,
            ArtifactRegistry,
            TestCase, TestCaseStatus,
            TestSummary, TestReport,
            CreateWebhookRequest, CreateEventRuleRequest,
            WebhookConfig, EventRule,
            DirectRunRequest
        )
    ),
    tags(
        (name = "stormchaser", description = "Stormchaser API"),
        (name = "hitl", description = "Human-in-the-Loop"),
        (name = "cron", description = "Cron workflows"),
        (name = "storage", description = "Storage and artifacts"),
        (name = "webhook", description = "Webhooks and rules"),
        (name = "event_rule", description = "Event rules"),
        (name = "step", description = "Step actions"),
        (name = "workflow", description = "Workflow actions")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
/// OpenAPI documentation struct for the API
pub struct ApiDoc;

/// Counter metric for tracking the total number of enqueued workflow runs
pub static RUNS_ENQUEUED: Lazy<Counter<u64>> = Lazy::new(|| {
    global::meter("stormchaser-api")
        .u64_counter("stormchaser.v1.runs_enqueued")
        .with_description("Total number of runs enqueued")
        .build()
});

use tokio::sync::RwLock;

/// Application state shared across routes
#[derive(Clone)]
pub struct AppState {
    /// Database connection pool
    pub pool: PgPool,
    /// NATS client connection
    pub nats: Client,
    /// OPA authorizer
    pub opa: Arc<dyn OpaAuthorizer>,
    /// Optional OIDC configuration
    pub oidc_config: Option<auth::jwks::OidcConfig>,
    /// JWKS cache for token validation
    pub jwks: Arc<RwLock<auth::jwks::JwksCache>>,
    /// Optional backend for logging
    pub log_backend: Option<LogBackend>,
    /// API base URL for MCP tool callback
    pub api_base_url: String,
}

/// Constructs the Axum application router with all routes and middleware
pub fn app(state: AppState) -> Router {
    let per_second = env::var("API_RATE_LIMIT_PER_SECOND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let burst_size = env::var("API_RATE_LIMIT_BURST_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let rate_limit_state = Arc::new(rate_limit::RateLimitState {
        nats: state.nats.clone(),
        store: Arc::new(sync::OnceCell::new()),
        per_second,
        burst_size,
    });

    #[allow(unused_mut)]
    let mut authenticated_routes = Router::new()
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
            "/runs/:id/steps/:step_id/logs/stream",
            get(stream_step_logs_api),
        )
        .route("/runs/:id/steps/:step_id/logs", get(get_step_logs_api))
        .route("/runs/:id/logs/stream", get(stream_run_logs_api))
        .route("/runs/:id/status/stream", get(stream_run_status_api))
        .route("/runs/direct", post(direct_run))
        .route("/webhooks", get(list_webhooks).post(create_webhook))
        .route(
            "/webhooks/:id",
            get(get_webhook)
                .patch(update_webhook)
                .delete(delete_webhook),
        )
        .route(
            "/cron-workflows",
            get(list_cron_workflows).post(create_cron_workflow),
        )
        .route("/cron-workflows/:id", delete(delete_cron_workflow))
        .route("/rules", get(list_event_rules).post(create_event_rule))
        .route("/rules/:id", delete(delete_event_rule))
        .route(
            "/connections",
            get(list_connections).post(create_connection),
        )
        .route(
            "/connections/:id",
            get(get_connection)
                .patch(update_connection)
                .delete(delete_connection),
        )
        .route("/connections/test", post(test_connection));

    #[cfg(feature = "mcp")]
    {
        if let Some(service) = mcp_service(&state.api_base_url) {
            authenticated_routes = authenticated_routes.nest_service("/mcp", service);
        }
    }

    let authenticated_routes = authenticated_routes.layer(middleware::from_fn_with_state(
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
        .route(
            "/cron-trigger/:id",
            post(routes::cron::trigger_cron_workflow),
        )
        .route("/schema", get(routes::schema::get_schema))
        .layer(middleware::from_fn_with_state(
            rate_limit_state,
            rate_limit::nats_rate_limiter,
        ))
        .layer(middleware::from_fn(
            |mut req: extract::Request, next: middleware::Next| async move {
                if req
                    .extensions()
                    .get::<extract::ConnectInfo<SocketAddr>>()
                    .is_none()
                {
                    req.extensions_mut()
                        .insert(extract::ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
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
