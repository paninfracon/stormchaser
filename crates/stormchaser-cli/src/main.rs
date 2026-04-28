use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "stormchaser")]
#[command(about = "Stormchaser CLI", long_about = None)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (rev: ", env!("VERGEN_GIT_SHA"), ", branch: ", env!("VERGEN_GIT_BRANCH"), ", built: ", env!("VERGEN_BUILD_TIMESTAMP"), ")"))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(
        short,
        long,
        env = "STORMCHASER_URL",
        default_value = "http://localhost:3000"
    )]
    pub url: String,

    #[arg(short, long, env = "STORMCHASER_TOKEN")]
    pub token: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a local workflow DSL file (direct execution)
    Run {
        /// Path to the .storm file
        file: PathBuf,

        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,

        /// Stream all logs from the workflow run until it completes
        #[arg(long, default_value_t = false)]
        tail: bool,

        /// Stream real-time state transition events for a run until it completes
        #[arg(long, default_value_t = false)]
        watch: bool,
    },

    /// Manage workflow runs
    Runs {
        #[command(subcommand)]
        command: RunCommands,
    },

    /// Manage webhooks
    Webhooks {
        #[command(subcommand)]
        command: WebhookCommands,
    },

    /// Manage event rules
    Rules {
        #[command(subcommand)]
        command: RuleCommands,
    },

    /// Manage storage backends
    Storage {
        #[command(subcommand)]
        command: StorageCommands,
    },

    /// Manage scheduled workflows (Cron)
    Cron {
        #[command(subcommand)]
        command: CronCommands,
    },

    /// Authentication commands
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },

    /// Interactive browser-based login
    Login {
        #[arg(long, default_value = "http://localhost:5556/dex")]
        issuer: String,
        #[arg(long, default_value = "stormchaser-cli")]
        client_id: String,
    },
}

#[derive(Subcommand)]
pub enum RunCommands {
    /// List workflow runs
    List {
        #[arg(long)]
        owner: Option<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        repo_url: Option<String>,
        #[arg(long)]
        workflow_path: Option<String>,
        #[arg(long)]
        created_after: Option<String>,
        #[arg(long)]
        created_before: Option<String>,
        #[arg(long)]
        status: Option<String>,
    },
    /// Get run details
    Get { id: Uuid },
    /// List artifacts for a run
    Artifacts { id: Uuid },
    /// List test reports for a run
    Reports { id: Uuid },
    /// Get a specific test report content
    Report {
        id: Uuid,
        #[arg(long)]
        report_id: Uuid,
    },
    /// Stream logs for a specific step in a run
    Logs {
        id: Uuid,
        #[arg(long)]
        step_name: String,
    },
    /// Stream real-time state transition events for a run
    Watch { id: Uuid },
    /// Enqueue a workflow from a git repository
    Enqueue {
        workflow_name: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        path: String,
        #[arg(long)]
        git_ref: String,
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
        /// Stream all logs from the workflow run until it completes
        #[arg(long, default_value_t = false)]
        tail: bool,
        /// Stream real-time state transition events for a run until it completes
        #[arg(long, default_value_t = false)]
        watch: bool,
    },
    /// List pending approvals/events
    Pending,
    /// Approve a waiting step
    Approve {
        run_id: Uuid,
        step_id: Uuid,
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
    },
    /// Reject a waiting step
    Reject { run_id: Uuid, step_id: Uuid },
    /// Approve or reject a step using an encrypted token link
    ApproveLink { token: String },
}

#[derive(Subcommand)]
pub enum WebhookCommands {
    /// List webhooks
    List,
    /// Create a new webhook
    Create {
        name: String,
        #[arg(long)]
        source_type: String,
        #[arg(long)]
        secret: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Get webhook details
    Get { id: Uuid },
    /// Delete a webhook
    Delete { id: Uuid },
}

#[derive(Subcommand)]
pub enum RuleCommands {
    /// List event rules
    List,
    /// Create a new event rule
    Create {
        name: String,
        #[arg(long)]
        webhook_id: Uuid,
        #[arg(long)]
        event_pattern: String,
        #[arg(long)]
        workflow: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "main")]
        git_ref: String,
        #[arg(long)]
        description: Option<String>,
        /// Input mappings in name=CEL_EXPR format
        #[arg(short, long)]
        mapping: Vec<String>,
    },
    /// Delete an event rule
    Delete { id: Uuid },
}

