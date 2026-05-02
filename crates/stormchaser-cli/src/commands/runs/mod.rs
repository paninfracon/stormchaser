use anyhow::Result;
use clap::Subcommand;
use uuid::Uuid;

pub mod approve;
pub mod artifacts;
pub mod enqueue;
pub mod get;
pub mod list;
pub mod logs;
pub mod reports;
pub mod watch;

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

pub async fn handle(
    url: &str,
    token: Option<&str>,
    http_client: &reqwest_middleware::ClientWithMiddleware,
    command: RunCommands,
) -> Result<()> {
    match command {
        RunCommands::List {
            owner,
            name,
            repo_url,
            workflow_path,
            created_after,
            created_before,
            status,
        } => {
            list::list_runs(
                url,
                token,
                http_client,
                list::ListRunsFilters {
                    owner,
                    name,
                    repo_url,
                    workflow_path,
                    created_after,
                    created_before,
                    status,
                },
            )
            .await
        }
        RunCommands::Get { id } => get::get_run(url, token, http_client, id).await,
        RunCommands::Artifacts { id } => {
            artifacts::list_artifacts(url, token, http_client, id).await
        }
        RunCommands::Reports { id } => reports::list_reports(url, token, http_client, id).await,
        RunCommands::Report { id, report_id } => {
            reports::get_report(url, token, http_client, id, report_id).await
        }
        RunCommands::Logs { id, step_name } => {
            logs::stream_logs(url, token, http_client, id, step_name).await
        }
        RunCommands::Watch { id } => watch::watch_run(url, token, http_client, id).await,
        RunCommands::Enqueue {
            workflow_name,
            repo,
            path,
            git_ref,
            input,
            tail,
            watch,
        } => {
            enqueue::enqueue_run(
                url,
                token,
                http_client,
                enqueue::EnqueueRunParams {
                    workflow_name,
                    repo,
                    path,
                    git_ref,
                    input,
                    tail,
                    watch,
                },
            )
            .await
        }
        RunCommands::Approve {
            run_id,
            step_id,
            input,
        } => approve::approve_step(url, token, http_client, run_id, step_id, input).await,
        RunCommands::Reject { run_id, step_id } => {
            approve::reject_step(url, token, http_client, run_id, step_id).await
        }
        RunCommands::ApproveLink { token: link_token } => {
            approve::approve_link(url, http_client, link_token).await
        }
        RunCommands::Pending => approve::list_pending(url, token, http_client).await,
    }
}
