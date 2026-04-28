mod commands;
mod utils;

use anyhow::Result;
use clap::{Parser, Subcommand};
use reqwest_middleware::ClientBuilder;
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use std::path::PathBuf;

use crate::commands::{auth, cron, rules, run, runs, storage, webhooks};

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
        command: runs::RunCommands,
    },

    /// Manage webhooks
    Webhooks {
        #[command(subcommand)]
        command: webhooks::WebhookCommands,
    },

    /// Manage event rules
    Rules {
        #[command(subcommand)]
        command: rules::RuleCommands,
    },

    /// Manage storage backends
    Storage {
        #[command(subcommand)]
        command: storage::StorageCommands,
    },

    /// Manage scheduled workflows (Cron)
    Cron {
        #[command(subcommand)]
        command: cron::CronCommands,
    },

    /// Authentication commands
    Auth {
        #[command(subcommand)]
        command: auth::AuthCommands,
    },

    /// Interactive browser-based login
    Login {
        #[arg(long, default_value = "http://localhost:5556/dex")]
        issuer: String,
        #[arg(long, default_value = "stormchaser-cli")]
        client_id: String,
    },
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

    let token_opt = cli.token.as_deref();

    match cli.command {
        Commands::Run {
            file,
            input,
            tail,
            watch,
        } => {
            run::handle(&cli.url, token_opt, &http_client, file, input, tail, watch).await?;
        }

        Commands::Runs { command } => {
            runs::handle(&cli.url, token_opt, &http_client, command).await?;
        }

        Commands::Webhooks { command } => {
            webhooks::handle(&cli.url, token_opt, &http_client, command).await?;
        }

        Commands::Rules { command } => {
            rules::handle(&cli.url, token_opt, &http_client, command).await?;
        }

        Commands::Storage { command } => {
            storage::handle(&cli.url, token_opt, &http_client, command).await?;
        }

        Commands::Cron { command } => {
            cron::handle(&cli.url, token_opt, &http_client, command).await?;
        }

        Commands::Auth { command } => {
            auth::handle(&cli.url, &http_client, command).await?;
        }

        Commands::Login { issuer, client_id } => {
            auth::handle_login(&cli.url, &issuer, &client_id, &http_client).await?;
        }
    }

    Ok(())
}
