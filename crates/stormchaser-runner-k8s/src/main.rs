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
use stormchaser_model::events::{
    EventSource, EventType, RunnerEventType, RunnerHeartbeatEvent, RunnerRegisterEvent,
    RunnerStepTypeSchema, SchemaVersion,
};
use stormchaser_model::nats::NatsSubject;
use stormchaser_model::runner::RunnerStatus;
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
    let nats_options = async_nats::ConnectOptions::new().retry_on_initial_connect();
    let nats_client = async_nats::connect_with_options(nats_url, nats_options)
        .await
        .context("Failed to connect to NATS")?;

    // 3. Register with the Orchestration Engine
    let nats_subject = format!("stormchaser.v1.runner.k8s.{}", runner_id);

    // Generate JSON Schemas for our supported step types
    let common_schema = schemars::schema_for!(dsl::CommonContainerSpec);
    let common_schema_json = serde_json::to_value(common_schema)?;
    let k8s_job_schema = schemars::schema_for!(dsl::K8sJobSpec);
    let k8s_job_schema_json = serde_json::to_value(k8s_job_schema)?;

    let registration_payload = RunnerRegisterEvent {
        runner_id: runner_id.clone(),
        runner_type: "k8s".to_string(),
        protocol_version: "v1".to_string(),
        nats_subject: nats_subject.clone(),
        capabilities: vec![
            "k8s".to_string(),
            "docker".to_string(),
            "linux".to_string(),
            "container".to_string(),
        ],
        step_types: vec![
            RunnerStepTypeSchema {
                step_type: "RunContainer".to_string(),
                schema: Some(common_schema_json),
                documentation: Some("Runs a container using a minimal common set of parameters portable across different runners.".to_string()),
            },
            RunnerStepTypeSchema {
                step_type: "RunK8sJob".to_string(),
                schema: Some(k8s_job_schema_json),
                documentation: Some("Runs a native Kubernetes Job with full access to all Job and Pod spec options.".to_string()),
            },
        ],
    };

    let ce = cloudevents::EventBuilderV10::new()
        .id(uuid::Uuid::new_v4().to_string())
        .ty("stormchaser.v1.runner.register")
        .source(EventSource::System.as_str())
        .time(chrono::Utc::now())
        .data(
            stormchaser_model::APPLICATION_JSON,
            serde_json::to_value(registration_payload).unwrap(),
        )
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
    info!("Ensuring JetStream stream 'stormchaser' exists...");
    let stream = js
        .get_or_create_stream(async_nats::jetstream::stream::Config {
            name: "stormchaser".to_string(),
            subjects: vec!["stormchaser.>".to_string()],
            ..Default::default()
        })
        .await
        .context("Failed to ensure JetStream stream")?;

    let runner_consumer = stream
        .create_consumer(async_nats::jetstream::consumer::pull::Config {
            filter_subject: nats_subject.clone(),
            ..Default::default()
        })
        .await
        .context("Failed to create runner-specific JetStream consumer")?;

    let mut runner_task_messages = runner_consumer
        .messages()
        .await
        .context("Failed to get runner-specific consumer messages")?;

    info!("Creating durable consumer for k8s-runner...");
    let consumer = stream
        .get_or_create_consumer(
            "k8s-runner",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("k8s-runner".to_string()),
                filter_subject: "stormchaser.v1.*.step.scheduled.>".to_string(),
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
        "Listening for tasks on {} and JetStream subject stormchaser.v1.*.step.scheduled.>",
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
                let heartbeat_payload = RunnerHeartbeatEvent {
                    runner_id: heartbeat_id.clone(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    state: RunnerStatus::Online,
                };

                if let Err(e) = stormchaser_model::nats::publish_cloudevent(&async_nats::jetstream::new(heartbeat_client.clone()), NatsSubject::RunnerHeartbeat, EventType::Runner(RunnerEventType::Heartbeat), EventSource::System, serde_json::to_value(heartbeat_payload).unwrap(), Some(SchemaVersion::new("1.0".to_string())), None)
                    .await
                {
                    error!("Failed to publish heartbeat: {:?}", e);
                }
            }
            message = runner_task_messages.next() => {
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
                        error!("JetStream runner consumer error: {:?}", e);
                        time::sleep(Duration::from_secs(1)).await;
                    }
                    None => {
                        error!("JetStream runner consumer closed");
                        break;
                    }
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
        NatsSubject::RunnerOffline,
        EventType::Runner(RunnerEventType::Offline),
        EventSource::System,
        serde_json::to_value(deregistration_payload).unwrap(),
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await;

    // Allow time for final messages to be sent
    time::sleep(Duration::from_secs(1)).await;

    health_server.abort();
    info!("Runner stopped");

    Ok(())
}
