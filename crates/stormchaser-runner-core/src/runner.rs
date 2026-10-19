//! A runner's life, the same for every kind of runner: it connects to NATS, obtains its
//! key, registers, adopts the steps a previous instance left running, then pulls tasks
//! and cancellations and sends heartbeats until it is told to stop.

use anyhow::{Context, Result};
use async_nats::jetstream::Message;
use async_trait::async_trait;
use axum::{extract::State, routing::get, Router};
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
use tracing::{error, info, warn};

/// How often a runner tells the engine it is alive.
const HEARTBEAT: Duration = Duration::from_secs(10);

/// What a runner needs to start: where NATS is, who it is, and the key it encrypts the
/// step definitions it keeps on its workloads with.
#[derive(Debug, Clone)]
pub struct RunnerSettings {
    /// The NATS server URL.
    pub nats_url: String,
    /// This runner's id.
    pub runner_id: String,
    /// `STORMCHASER_STATE_ENCRYPTION_KEY`, if set.
    pub encryption_key: Option<String>,
}

/// What a backend's handlers get from the runner around them.
#[derive(Debug, Clone)]
pub struct RunnerContext {
    /// The NATS connection.
    pub nats: async_nats::Client,
    /// This runner's id.
    pub runner_id: String,
    /// `STORMCHASER_STATE_ENCRYPTION_KEY`, if set.
    pub encryption_key: Option<String>,
}

/// One kind of runner: where it runs steps, and how.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    /// The runner's kind as it registers and names its consumer: `docker` or `k8s`.
    fn kind(&self) -> &'static str;
    /// The port of the runner's `/healthz` and `/readyz` endpoints.
    fn health_port(&self) -> u16;
    /// What the runner tells the engine it can do.
    fn capabilities(&self) -> Vec<String>;
    /// The step types the runner runs, with their spec schemas.
    fn step_types(&self) -> Result<Vec<RunnerStepTypeSchema>>;
    /// The shared consumers the runner pulls tasks from, as (consumer, subject).
    fn task_consumers(&self) -> Vec<(&'static str, &'static str)>;
    /// Removes what is left of old workloads, for as long as the runner runs.
    async fn reap(self: Arc<Self>);
    /// Picks up, or cleans up, the workloads a previous instance of this runner left.
    async fn adopt_orphans(&self, ctx: &RunnerContext) -> Result<()>;
    /// Runs the step a task message names, and reports its result.
    async fn handle_task(self: Arc<Self>, msg: Message, ctx: RunnerContext);
    /// Stops the workloads of the run a cancellation names.
    async fn handle_cancel(self: Arc<Self>, msg: Message);
}

