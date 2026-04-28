use axum::extract::State;
use serde_json::Value;
use std::collections::HashMap;
use tokio::time::sleep;
mod job_machine;

use anyhow::{Context, Result};
use axum::{routing::get, Router};
use dashmap::DashMap;
use futures::StreamExt;
use http::Request;
use k8s_openapi::api::batch::v1::Job;
use kube::{
    api::{Api, DeleteParams, ListParams, PropagationPolicy},
    Client, ResourceExt,
};
use serde_json::json;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::time;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

/// Pool of Kubernetes cluster connections
use stormchaser_model::dsl;

pub struct ClusterPool {
    clients: DashMap<String, (Client, String)>, // (Client, Version)
}

impl Default for ClusterPool {
    fn default() -> Self {
        Self::new()
    }
}

impl ClusterPool {
    pub fn new() -> Self {
        Self {
            clients: DashMap::new(),
        }
    }

    /// Get a client and version for a named cluster, or the default if not found
    pub async fn get_client(&self, cluster_name: &str) -> Result<(Client, String)> {
        if let Some(entry) = self.clients.get(cluster_name) {
            return Ok(entry.clone());
        }

        let client = Client::try_default().await.context(format!(
            "Failed to create default K8s client for cluster {}",
            cluster_name
        ))?;

        // Query cluster version
        let version_resp = client
            .request_text(Request::builder().uri("/version").body(vec![])?)
            .await?;

        let version_data: Value = serde_json::from_str(&version_resp)?;
        let major = version_data["major"].as_str().unwrap_or("0");
        let minor = version_data["minor"].as_str().unwrap_or("0");
        let version = format!("{}.{}", major, minor.replace('+', ""));

        info!(
            "Connected to Kubernetes cluster {}: v{}",
            cluster_name, version
        );

        // Basic version check: we require at least v1.21.0 for batch/v1 Jobs
        let major_int: i32 = major.parse().unwrap_or(0);
        let minor_int: i32 = minor.replace('+', "").parse().unwrap_or(0);

        if major_int < 1 || (major_int == 1 && minor_int < 21) {
            let err_msg = format!("Kubernetes cluster {} version v{}.{} is not supported. Minimum required is v1.21.0", cluster_name, major, minor);
            error!("{}", err_msg);
            return Err(anyhow::anyhow!(err_msg));
        }

        self.clients
            .insert(cluster_name.to_string(), (client.clone(), version.clone()));
        Ok((client, version))
    }

    /// Add a pre-configured client to the pool
    pub fn add_client(&self, cluster_name: &str, client: Client, version: String) {
        self.clients
            .insert(cluster_name.to_string(), (client, version));
    }

    /// Get list of all cluster names in the pool
    pub fn cluster_names(&self) -> Vec<String> {
        self.clients.iter().map(|r| r.key().clone()).collect()
    }
}

struct AppState {
    is_ready: watch::Receiver<bool>,
    #[allow(dead_code)]
    cluster_pool: Arc<ClusterPool>,
}

