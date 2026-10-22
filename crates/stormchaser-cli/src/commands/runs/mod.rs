use anyhow::Result;
use clap::Subcommand;
use stormchaser_client::Client;

pub mod approve;
pub mod artifacts;
pub mod delete;
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
    /// Delete a run
    Delete { id: stormchaser_model::RunId },
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
        /// The step's name, or the id of one of its instances. A step that ran more
        /// than once (in a loop, say) is followed in its most recent instance.
        #[arg(long, alias = "step")]
        step_name: String,
    },
    /// Stream real-time state transition events for a run
    Watch { id: stormchaser_model::RunId },
    /// Enqueue a workflow from a git repository
    Enqueue {
        workflow_name: String,
        #[arg(long)]
        connection: String,
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

pub async fn handle(client: &Client, command: RunCommands) -> Result<()> {
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
                client,
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
        RunCommands::Get { id } => get::get_run(client, id).await,
        RunCommands::Delete { id } => delete::delete_run(client, id).await,
        RunCommands::Artifacts { id } => artifacts::list_artifacts(client, id).await,
        RunCommands::Reports { id } => reports::list_reports(client, id).await,
        RunCommands::Report { id, report_id } => reports::get_report(client, id, report_id).await,
        RunCommands::Logs { id, step_name } => logs::stream_logs(client, id, step_name).await,
        RunCommands::Watch { id } => watch::watch_run(client, id).await,
        RunCommands::Enqueue {
            workflow_name,
            connection,
            path,
            git_ref,
            input,
            tail,
            watch,
        } => {
            enqueue::enqueue_run(
                client,
                enqueue::EnqueueRunParams {
                    workflow_name,
                    connection,
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
        } => approve::approve_step(client, run_id, step_id, input).await,
        RunCommands::Reject { run_id, step_id } => {
            approve::reject_step(client, run_id, step_id).await
        }
        RunCommands::ApproveLink { token: link_token } => {
            approve::approve_link(client, link_token).await
        }
        RunCommands::Pending => approve::list_pending(client).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stormchaser_model::StepInstanceId;

    fn build_dummy_client() -> Client {
        // An address nothing listens on, to force connection refused
        Client::builder("http://127.0.0.1:1")
            .token("test_token")
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn test_handle_commands() {
        let client = build_dummy_client();
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
            RunCommands::Delete { id },
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
                connection: "test".to_string(),
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
            let res = handle(&client, cmd).await;
            let err = res.expect_err("Expected an error but got Ok for command");
            let err_msg = format!("{err:#}");
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
