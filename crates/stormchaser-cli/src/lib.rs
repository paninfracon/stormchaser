//! Main entry point for the Stormchaser CLI.
//!
//! Provides commands for interacting with the Stormchaser API and running local workflows.

pub mod commands;
pub mod utils;

use anyhow::Result;
use clap::{Parser, Subcommand};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use std::path::PathBuf;

use crate::commands::{auth, connections, cron, lint, rules, run, runs, schema, webhooks};

/// Main CLI arguments and configuration.
#[derive(Parser)]
#[command(name = "stormchaser")]
#[command(about = "Stormchaser CLI", long_about = None)]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (rev: ", env!("VERGEN_GIT_SHA"), ", branch: ", env!("VERGEN_GIT_BRANCH"), ", built: ", env!("VERGEN_BUILD_TIMESTAMP"), ")"))]
pub struct Cli {
    /// The specific subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,

    /// The base URL of the Stormchaser API.
    #[arg(short, long, env = "STORMCHASER_URL")]
    pub url: Option<String>,

    /// The authentication token for the API.
    #[arg(short, long, env = "STORMCHASER_TOKEN")]
    pub token: Option<String>,
}

/// Available CLI subcommands.
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
        /// Subcommands for runs
        #[command(subcommand)]
        command: runs::RunCommands,
    },

    /// Manage webhooks
    Webhooks {
        /// Subcommands for webhooks
        #[command(subcommand)]
        command: webhooks::WebhookCommands,
    },

    /// Manage event rules
    Rules {
        /// Subcommands for event rules
        #[command(subcommand)]
        command: rules::RuleCommands,
    },

    /// Manage connections
    Connections {
        /// Subcommands for connections
        #[command(subcommand)]
        command: connections::ConnectionCommands,
    },

    /// Manage scheduled workflows (Cron)
    Cron {
        /// Subcommands for cron workflows
        #[command(subcommand)]
        command: cron::CronCommands,
    },

    /// Lint a workflow file against the schema
    Lint(lint::LintCommand),

    /// Manage schemas
    Schema {
        /// Subcommands for schemas
        #[command(subcommand)]
        command: schema::SchemaCommands,
    },

    /// Authentication commands
    Auth {
        /// Subcommands for authentication
        #[command(subcommand)]
        command: auth::AuthCommands,
    },

    /// Interactive browser-based login
    Login {
        /// The OIDC issuer URL.
        #[arg(long, default_value = "http://localhost:5556/dex")]
        issuer: String,
        /// The OIDC client ID.
        #[arg(long, default_value = "stormchaser-cli")]
        client_id: String,
    },
}

/// Executes the specified CLI command.
///
/// This function sets up the HTTP client and dispatches the command logic
/// to the appropriate handler based on the parsed CLI options.
pub async fn run_cli(cli: Cli) -> Result<()> {
    let retry_policy = ExponentialBackoff::builder().build_with_max_retries(3);
    let http_client = ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build();

    let mut final_url = cli.url.clone();
    let mut final_token = cli.token.clone();

    if let Ok(Some(profile)) = commands::auth_profiles::get_active_profile() {
        if final_url.is_none() {
            final_url = Some(profile.url);
        }
        if final_token.is_none() {
            final_token = profile.token;
        }
    }

    let resolved_url = final_url.unwrap_or_else(|| "http://localhost:3000".to_string());
    let token_opt = final_token.as_deref();

    match cli.command {
        Commands::Run {
            file,
            input,
            tail,
            watch,
        } => {
            run::handle(
                &resolved_url,
                token_opt,
                &http_client,
                file,
                input,
                tail,
                watch,
            )
            .await?;
        }

        Commands::Runs { command } => {
            runs::handle(&resolved_url, token_opt, &http_client, command).await?;
        }

        Commands::Webhooks { command } => {
            webhooks::handle(&resolved_url, token_opt, &http_client, command).await?;
        }

        Commands::Rules { command } => {
            rules::handle(&resolved_url, token_opt, &http_client, command).await?;
        }

        Commands::Connections { command } => {
            connections::handle(&resolved_url, token_opt, &http_client, command).await?;
        }

        Commands::Cron { command } => {
            cron::handle(&resolved_url, token_opt, &http_client, command).await?;
        }

        Commands::Lint(command) => {
            lint::handle(&resolved_url, &http_client, command).await?;
        }

        Commands::Schema { command } => {
            schema::handle(command)?;
        }

        Commands::Auth { command } => {
            auth::handle(&resolved_url, &http_client, command).await?;
        }

        Commands::Login { issuer, client_id } => {
            auth::handle_login(&resolved_url, &issuer, &client_id, &http_client).await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parse_cli_basic() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["stormchaser", "--url", "http://test", "webhooks", "list"])
            .unwrap();
        assert_eq!(cli.url, Some("http://test".to_string()));
        assert!(matches!(cli.command, Commands::Webhooks { .. }));
    }
}