#[derive(Subcommand)]
pub enum StorageCommands {
    /// List storage backends
    List,
    /// Create a storage backend
    Create {
        name: String,
        /// The type of storage backend (e.g., s3, oci)
        #[arg(long)]
        backend_type: String,
        /// Path to JSON configuration file
        #[arg(long)]
        config: PathBuf,
        #[arg(long)]
        default_sfs: bool,
        #[arg(long)]
        description: Option<String>,
    },
    /// Get storage backend details
    Get { id: Uuid },
    /// Update a storage backend
    Update {
        id: Uuid,
        #[arg(long)]
        name: Option<String>,
        /// Path to JSON configuration file
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        default_sfs: Option<bool>,
        #[arg(long)]
        description: Option<String>,
    },
    /// Delete a storage backend
    Delete { id: Uuid },
}

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Exchange an SSO token for a Stormchaser JWT
    Exchange { sso_token: String },
}

#[derive(Subcommand)]
pub enum CronCommands {
    /// List scheduled cron workflows
    List,
    /// Schedule a new cron workflow
    Create {
        name: String,
        #[arg(long)]
        cron: String,
        #[arg(long)]
        workflow: String,
        #[arg(long)]
        repo: String,
        #[arg(long)]
        path: String,
        #[arg(long, default_value = "main")]
        git_ref: String,
        #[arg(long)]
        description: Option<String>,
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
    },
    /// Delete a scheduled cron workflow
    Delete { id: Uuid },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_cli(cli).await
}

