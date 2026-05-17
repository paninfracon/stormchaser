use anyhow::Result;
use clap::Subcommand;

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
    Get { id: stormchaser_model::RunId },
    /// List artifacts for a run
    Artifacts { id: stormchaser_model::RunId },
    /// List test reports for a run
    Reports { id: stormchaser_model::RunId },
    /// Get a specific test report content
    Report {
        id: stormchaser_model::RunId,
        #[arg(long)]
        report_id: stormchaser_model::TestReportId,
    },
    /// Stream logs for a specific step in a run
    Logs {
        id: stormchaser_model::RunId,
        #[arg(long)]
        step_name: String,
    },
    /// Stream real-time state transition events for a run
    Watch { id: stormchaser_model::RunId },
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
        run_id: stormchaser_model::RunId,
        step_id: stormchaser_model::StepInstanceId,
        /// Input parameters in key=value format
        #[arg(short, long)]
        input: Vec<String>,
    },
    /// Reject a waiting step
    Reject {
        run_id: stormchaser_model::RunId,
        step_id: stormchaser_model::StepInstanceId,
    },
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

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::StepInstanceId;

    fn build_dummy_client() -> reqwest_middleware::ClientWithMiddleware {
        reqwest_middleware::ClientBuilder::new(reqwest::Client::new()).build()
    }

    #[tokio::test]
    async fn test_handle_commands() {
        let client = build_dummy_client();
        let url = "http://127.0.0.1:1"; // Invalid dummy URL to force connection refused
        let token = Some("test_token");
        let id = stormchaser_model::RunId::new_v4();

        let commands = vec![
            RunCommands::List {
                owner: None,
                name: None,
                repo_url: None,
                workflow_path: None,
                created_after: None,
                created_before: None,
                status: None,
            },
            RunCommands::Get { id },
            RunCommands::Artifacts { id },
            RunCommands::Reports { id },
            RunCommands::Report {
                id,
                report_id: stormchaser_model::TestReportId::new_v4(),
            },
            RunCommands::Logs {
                id,
                step_name: "test".to_string(),
            },
            RunCommands::Watch { id },
            RunCommands::Enqueue {
                workflow_name: "test".to_string(),
                repo: "test".to_string(),
                path: "test".to_string(),
                git_ref: "test".to_string(),
                input: vec![],
                tail: false,
                watch: false,
            },
            RunCommands::Approve {
                run_id: id,
                step_id: StepInstanceId::new_v4(),
                input: vec![],
            },
            RunCommands::Reject {
                run_id: id,
                step_id: StepInstanceId::new_v4(),
            },
            RunCommands::ApproveLink {
                token: "dummy".to_string(),
            },
            RunCommands::Pending,
        ];

        for cmd in commands {
            let res = handle(url, token, &client, cmd).await;
            assert!(res.is_err(), "Expected an error but got Ok for command");
            let err_msg = res.unwrap_err().to_string();
            assert!(
                err_msg.contains("Connection refused")
                    || err_msg.contains("error sending request")
                    || err_msg.contains("builder error")
                    || err_msg.contains("ConnectError"),
                "Unexpected error: {}",
                err_msg
            );
        }
    }
}