/// Runs `backend` until the runner receives SIGTERM or SIGINT, or loses a consumer.
pub async fn run<B: Backend>(backend: Arc<B>, settings: RunnerSettings) -> Result<()> {
    let RunnerSettings {
        nats_url,
        runner_id,
        encryption_key,
    } = settings;
    let kind = backend.kind();

    if encryption_key.is_some() {
        info!("State encryption is enabled");
    } else {
        warn!(
            "State encryption is DISABLED. Sensitive step data kept on {} workloads is stored in plaintext.",
            kind
        );
    }

    let (ready_tx, ready_rx) = watch::channel(false);
    let health_server = serve_health(backend.health_port(), ready_rx).await?;

    tokio::spawn(backend.clone().reap());

    let nats_options = stormchaser_model::nats::apply_auth_from_env(
        async_nats::ConnectOptions::new().retry_on_initial_connect(),
    )
    .await?;
    let nats = async_nats::connect_with_options(nats_url, nats_options)
        .await
        .context("Failed to connect to NATS")?;

    // A key of this runner's own, which signs everything it reports from here on
    stormchaser_model::runner_auth::obtain_session(&nats, &runner_id).await?;

    let own_subject = format!("stormchaser.v1.runner.{}.{}", kind, runner_id);
    register(&nats, &*backend, &runner_id, &own_subject).await?;
    info!("Runner registered successfully");

    let ctx = RunnerContext {
        nats: nats.clone(),
        runner_id: runner_id.clone(),
        encryption_key,
    };
    if let Err(e) = backend.adopt_orphans(&ctx).await {
        error!("Could not adopt orphaned workloads: {:?}", e);
    }

    let _ = ready_tx.send(true);

    let js = async_nats::jetstream::new(nats.clone());
    info!("Waiting for the JetStream stream 'stormchaser'...");
    let stream = stormchaser_model::nats::wait_for_stream(&js).await;

    let mut own_tasks = stream
        .create_consumer(async_nats::jetstream::consumer::pull::Config {
            // Named so the create request carries the filter in its subject, which lets
            // NATS permissions limit runners to consumers on their own task subjects.
            name: Some(format!("runner-{}-{}", kind, runner_id)),
            filter_subject: own_subject.clone(),
            ..Default::default()
        })
        .await
        .context("Failed to create runner-specific JetStream consumer")?
        .messages()
        .await
        .context("Failed to get runner-specific consumer messages")?;

    // Shared consumers, so that each task goes to one runner
    let mut shared = Vec::new();
    for (consumer, subject) in backend.task_consumers() {
        shared.push(
            stormchaser_model::runner::task_consumer(&stream, consumer, subject)
                .await?
                .messages()
                .await
                .with_context(|| format!("Failed to get messages of consumer {}", consumer))?,
        );
    }
    let mut shared_tasks = futures::stream::select_all(shared);

    // Cancellations of runs, which every runner reads for itself
    let mut cancellations = stormchaser_model::run_cancel::subscribe(&stream).await?;

    info!(
        "Listening for tasks on {} and {:?}",
        own_subject,
        backend.task_consumers()
    );

    let mut heartbeat = tokio::time::interval(HEARTBEAT);
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
            _ = heartbeat.tick() => send_heartbeat(&js, &runner_id).await,
            message = cancellations.next() => match message {
                Some(Ok(msg)) => {
                    tokio::spawn(backend.clone().handle_cancel(msg));
                }
                Some(Err(e)) => {
                    error!("Run cancellation consumer error: {:?}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                None => {
                    error!("Run cancellation consumer closed");
                    break;
                }
            },
            message = own_tasks.next() => match message {
                Some(Ok(msg)) => {
                    tokio::spawn(backend.clone().handle_task(msg, ctx.clone()));
                }
                Some(Err(e)) => {
                    error!("JetStream runner consumer error: {:?}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                None => {
                    error!("JetStream runner consumer closed");
                    break;
                }
            },
            message = shared_tasks.next() => match message {
                Some(Ok(msg)) => {
                    tokio::spawn(backend.clone().handle_task(msg, ctx.clone()));
                }
                Some(Err(e)) => {
                    error!("JetStream consumer error: {:?}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
                None => {
                    error!("JetStream consumer closed");
                    break;
                }
            },
        }
    }

    info!("Graceful shutdown initiated...");
    let _ = ready_tx.send(false);
    // Tells the engine at once, rather than when the heartbeats stop
    let offline = serde_json::json!({
        "runner_id": runner_id,
        "event_type": "runner_offline",
    });
    let _ = publish_cloudevent(
        &js,
        NatsSubject::RunnerOffline,
        EventType::Runner(RunnerEventType::Offline),
        EventSource::System,
        offline,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await;
    // Time for the last messages to go out
    tokio::time::sleep(Duration::from_secs(1)).await;

    health_server.abort();
    info!("Runner stopped");
    Ok(())
}

/// Serves `/healthz`, and `/readyz` once the runner is registered.
async fn serve_health(
    port: u16,
    ready: watch::Receiver<bool>,
) -> Result<tokio::task::JoinHandle<()>> {
    let app = Router::new()
        .route("/healthz", get(|| async { "OK" }))
        .route(
            "/readyz",
            get(|State(ready): State<watch::Receiver<bool>>| async move {
                if *ready.borrow() {
                    axum::http::StatusCode::OK
                } else {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        )
        .with_state(ready);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("Health server listening on {}", addr);
    Ok(tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            error!("Health server error: {:?}", e);
        }
    }))
}

/// Tells the engine this runner exists, what it runs, and where to send its tasks.
async fn register<B: Backend>(
    nats: &async_nats::Client,
    backend: &B,
    runner_id: &str,
    own_subject: &str,
) -> Result<()> {
    const SUBJECT: &str = "stormchaser.v1.runner.register";
    let registration = RunnerRegisterEvent {
        runner_id: runner_id.to_string(),
        runner_type: backend.kind().to_string(),
        protocol_version: "v1".to_string(),
        nats_subject: own_subject.to_string(),
        capabilities: backend.capabilities(),
        step_types: backend.step_types()?,
    };
    let event = cloudevents::EventBuilderV10::new()
        .id(uuid::Uuid::new_v4().to_string())
        .ty(SUBJECT)
        .source(EventSource::System.as_str())
        .time(chrono::Utc::now())
        .data(
            stormchaser_model::APPLICATION_JSON,
            stormchaser_model::runner_auth::sign_if_runner(
                serde_json::to_value(registration)?,
                SUBJECT,
            ),
        )
        .build()
        .context("Failed to build CloudEvent")?;
    nats.publish(SUBJECT, serde_json::to_vec(&event)?.into())
        .await
        .context("Failed to publish registration event")?;
    Ok(())
}

async fn send_heartbeat(js: &async_nats::jetstream::Context, runner_id: &str) {
    let heartbeat = RunnerHeartbeatEvent {
        runner_id: runner_id.to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        state: RunnerStatus::Online,
    };
    let Ok(payload) = serde_json::to_value(heartbeat) else {
        return;
    };
    if let Err(e) = publish_cloudevent(
        js,
        NatsSubject::RunnerHeartbeat,
        EventType::Runner(RunnerEventType::Heartbeat),
        EventSource::System,
        payload,
        Some(SchemaVersion::new("1.0".to_string())),
        None,
    )
    .await
    {
        error!("Failed to publish heartbeat: {:?}", e);
    }
}
