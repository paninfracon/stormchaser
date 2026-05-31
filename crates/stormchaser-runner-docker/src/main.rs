use anyhow::{Context, Result};
use axum::{extract::State, routing::get, Router};
use bollard::Docker;
use cloudevents::EventBuilder;
use futures::StreamExt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use stormchaser_model::events::{
    EventSource, EventType, RunnerEventType, RunnerHeartbeatEvent, RunnerRegisterEvent,
    RunnerStepTypeSchema, SchemaVersion,
};
use stormchaser_model::nats::{publish_cloudevent, NatsSubject};
use stormchaser_model::runner::RunnerStatus;
use tokio::sync::watch;
use tokio::time;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

use stormchaser_model::{dsl, APPLICATION_JSON};

pub mod container_machine;
pub mod handler;
pub mod parsing;
pub mod reaper;
pub mod traits;

use crate::handler::{handle_task, scan_for_orphans};
use crate::reaper::run_reaper;

/// Configuration for the Stormchaser Docker Runner.
#[derive(Debug, Clone)]
pub struct Config {
    pub nats_url: String,
    pub runner_id: String,
    pub encryption_key: Option<String>,
    pub loki_url: Option<String>,
    pub rust_log: String,
}

impl Config {
    /// Creates a new `Config` from an environment variable iterator.
    pub fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut runner_id = Uuid::new_v4().to_string();
        let mut encryption_key = None;
        let mut loki_url = None;
        let mut rust_log = "stormchaser_runner_docker=info".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUNNER_ID" => runner_id = v.as_ref().to_string(),
                "STORMCHASER_STATE_ENCRYPTION_KEY" => encryption_key = Some(v.as_ref().to_string()),
                "LOKI_URL" => loki_url = Some(v.as_ref().to_string()),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        Self {
            nats_url,
            runner_id,
            encryption_key,
            loki_url,
            rust_log,
        }
    }
}

struct AppState {
    is_ready: watch::Receiver<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env(std::env::vars());
    run_runner(config).await
}

/// Main entry point for the Stormchaser Docker Runner.
/// Initializes the Docker client, connects to NATS, registers the runner,
/// and begins processing tasks from the JetStream queue.
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
        warn!("State encryption is DISABLED. Sensitive step data in Docker labels will be stored in plaintext.");
    }

    info!(
        "Starting Stormchaser Docker Runner {} (rev: {}, branch: {}, built: {}): {}",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_GIT_SHA"),
        env!("VERGEN_GIT_BRANCH"),
        env!("VERGEN_BUILD_TIMESTAMP"),
        runner_id
    );

    // Initialize Docker client
    let docker =
        Docker::connect_with_local_defaults().context("Failed to connect to Docker daemon")?;

    // Watch channel for readiness state
    let (ready_tx, ready_rx) = watch::channel(false);

    // 1. Start Health Check Server
    let health_state = Arc::new(AppState { is_ready: ready_rx });

    let reaper_docker = docker.clone();
    tokio::spawn(async move {
        if let Err(e) = run_reaper(reaper_docker).await {
            error!("Reaper error: {:?}", e);
        }
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

    let health_addr = SocketAddr::from(([0, 0, 0, 0], 8081)); // Different port from K8s runner
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
    let nats_subject = format!("stormchaser.v1.runner.docker.{}", runner_id);

    // Generate JSON Schema for our supported step type
    let common_schema = schemars::schema_for!(dsl::CommonContainerSpec);
    let common_schema_json = serde_json::to_value(common_schema)?;

    let registration_payload = RunnerRegisterEvent {
        runner_id: runner_id.clone(),
        runner_type: "docker".to_string(),
        protocol_version: "v1".to_string(),
        nats_subject: nats_subject.clone(),
        capabilities: vec![
            "docker".to_string(),
            "linux".to_string(),
            "container".to_string(),
        ],
        step_types: vec![RunnerStepTypeSchema {
            step_type: "RunContainer".to_string(),
            schema: Some(common_schema_json),
            documentation: Some(
                "Runs a container using Docker with a minimal common set of parameters."
                    .to_string(),
            ),
        }],
    };

    let ce = cloudevents::EventBuilderV10::new()
        .id(uuid::Uuid::new_v4().to_string())
        .ty("stormchaser.v1.runner.register")
        .source(EventSource::System.as_str())
        .time(chrono::Utc::now())
        .data(
            APPLICATION_JSON,
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

    // Scan for orphaned containers
    let _ = scan_for_orphans(
        docker.clone(),
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

    info!("Creating durable consumer for docker-runner...");
    let consumer = stream
        .get_or_create_consumer(
            "docker-runner",
            async_nats::jetstream::consumer::pull::Config {
                durable_name: Some("docker-runner".to_string()),
                filter_subject: "stormchaser.v1.*.step.scheduled.runcontainer".to_string(),
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
        "Listening for tasks on {} and JetStream subject stormchaser.v1.*.step.scheduled.runcontainer",
        nats_subject
    );

    // 6. Main event loop
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

                if let Err(e) = publish_cloudevent(&async_nats::jetstream::new(heartbeat_client.clone()), NatsSubject::RunnerHeartbeat, EventType::Runner(RunnerEventType::Heartbeat), EventSource::System, serde_json::to_value(heartbeat_payload).unwrap(), Some(SchemaVersion::new("1.0".to_string())), None)
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
                            docker.clone(),
                            nats_client.clone(),
                            runner_id.clone(),
                            encryption_key.clone(),
                            config.loki_url.clone(),
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
                            docker.clone(),
                            nats_client.clone(),
                            runner_id.clone(),
                            encryption_key.clone(),
                            config.loki_url.clone(),
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

    health_server.abort();
    info!("Runner stopped");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_from_env_defaults() {
        let env: Vec<(String, String)> = vec![];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://localhost:4222");
        assert!(config.encryption_key.is_none());
        assert_eq!(config.rust_log, "stormchaser_runner_docker=info");
        // runner_id is random, just assert it's not empty
        assert!(!config.runner_id.is_empty());
    }

    #[test]
    fn test_config_from_env_custom() {
        let env = vec![
            ("NATS_URL".to_string(), "nats://remote:4222".to_string()),
            ("RUNNER_ID".to_string(), "my-runner".to_string()),
            (
                "STORMCHASER_STATE_ENCRYPTION_KEY".to_string(),
                "my-key".to_string(),
            ),
            ("RUST_LOG".to_string(), "debug".to_string()),
        ];
        let config = Config::from_env(env);
        assert_eq!(config.nats_url, "nats://remote:4222");
        assert_eq!(config.runner_id, "my-runner");
        assert_eq!(config.encryption_key.unwrap(), "my-key");
        assert_eq!(config.rust_log, "debug");
    }
}