pub async fn run_cli(cli: Cli) -> Result<()> {
    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    let http_client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    match cli.command {
        Commands::Run {
            ref file,
            ref input,
            tail,
            watch,
        } => {
            let dsl = fs::read_to_string(file)?;
            let inputs = parse_key_val_list(input.clone());
            let token = require_token(&cli)?;

            let res = http_client
                .post(format!("{}/api/v1/runs/direct", cli.url))
                .header("Authorization", format!("Bearer {}", token))
                .json(&json!({ "dsl": dsl, "inputs": inputs }))
                .send()
                .await?;

            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            if status.is_success() {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                    println!("{}", serde_json::to_string_pretty(&val)?);
                    if tail {
                        if let Some(id_str) = val.get("run_id").and_then(|i| i.as_str()) {
                            if let Ok(run_id) = Uuid::parse_str(id_str) {
                                println!("Streaming logs for run {}...", run_id);
                                stream_run_logs(&http_client, &cli.url, token, run_id).await?;
                            }
                        }
                    } else if watch {
                        if let Some(id_str) = val.get("run_id").and_then(|i| i.as_str()) {
                            if let Ok(run_id) = Uuid::parse_str(id_str) {
                                println!("Watching status for run {}...", run_id);
                                stream_run_status(&http_client, &cli.url, token, run_id).await?;
                            }
                        }
                    }
                } else {
                    println!("{}", body);
                }
            } else {
                eprintln!("Error ({}): {}", status, body);
            }
        }

        Commands::Runs { ref command } => match command {
            RunCommands::List {
                owner,
                name,
                repo_url,
                workflow_path,
                created_after,
                created_before,
                status,
            } => {
                let token = require_token(&cli)?;
                let mut url = reqwest::Url::parse(&format!("{}/api/v1/runs", cli.url))?;
                if let Some(o) = owner {
                    url.query_pairs_mut().append_pair("initiating_user", o);
                }
                if let Some(n) = name {
                    url.query_pairs_mut().append_pair("workflow_name", n);
                }
                if let Some(r) = repo_url {
                    url.query_pairs_mut().append_pair("repo_url", r);
                }
                if let Some(w) = workflow_path {
                    url.query_pairs_mut().append_pair("workflow_path", w);
                }
                if let Some(ca) = created_after {
                    url.query_pairs_mut().append_pair("created_after", ca);
                }
                if let Some(cb) = created_before {
                    url.query_pairs_mut().append_pair("created_before", cb);
                }
                if let Some(s) = status {
                    url.query_pairs_mut().append_pair("status", s);
                }

                let res = http_client
                    .get(url)
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Get { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/runs/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Artifacts { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/runs/{}/artifacts", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Reports { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/runs/{}/reports", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Report { id, report_id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!(
                        "{}/api/v1/runs/{}/reports/{}",
                        cli.url, id, report_id
                    ))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Logs { id, step_name } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!(
                        "{}/api/v1/runs/{}/steps/{}/logs/stream",
                        cli.url,
                        id,
                        urlencoding::encode(step_name)
                    ))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;

                if !res.status().is_success() {
                    handle_response(res).await?;
                    return Ok(());
                }

                use eventsource_stream::Eventsource;
                use futures::stream::StreamExt;
                let mut stream = res.bytes_stream().eventsource();
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(event) => {
                            if event.event == "error" {
                                eprintln!("Error from stream: {}", event.data);
                                break;
                            }
                            println!("{}", event.data);
                        }
                        Err(e) => {
                            eprintln!("Stream error: {}", e);
                            break;
                        }
                    }
                }
            }
            RunCommands::Watch { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/runs/{}/status/stream", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;

                if !res.status().is_success() {
                    handle_response(res).await?;
                    return Ok(());
                }

                use eventsource_stream::Eventsource;
                use futures::stream::StreamExt;
                let mut stream = res.bytes_stream().eventsource();
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(event) => {
                            if event.event == "error" {
                                eprintln!("Error from stream: {}", event.data);
                                break;
                            }
                            println!("{}: {}", event.event, event.data);
                        }
                        Err(e) => {
                            eprintln!("Stream error: {}", e);
                            break;
                        }
                    }
                }
            }
            RunCommands::Enqueue {
                workflow_name,
                repo,
                path,
                git_ref,
                input,
                tail,
                watch,
            } => {
                let inputs = parse_key_val_list(input.clone());
                let token = require_token(&cli)?;
                let res = http_client
                    .post(format!("{}/api/v1/runs", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&json!({
                        "workflow_name": workflow_name,
                        "repo_url": repo,
                        "workflow_path": path,
                        "git_ref": git_ref,
                        "inputs": inputs,
                    }))
                    .send()
                    .await?;

                let status = res.status();
                let body = res.text().await.unwrap_or_default();
                if status.is_success() {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                        println!("{}", serde_json::to_string_pretty(&val)?);
                        if *tail {
                            if let Some(id_str) = val.get("run_id").and_then(|i| i.as_str()) {
                                if let Ok(run_id) = Uuid::parse_str(id_str) {
                                    println!("Streaming logs for run {}...", run_id);
                                    stream_run_logs(&http_client, &cli.url, token, run_id).await?;
                                }
                            }
                        } else if *watch {
                            if let Some(id_str) = val.get("run_id").and_then(|i| i.as_str()) {
                                if let Ok(run_id) = Uuid::parse_str(id_str) {
                                    println!("Watching status for run {}...", run_id);
                                    stream_run_status(&http_client, &cli.url, token, run_id)
                                        .await?;
                                }
                            }
                        }
                    } else {
                        println!("{}", body);
                    }
                } else {
                    eprintln!("Error ({}): {}", status, body);
                }
            }
            RunCommands::Approve {
                run_id,
                step_id,
                input,
            } => {
                let token = require_token(&cli)?;
                let inputs = parse_key_val_list(input.clone());
                let res = http_client
                    .post(format!(
                        "{}/api/v1/runs/{}/steps/{}/approve",
                        cli.url, run_id, step_id
                    ))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&inputs)
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Reject { run_id, step_id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .post(format!(
                        "{}/api/v1/runs/{}/steps/{}/reject",
                        cli.url, run_id, step_id
                    ))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::ApproveLink { token } => {
                let res = http_client
                    .get(format!("{}/api/v1/approve-link/{}", cli.url, token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RunCommands::Pending => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/runs?status=Running", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
        },

        Commands::Webhooks { ref command } => match command {
            WebhookCommands::List => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/webhooks", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            WebhookCommands::Create {
                name,
                source_type,
                secret,
                description,
            } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .post(format!("{}/api/v1/webhooks", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&json!({
                        "name": name,
                        "source_type": source_type,
                        "secret_token": secret,
                        "description": description,
                    }))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            WebhookCommands::Get { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/webhooks/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            WebhookCommands::Delete { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .delete(format!("{}/api/v1/webhooks/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
        },

        Commands::Rules { ref command } => match command {
            RuleCommands::List => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/rules", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RuleCommands::Create {
                name,
                webhook_id,
                event_pattern,
                workflow,
                repo,
                path,
                git_ref,
                description,
                mapping,
            } => {
                let mappings = parse_key_val_list(mapping.clone());
                let token = require_token(&cli)?;
                let res = http_client
                    .post(format!("{}/api/v1/rules", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&json!({
                        "name": name,
                        "webhook_id": webhook_id,
                        "event_type_pattern": event_pattern,
                        "workflow_name": workflow,
                        "repo_url": repo,
                        "workflow_path": path,
                        "git_ref": git_ref,
                        "description": description,
                        "input_mappings": mappings,
                    }))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            RuleCommands::Delete { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .delete(format!("{}/api/v1/rules/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
        },

        Commands::Storage { ref command } => match command {
            StorageCommands::List => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/storage-backends", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            StorageCommands::Create {
                name,
                backend_type,
                config,
                default_sfs,
                description,
            } => {
                let config_json: serde_json::Value =
                    serde_json::from_str(&fs::read_to_string(config)?)?;
                let token = require_token(&cli)?;
                let res = http_client
                    .post(format!("{}/api/v1/storage-backends", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&json!({
                        "name": name,
                        "backend_type": backend_type,
                        "config": config_json,
                        "is_default_sfs": default_sfs,
                        "description": description,
                    }))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            StorageCommands::Get { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/storage-backends/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            StorageCommands::Update {
                id,
                name,
                config,
                default_sfs,
                description,
            } => {
                let mut body = json!({});
                if let Some(n) = name {
                    body["name"] = json!(n);
                }
                if let Some(c) = config {
                    body["config"] = serde_json::from_str(&fs::read_to_string(c)?)?;
                }
                if let Some(d) = default_sfs {
                    body["is_default_sfs"] = json!(d);
                }
                if let Some(desc) = description {
                    body["description"] = json!(desc);
                }

                let token = require_token(&cli)?;
                let res = http_client
                    .patch(format!("{}/api/v1/storage-backends/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&body)
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            StorageCommands::Delete { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .delete(format!("{}/api/v1/storage-backends/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
        },

        Commands::Cron { ref command } => match command {
            CronCommands::List => {
                let token = require_token(&cli)?;
                let res = http_client
                    .get(format!("{}/api/v1/cron-workflows", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            CronCommands::Create {
                name,
                cron,
                workflow,
                repo,
                path,
                git_ref,
                description,
                input,
            } => {
                let inputs = parse_key_val_list(input.clone());
                let token = require_token(&cli)?;
                let res = http_client
                    .post(format!("{}/api/v1/cron-workflows", cli.url))
                    .header("Authorization", format!("Bearer {}", token))
                    .json(&json!({
                        "name": name,
                        "cronspec": cron,
                        "workflow_name": workflow,
                        "repo_url": repo,
                        "workflow_path": path,
                        "git_ref": git_ref,
                        "inputs": inputs,
                        "description": description,
                    }))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
            CronCommands::Delete { id } => {
                let token = require_token(&cli)?;
                let res = http_client
                    .delete(format!("{}/api/v1/cron-workflows/{}", cli.url, id))
                    .header("Authorization", format!("Bearer {}", token))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
        },

        Commands::Auth { ref command } => match command {
            AuthCommands::Exchange { ref sso_token } => {
                let res = http_client
                    .post(format!("{}/api/v1/auth/exchange", cli.url))
                    .json(&json!({ "sso_token": sso_token }))
                    .send()
                    .await?;
                handle_response(res).await?;
            }
        },

        Commands::Login { issuer, client_id } => {
            handle_login(&cli.url, &issuer, &client_id, &http_client).await?;
        }
    }

    Ok(())
}

async fn handle_login(
    cli_url: &str,
    issuer: &str,
    client_id: &str,
    http_client: &reqwest_middleware::ClientWithMiddleware,
) -> Result<()> {
    let redirect_uri = "http://localhost:8080/callback";
    let auth_url = format!(
        "{}/auth?client_id={}&redirect_uri={}&response_type=code&scope=openid+profile+email",
        issuer.trim_end_matches('/'),
        client_id,
        redirect_uri
    );

    println!("Opening browser for authentication...");
    println!("If the browser does not open automatically, please visit:");
    println!("{}", auth_url);

    if let Err(e) = open::that(&auth_url) {
        eprintln!("Failed to open browser: {}", e);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
        .await
        .context(
            "Failed to bind callback server on port 8080. Is another login process running?",
        )?;

    let (mut stream, _) = listener
        .accept()
        .await
        .context("Failed to accept callback connection")?;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut buf = [0; 4096];
    let mut request_str = String::new();
    if let Ok(n) = stream.read(&mut buf).await {
        request_str = String::from_utf8_lossy(&buf[0..n]).to_string();
    }

    let mut code = None;
    for line in request_str.lines() {
        if line.starts_with("GET /callback") {
            if let Some(query) = line
                .split_whitespace()
                .nth(1)
                .and_then(|p| p.split('?').nth(1))
            {
                for pair in query.split('&') {
                    if let Some((k, v)) = pair.split_once('=') {
                        if k == "code" {
                            code = Some(v.to_string());
                        }
                    }
                }
            }
            break;
        }
    }

    let response =
        "HTTP/1.1 200 OK\r\nContent-Length: 44\r\n\r\nLogin successful. You can close this window.";
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;

    let code = match code {
        Some(c) => c,
        None => {
            anyhow::bail!("Failed to parse authorization code from callback. Response may have been an error.");
        }
    };

    println!("Exchanging authorization code for token...");

    // The client uses the standard reqwest client, not the wrapped one, for this specific call because it's external
    let client = reqwest::Client::new();
    let token_res = client
        .post(format!("{}/token", issuer.trim_end_matches('/')))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("client_secret", "stormchaser-cli-secret"),
            ("redirect_uri", redirect_uri),
            ("code", &code),
        ])
        .send()
        .await?;

    let sso_token = if token_res.status().is_success() {
        let json: serde_json::Value = token_res.json().await.unwrap_or_default();
        if let Some(id_token) = json.get("id_token").and_then(|v| v.as_str()) {
            id_token.to_string()
        } else {
            anyhow::bail!("Dex response missing id_token");
        }
    } else {
        anyhow::bail!("Dex token exchange failed: {}", token_res.status());
    };

    println!("Exchanging SSO token for Stormchaser JWT...");
    let res = http_client
        .post(format!("{}/api/v1/auth/exchange", cli_url))
        .json(&json!({ "sso_token": sso_token }))
        .send()
        .await?;

    // To mirror typical CLI login experience, let's parse the response and print the token directly
    // so the user can easily export it
    let status = res.status();
    let body = res.text().await.unwrap_or_default();
    if status.is_success() {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
            if let Some(access_token) = val.get("access_token").and_then(|t| t.as_str()) {
                println!("\nSuccessfully logged in! Export your token to use it:");
                println!("export STORMCHASER_TOKEN=\"{}\"", access_token);
            } else {
                println!("Success, but could not parse access_token from response.");
                println!("{}", serde_json::to_string_pretty(&val).unwrap_or(body));
            }
        } else {
            println!("Success:\n{}", body);
        }
    } else {
        eprintln!("Error exchanging SSO token ({}): {}", status, body);
    }

    Ok(())
}

fn parse_key_val_list(list: Vec<String>) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for i in list {
        if let Some((key, value)) = i.split_once('=') {
            // Try to parse as JSON first (for numbers, bools), fallback to string
            let val = serde_json::from_str(value).unwrap_or_else(|_| json!(value));
            map.insert(key.to_string(), val);
        }
    }
    map
}

fn require_token(cli: &Cli) -> Result<&str> {
    cli.token
        .as_deref()
        .context("Authentication token required (use --token or STORMCHASER_TOKEN env var)")
}

async fn stream_run_logs(
    http_client: &reqwest_middleware::ClientWithMiddleware,
    cli_url: &str,
    token: &str,
    run_id: Uuid,
) -> Result<()> {
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/logs/stream", cli_url, run_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        handle_response(res).await?;
        return Ok(());
    }

    use eventsource_stream::Eventsource;
    use futures::stream::StreamExt;
    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}", event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

async fn handle_response(res: reqwest::Response) -> Result<()> {
    let status = res.status();
    if status.is_success() {
        let body = res.text().await?;
        if !body.is_empty() {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&body) {
                println!("{}", serde_json::to_string_pretty(&val)?);
            } else {
                println!("{}", body);
            }
        } else {
            println!("Success ({})", status);
        }
    } else {
        let error_text = res.text().await.unwrap_or_default();
        eprintln!("Error ({}): {}", status, error_text);
        std::process::exit(1);
    }
    Ok(())
}

async fn stream_run_status(
    http_client: &reqwest_middleware::ClientWithMiddleware,
    cli_url: &str,
    token: &str,
    run_id: Uuid,
) -> Result<()> {
    let res = http_client
        .get(format!("{}/api/v1/runs/{}/status/stream", cli_url, run_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await?;

    if !res.status().is_success() {
        handle_response(res).await?;
        return Ok(());
    }

    use eventsource_stream::Eventsource;
    use futures::stream::StreamExt;
    let mut stream = res.bytes_stream().eventsource();
    while let Some(event) = stream.next().await {
        match event {
            Ok(event) => {
                if event.event == "error" {
                    eprintln!("Error from stream: {}", event.data);
                    break;
                }
                println!("{}: {}", event.event, event.data);
            }
            Err(e) => {
                eprintln!("Stream error: {}", e);
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_key_val_list_strings() {
        let input = vec!["key1=value1".to_string(), "key2=value2".to_string()];
        let map = parse_key_val_list(input);
        assert_eq!(map.get("key1").unwrap().as_str().unwrap(), "value1");
        assert_eq!(map.get("key2").unwrap().as_str().unwrap(), "value2");
    }

    #[test]
    fn test_parse_key_val_list_json_types() {
        let input = vec!["num=42".to_string(), "bool=true".to_string()];
        let map = parse_key_val_list(input);
        assert_eq!(map.get("num").unwrap().as_i64().unwrap(), 42);
        assert!(map.get("bool").unwrap().as_bool().unwrap());
    }

    #[test]
    fn test_require_token_missing() {
        let cli = Cli {
            command: Commands::Webhooks {
                command: WebhookCommands::List,
            },
            url: "http://localhost:3000".to_string(),
            token: None,
        };
        let err = require_token(&cli).unwrap_err();
        assert!(err.to_string().contains("Authentication token required"));
    }

    #[test]
    fn test_require_token_present() {
        let cli = Cli {
            command: Commands::Webhooks {
                command: WebhookCommands::List,
            },
            url: "http://localhost:3000".to_string(),
            token: Some("my-token".to_string()),
        };
        let token = require_token(&cli).unwrap();
        assert_eq!(token, "my-token");
    }
}
