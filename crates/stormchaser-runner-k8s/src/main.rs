mod cluster;
mod config;
mod job_machine;
mod reaper;
mod scanner;
mod task;

use anyhow::{Context, Result};
use axum::{extract::State, routing::get, Router};
use cloudevents::EventBuilder;
use futures::StreamExt;
use kube::Client;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use stormchaser_model::dsl;

use crate::cluster::ClusterPool;
use crate::config::Config;
use crate::reaper::run_reaper;
use crate::scanner::scan_for_orphans;
use crate::task::handle_task;

pub struct AppState {
    is_ready: watch::Receiver<bool>,
    #[allow(dead_code)]
    cluster_pool: Arc<ClusterPool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env(std::env::vars());
    run_runner(config).await
}

/// Run runner.
pub async fn run_runner(config: Config) -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install default crypto provider");

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(&config.rust_log))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let nats_url = config.nats_url;
    let runner_id = config.runner_id;
    let encryption_key = config.encryption_key;

    if encryption_key.is_some() {
        info!("State encryption is enabled");
    } else {
        warn!("State encryption is DISABLED. Sensitive step data in K8s annotations will be stored in plaintext.");
    }

    info!(
        "Starting Stormchaser K8s Runner {} (rev: {}, branch: {}, built: {}): {}",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP"),
        runner_id
    );

    // Initialize Kubernetes Cluster Pool
    let cluster_pool = Arc::new(ClusterPool::new());

    // Initialize default "local" client with versioning
    let local_client = Client::try_default()
        .await
        .context("Failed to initialize local Kubernetes client")?;

    let version_resp = local_client
        .request_text(http::Request::builder().uri("/version").body(vec![])?)
        .await?;
    let version_data: Value = serde_json::from_str(&version_resp)?;
    let major = version_data["major"].as_str().unwrap_or("0");
    let minor = version_data["minor"].as_str().unwrap_or("0");
    let local_version = format!("{}.{}", major, minor.replace('+', ""));

    cluster_pool.add_client("local", local_client, local_version);
    info!("Default 'local' Kubernetes client initialized");

    // Watch channel for readiness state
    let (ready_tx, ready_rx) = watch::channel(false);

    // 1. Start Health Check Server
    let health_state = Arc::new(AppState {
        is_ready: ready_rx,
        cluster_pool: cluster_pool.clone(),
    });

    let app = Router::new()
        .route("/healthz", get(|| async { "OK" }))
        .route(
            "/readyz",
            get(|state: State<Arc<AppState>>| async move {
                if *state.is_ready.borrow() {
                    axum::http::StatusCode::OK
                } else {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        )
        .with_state(health_state);

    let health_addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = tokio::net::TcpListener::bind(health_addr).await?;
    info!("Health server listening on {}", health_addr);

    let health_server = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("Health server error: {:?}", e);
        }
    });

    // 2. Connect to NATS
    let nats_client = async_nats::connect(nats_url)
        .await
        .context("Failed to connect to NATS")?;

    // 3. Register with the Orchestration Engine
    let nats_subject = format!("stormchaser.v1.runner.k8s.{}", runner_id);

    // Generate JSON Schemas for our supported step types
    let common_schema = schemars::schema_for!(dsl::CommonContainerSpec);
    let common_schema_json = serde_json::to_value(common_schema)?;
    let k8s_job_schema = schemars::schema_for!(dsl::K8sJobSpec);
    let k8s_job_schema_json = serde_json::to_value(k8s_job_schema)?;

    let registration_payload = json!({
        "runner_id": runner_id,
        "runner_type": "k8s",
        "protocol_version": "v1",
        "nats_subject": nats_subject,
        "capabilities": ["k8s", "docker", "linux", "container"],
        "step_types": [
            {
                "step_type": "RunContainer",
                "schema": common_schema_json,
                "documentation": "Runs a container using a minimal common set of parameters portable across different runners."
            },
            {
                "step_type": "RunK8sJob",
                "schema": k8s_job_schema_json,
                "documentation": "Runs a native Kubernetes Job with full access to all Job and Pod spec options."
            }
        ]
    });

    let ce = cloudevents::EventBuilderV10::new()
        .id(uuid::Uuid::new_v4().to_string())
        .ty("stormchaser.v1.runner.register")
        .source("/stormchaser")
        .time(chrono::Utc::now())
        .data("application/json", registration_payload)
        .build()
        .context("Failed to build CloudEvent")?;

    let payload_bytes = serde_json::to_vec(&ce).context("Failed to serialize CloudEvent")?;

    nats_client
        .publish("stormchaser.v1.runner.register", payload_bytes.into())
        .await
        .context("Failed to publish registration event")?;

    info!("Runner registered successfully");

    // 3.4 Start garbage collection (reaper) loop
    let reaper_pool = cluster_pool.clone();
    tokio::spawn(async move {
        if let Err(e) = run_reaper(reaper_pool).await {
            error!("Reaper error: {:?}", e);
        }
    });

    // 3.5 Scan for orphaned jobs
    let _ = scan_for_orphans(
        cluster_pool.clone(),
        nats_client.clone(),
        runner_id.clone(),
        encryption_key.clone(),
    )
    .await;

    let _ = ready_tx.send(true);

    // 4. Start heartbeat loop
    let heartbeat_client = nats_client.clone();
    let heartbeat_id = runner_id.clone();
    let mut heartbeat_interval = time::interval(Duration::from_secs(10));

    // 5. Subscribe to task subjects
    let js = async_nats::jetstream::new(nats_client.clone());
    // 5a. Specific subject for this runner instance
    let mut runner_subscriber = nats_client.subscribe(nats_subject.clone()).await?;

    info!("Ensuring JetStream stream 'stormchaser' exists...");
    let stream = js
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: "stormchaser".to_string(),
            subjects: vec!["stormchaser.>".to_string()],
            ..Default::default()
        })
        .await
        .context("Failed to ensure JetStream stream")?;

    info!("Creating durable consumer for k8s-runner...");
    let consumer = stream
        .get_or_create_consumer(
            "k8s-runner",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("k8s-runner".to_string()),
                filter_subject: "stormchaser.v1.step.scheduled.>".to_string(),
                ..Default::default()
            },
        )
        .await
        .context("Failed to create JetStream consumer")?;

    let mut task_messages = consumer
        .messages()
        .await
        .context("Failed to get consumer messages")?;

    info!(
        "Listening for tasks on {} and JetStream subject stormchaser.step.scheduled.>",
        nats_subject
    );

    // 6. Main event loop with signal handling
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut sigint = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                info!("SIGTERM received, shutting down...");
                break;
            }
            _ = sigint.recv() => {
                info!("SIGINT received, shutting down...");
                break;
            }
            _ = heartbeat_interval.tick() => {
                let heartbeat_payload = stormchaser_model::events::RunnerHeartbeatEvent {
                    runner_id: heartbeat_id.clone(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    state: "online".to_string(),
                };

                if let Err(e) = stormchaser_model::nats::publish_cloudevent(&async_nats::jetstream::new(heartbeat_client.clone()), "stormchaser.v1.runner.heartbeat", "stormchaser.v1.runner.heartbeat", "/stormchaser", serde_json::to_value(heartbeat_payload).unwrap(), Some("1.0"), None)
                    .await
                {
                    error!("Failed to publish heartbeat: {:?}", e);
                }
            }
            message = runner_subscriber.next() => {
                if let Some(msg) = message {
                    info!("Received runner-specific message: {:?}", msg.payload);
                }
            }
            message = task_messages.next() => {
                match message {
                    Some(Ok(msg)) => {
                        tokio::spawn(handle_task(
                            msg,
                            cluster_pool.clone(),
                            nats_client.clone(),
                            runner_id.clone(),
                            encryption_key.clone(),
                        ));
                    }
                    Some(Err(e)) => {
                        error!("JetStream consumer error: {:?}", e);
                        time::sleep(Duration::from_secs(1)).await;
                    }
                    None => {
                        error!("JetStream consumer closed");
                        break;
                    }
                }
            }
        }
    }

    // 7. Cleanup
    info!("Graceful shutdown initiated...");
    let _ = ready_tx.send(false);

    // Deregister (optional but polite)
    let deregistration_payload = json!({
        "runner_id": runner_id,
        "event_type": "runner_offline",
    });
    let _ = stormchaser_model::nats::publish_cloudevent(
        &async_nats::jetstream::new(nats_client.clone()),
        "stormchaser.v1.runner.offline",
        "stormchaser.v1.runner.offline",
        "/stormchaser",
        deregistration_payload,
        Some("1.0"),
        None,
    )
    .await;

    // Allow time for final messages to be sent
    time::sleep(Duration::from_secs(1)).await;

    health_server.abort();
    info!("Runner stopped");

    Ok(())
}