async fn run_reaper(cluster_pool: Arc<ClusterPool>) -> Result<()> {
    let mut interval = time::interval(Duration::from_secs(3600)); // Check every hour
    loop {
        interval.tick().await;
        info!("Running garbage collection (reaper) on all clusters...");

        for cluster_name in cluster_pool.cluster_names() {
            if let Ok((client, _)) = cluster_pool.get_client(&cluster_name).await {
                let jobs: Api<Job> = Api::all(client.clone());
                let lp = ListParams::default().labels("managed-by=stormchaser");

                if let Ok(job_list) = jobs.list(&lp).await {
                    for job in job_list {
                        let now = chrono::Utc::now();
                        let creation_time = job.metadata.creation_timestamp.as_ref().map(|ts| ts.0);

                        if let Some(created) = creation_time {
                            let age = now - created;
                            // If job is older than 24 hours, clean it up
                            if age.num_hours() >= 24 {
                                let job_name = job.name_any();
                                let namespace =
                                    job.namespace().unwrap_or_else(|| "default".to_string());
                                info!(
                                    "Reaping old orphaned job {} in namespace {} (age: {}h)",
                                    job_name,
                                    namespace,
                                    age.num_hours()
                                );

                                let dp = DeleteParams {
                                    propagation_policy: Some(PropagationPolicy::Background),
                                    ..Default::default()
                                };
                                let _ = Api::<Job>::namespaced(client.clone(), &namespace)
                                    .delete(&job_name, &dp)
                                    .await;
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn scan_for_orphans(
    cluster_pool: Arc<ClusterPool>,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) -> Result<()> {
    info!("Scanning all clusters for orphaned jobs...");

    for cluster_name in cluster_pool.cluster_names() {
        let (client, cluster_version) = cluster_pool.get_client(&cluster_name).await?;
        let jobs: Api<Job> = Api::all(client.clone());

        let lp = ListParams::default().labels("managed-by=stormchaser");
        let job_list = jobs.list(&lp).await?;

        for job in job_list {
            let job_name = job.name_any();
            let namespace = job.namespace().unwrap_or_else(|| "default".to_string());
            let labels = job.metadata.labels.as_ref().context("Job missing labels")?;

            let run_id_str = labels.get("stormchaser-run-id");
            let step_id_str = labels.get("stormchaser-step-id");

            if let (Some(run_id_s), Some(step_id_s)) = (run_id_str, step_id_str) {
                let run_id = Uuid::parse_str(run_id_s).unwrap_or_default();
                let step_id = Uuid::parse_str(step_id_s).unwrap_or_default();

                info!(
                    "Found orphaned job {} for step {} (run {}) in cluster {}",
                    job_name, step_id, run_id, cluster_name
                );

                let annotations = job.metadata.annotations.as_ref();
                let received_at = annotations
                    .and_then(|a| a.get("stormchaser.io/received-at"))
                    .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(chrono::Utc::now);

                let is_encrypted = annotations
                    .and_then(|a| a.get("stormchaser.io/state-encrypted"))
                    .map(|v| v == "true")
                    .unwrap_or(false);

                let raw_step_dsl = annotations.and_then(|a| a.get("stormchaser.io/step-dsl"));

                let step_dsl: dsl::Step = if let Some(raw) = raw_step_dsl {
                    let dsl_str = if is_encrypted {
                        if let Some(key) = &encryption_key {
                            match job_machine::crypto::decrypt_state(raw, key) {
                                Ok(decrypted) => decrypted,

                                Err(e) => {
                                    error!("Failed to decrypt state for job {}: {:?}", job_name, e);
                                    continue;
                                }
                            }
                        } else {
                            error!(
                                "Job {} is encrypted but no encryption key is configured",
                                job_name
                            );
                            continue;
                        }
                    } else {
                        raw.clone()
                    };

                    serde_json::from_str(&dsl_str).unwrap_or_else(|_| {
                        // Fallback if parsing fails
                        dsl::Step {
                            name: job_name.clone(),
                            r#type: "RunContainer".to_string(),
                            spec: Value::Null,
                            params: HashMap::new(),
                            condition: None,
                            strategy: None,
                            aggregation: Vec::new(),
                            iterate: None,
                            iterate_as: None,
                            steps: None,
                            next: Vec::new(),
                            on_failure: None,
                            retry: None,
                            timeout: None,
                            allow_failure: None,
                            start_marker: None,
                            end_marker: None,
                            outputs: Vec::new(),
                            reports: Vec::new(),
                            artifacts: None,
                        }
                    })
                } else {
                    // Reconstruct minimal step metadata as fallback
                    dsl::Step {
                        name: job_name.clone(),
                        r#type: "RunContainer".to_string(),
                        spec: Value::Null,
                        params: HashMap::new(),
                        condition: None,
                        strategy: None,
                        aggregation: Vec::new(),
                        iterate: None,
                        iterate_as: None,
                        steps: None,
                        next: Vec::new(),
                        on_failure: None,
                        retry: None,
                        timeout: None,
                        allow_failure: None,
                        start_marker: None,
                        end_marker: None,
                        outputs: Vec::new(),
                        reports: Vec::new(),
                        artifacts: None,
                    }
                };

                let nats = nats_client.clone();
                let r_id = runner_id.clone();
                let client_clone = client.clone();
                let cv = cluster_version.clone();
                let key_clone = encryption_key.clone();

                tokio::spawn(async move {
                    // Before adopting, query the orchestrator to see if this step is still relevant
                    let query_payload = json!({
                        "step_id": step_id,
                    });

                    match nats
                        .request("stormchaser.step.query", query_payload.to_string().into())
                        .await
                    {
                        Ok(reply) => {
                            let response: Value =
                                serde_json::from_slice(&reply.payload).unwrap_or_default();
                            let status = response["status"].as_str().unwrap_or_default();
                            let exists = response["exists"].as_bool().unwrap_or(false);

                            if !exists || (status != "pending" && status != "running") {
                                info!("Step {} is in status {}, skipping adoption and cleaning up job {}", step_id, status, job_name);
                                let machine = job_machine::K8sJobMachine::new(
                                    client_clone,
                                    job_machine::JobMetadata {
                                        run_id,
                                        step_id,
                                        step_dsl,
                                        namespace,
                                        received_at,
                                        cluster_version: cv,
                                        encryption_key: key_clone,
                                        storage: None,
                                        test_report_urls: None,
                                    },
                                );
                                let _ = machine.clean_up(&job_name).await;
                                return;
                            }
                        }
                        Err(e) => {
                            error!("Failed to query orchestrator for step {}: {:?}", step_id, e);
                            // If we can't reach the orchestrator, we might want to wait or try again later.
                            // For now, let's assume it's safer NOT to adopt until we're sure.
                            return;
                        }
                    }

                    let metadata = job_machine::JobMetadata {
                        run_id,
                        step_id,
                        step_dsl: step_dsl.clone(),
                        namespace: namespace.clone(),
                        received_at,
                        cluster_version: cv.clone(),
                        encryption_key: key_clone,
                        storage: None,
                        test_report_urls: None,
                    };

                    let machine = job_machine::K8sJobMachine::new(client_clone, metadata);

                    match machine.adopt(job_name.clone()).wait().await {
                        Ok(job_state) => match job_state {
                            job_machine::JobState::Succeeded(metrics) => {
                                info!("Adopted step {} completed successfully", step_id);
                                let event = json!({
                                    "run_id": run_id,
                                    "step_id": step_id,
                                    "status": "succeeded",
                                    "runner_id": r_id,
                                    "exit_code": metrics.exit_code,
                                    "outputs": {
                                        "k8s exit code": metrics.exit_code,
                                        "Number of attempts": metrics.attempts,
                                        "run duration": format!("{}ms", metrics.duration_ms),
                                        "run latency": format!("{}ms", metrics.latency_ms),
                                    }
                                });
                                let _ = nats
                                    .publish("stormchaser.step.completed", event.to_string().into())
                                    .await;
                            }
                            job_machine::JobState::Failed(reason, metrics) => {
                                warn!("Adopted step {} failed: {}", step_id, reason);
                                let event = json!({
                                    "run_id": run_id,
                                    "step_id": step_id,
                                    "status": "failed",
                                    "error": reason,
                                    "runner_id": r_id,
                                    "exit_code": metrics.exit_code,
                                    "outputs": {
                                        "k8s exit code": metrics.exit_code,
                                        "Number of attempts": metrics.attempts,
                                        "run duration": format!("{}ms", metrics.duration_ms),
                                        "run latency": format!("{}ms", metrics.latency_ms),
                                    }
                                });
                                let _ = nats
                                    .publish("stormchaser.step.failed", event.to_string().into())
                                    .await;
                            }
                        },
                        Err(e) => error!("Error adopting job {}: {:?}", job_name, e),
                    }
                });
            }
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
pub struct Config {
    pub nats_url: String,
    pub runner_id: String,
    pub encryption_key: Option<String>,
    pub rust_log: String,
}

impl Config {
    pub fn from_env<I, K, V>(env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut nats_url = "nats://localhost:4222".to_string();
        let mut runner_id = Uuid::new_v4().to_string();
        let mut encryption_key = None;
        let mut rust_log = "stormchaser_runner_k8s=info".to_string();

        for (k, v) in env {
            match k.as_ref() {
                "NATS_URL" => nats_url = v.as_ref().to_string(),
                "RUNNER_ID" => runner_id = v.as_ref().to_string(),
                "STORMCHASER_STATE_ENCRYPTION_KEY" => encryption_key = Some(v.as_ref().to_string()),
                "RUST_LOG" => rust_log = v.as_ref().to_string(),
                _ => {}
            }
        }

        Self {
            nats_url,
            runner_id,
            encryption_key,
            rust_log,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env(std::env::vars());
    run_runner(config).await
}

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

    info!("Starting Stormchaser K8s Runner: {}", runner_id);

    // Initialize Kubernetes Cluster Pool
    let cluster_pool = Arc::new(ClusterPool::new());

    // Initialize default "local" client with versioning
    let local_client = Client::try_default()
        .await
        .context("Failed to initialize local Kubernetes client")?;

    let version_resp = local_client
        .request_text(Request::builder().uri("/version").body(vec![])?)
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
    let nats_subject = format!("stormchaser.runner.k8s.{}", runner_id);

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
        "capabilities": ["k8s", "linux", "container"],
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

    nats_client
        .publish(
            "stormchaser.runner.register",
            registration_payload.to_string().into(),
        )
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
                filter_subject: "stormchaser.step.scheduled.>".to_string(),
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
                let heartbeat_payload = json!({
                    "runner_id": heartbeat_id,
                });

                if let Err(e) = heartbeat_client
                    .publish("stormchaser.runner.heartbeat", heartbeat_payload.to_string().into())
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
    let _ = nats_client
        .publish(
            "stormchaser.runner.offline",
            deregistration_payload.to_string().into(),
        )
        .await;

    // Allow time for final messages to be sent
    time::sleep(Duration::from_secs(1)).await;

    health_server.abort();
    info!("Runner stopped");

    Ok(())
}

async fn handle_task(
    msg: async_nats::jetstream::message::Message,
    cluster_pool: Arc<ClusterPool>,
    nats_client: async_nats::Client,
    runner_id: String,
    encryption_key: Option<String>,
) {
    let received_at = chrono::Utc::now();
    tracing::info!("Received task message: {:?}", msg.subject);

    let payload: Value = serde_json::from_slice(&msg.payload).unwrap_or_default();
    let run_id_str = payload["run_id"].as_str().unwrap_or_default();
    let run_id = Uuid::parse_str(run_id_str).unwrap_or_default();
    let step_id_str = payload["step_id"].as_str().unwrap_or_default();
    let step_id = Uuid::parse_str(step_id_str).unwrap_or_default();

    let step_dsl: dsl::Step = match serde_json::from_value(payload["spec"].clone()) {
        Ok(spec) => {
            // Reconstruct Step from resolved spec and other fields
            dsl::Step {
                name: payload["step_name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                r#type: payload["step_type"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                spec,
                params: serde_json::from_value(payload["params"].clone()).unwrap_or_default(),
                condition: None,
                strategy: None,
                aggregation: Vec::new(),
                iterate: None,
                iterate_as: None,
                steps: None,
                next: Vec::new(),
                on_failure: None,
                retry: None,
                timeout: None,
                allow_failure: None,
                start_marker: None,
                end_marker: None,
                outputs: Vec::new(),
                reports: Vec::new(),
                artifacts: None,
            }
        }
        Err(e) => {
            tracing::error!("Failed to parse step spec: {:?}", e);
            return;
        }
    };
    let storage: Option<HashMap<String, Value>> =
        serde_json::from_value(payload["storage"].clone()).ok();
    let test_report_urls: Option<HashMap<String, Value>> =
        serde_json::from_value(payload["test_report_urls"].clone()).ok();

    let in_progress_msg = msg.clone();
    let in_progress_handle = tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(15)).await;
            let _ = in_progress_msg
                .ack_with(async_nats::jetstream::message::AckKind::Progress)
                .await;
        }
    });

    // Notify orchestrator that we are starting
    let running_event = serde_json::json!({
        "run_id": run_id,
        "step_id": step_id,
        "status": "running",
        "runner_id": runner_id,
        "timestamp": chrono::Utc::now(),
    });
    let _ = nats_client
        .publish("stormchaser.step.running", running_event.to_string().into())
        .await;

    let target_cluster = "local"; // In future, get from affinity/params
    match cluster_pool.get_client(target_cluster).await {
        Ok((client, cluster_version)) => {
            let namespace =
                std::env::var("KUBERNETES_NAMESPACE").unwrap_or_else(|_| "default".to_string());
            let metadata = job_machine::JobMetadata {
                run_id,
                step_id,
                step_dsl,
                namespace,
                received_at,
                cluster_version,
                encryption_key,
                storage,
                test_report_urls,
            };

            let machine = job_machine::K8sJobMachine::new(client.clone(), metadata.clone());

            let result = match machine.start().await {
                Ok(job_machine::StartResult::Running(running_machine)) => {
                    running_machine.wait().await
                }
                Ok(job_machine::StartResult::Failed(finished_machine)) => {
                    Ok(finished_machine.into_result())
                }
                Err(e) => Err(e),
            };
            in_progress_handle.abort();
            let _ = msg.double_ack().await;
            match result {
                Ok(job_machine::JobState::Succeeded(metrics)) => {
                    tracing::info!("Step {} (Run {}) completed successfully", step_id, run_id);
                    let complete_event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_id,
                        "status": "succeeded",
                        "runner_id": runner_id,
                        "exit_code": metrics.exit_code,
                        "storage_hashes": metrics.storage_hashes,
                        "artifacts": metrics.artifacts,
                        "test_reports": metrics.test_reports,
                        "outputs": {
                            "k8s exit code": metrics.exit_code,
                            "Number of attempts": metrics.attempts,
                            "run duration": format!("{}ms", metrics.duration_ms),
                            "run latency": format!("{}ms", metrics.latency_ms),
                        }
                    });
                    let _ = nats_client
                        .publish(
                            "stormchaser.step.completed",
                            complete_event.to_string().into(),
                        )
                        .await;
                }
                Ok(job_machine::JobState::Failed(reason, metrics)) => {
                    tracing::error!("Step {} (Run {}) failed: {}", step_id, run_id, reason);
                    let fail_event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_id,
                        "status": "failed",
                        "error": reason,
                        "runner_id": runner_id,
                        "exit_code": metrics.exit_code,
                        "storage_hashes": metrics.storage_hashes,
                        "artifacts": metrics.artifacts,
                        "test_reports": metrics.test_reports,
                        "outputs": {
                            "k8s exit code": metrics.exit_code,
                            "Number of attempts": metrics.attempts,
                            "run duration": format!("{}ms", metrics.duration_ms),
                            "run latency": format!("{}ms", metrics.latency_ms),
                        }
                    });
                    let _ = nats_client
                        .publish("stormchaser.step.failed", fail_event.to_string().into())
                        .await;
                }
                Err(e) => {
                    tracing::error!("Error running K8s job for step {}: {:?}", step_id, e);
                    let fail_event = serde_json::json!({
                        "run_id": run_id,
                        "step_id": step_id,
                        "status": "failed",
                        "error": format!("{:?}", e),
                        "runner_id": runner_id,
                    });
                    let _ = nats_client
                        .publish("stormchaser.step.failed", fail_event.to_string().into())
                        .await;
                }
            }
        }
        Err(e) => {
            tracing::error!("Failed to acquire K8s client: {:?}", e);
            let fail_event = serde_json::json!({
                "run_id": run_id,
                "step_id": step_id,
                "status": "failed",
                "error": format!("Failed to acquire K8s client: {:?}", e),
                "runner_id": runner_id,
            });
            let _ = nats_client
                .publish("stormchaser.step.failed", fail_event.to_string().into())
                .await;
        }
    }
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
        assert_eq!(config.rust_log, "stormchaser_runner_k8s=info");
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
