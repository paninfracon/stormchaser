//! Main entry point for the Stormchaser CLI.
//!
//! Provides commands for interacting with the Stormchaser API and running local workflows.

pub mod commands;
pub mod utils;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use stormchaser_client::Client;

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

    /// The authentication token for the API. Prefer the STORMCHASER_TOKEN environment
    /// variable or a profile: other users on the host can read a command's arguments.
    #[arg(short, long, env = "STORMCHASER_TOKEN", hide_env_values = true)]
    pub token: Option<String>,

    /// A PEM file of CA certificates to trust as well as the system's, for an API
    /// whose certificate a private CA signed.
    #[arg(long, env = "STORMCHASER_CA_CERT")]
    pub ca_cert: Option<PathBuf>,
}

/// Available CLI subcommands.
#[derive(Subcommand)]
pub enum Commands {
    /// Run a local workflow DSL file (direct execution)
    Run {
        /// Path to the .storm file
        file: PathBuf,

        /// Input parameters in key=value format, or key=@path to read the value from a
        /// file (for secrets, which would otherwise show in the process list)
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
        /// Loopback port the browser is sent back to. It must be one of the redirect
        /// URLs registered with the identity provider.
        #[arg(long, default_value_t = 8080)]
        callback_port: u16,
        /// Print the token instead of storing it in a profile.
        #[arg(long)]
        print_token: bool,
        /// No longer used: the API knows its own identity provider.
        #[arg(long, hide = true)]
        issuer: Option<String>,
        /// No longer used: the API knows its own client.
        #[arg(long, hide = true)]
        client_id: Option<String>,
    },
}

/// The API client for `url`, carrying `token` and trusting the CA certificates in
/// `ca_cert` as well as the system's. Transient failures are retried three times.
pub fn api_client(url: &str, token: Option<&str>, ca_cert: Option<&Path>) -> Result<Client> {
    let mut builder = Client::builder(url).retries(3);
    if let Some(token) = token {
        builder = builder.token(token);
    }
    if let Some(path) = ca_cert {
        let pem = std::fs::read(path)
            .with_context(|| format!("cannot read the CA certificate {}", path.display()))?;
        builder = builder.ca_pem(pem);
    }
    builder.build().with_context(|| match ca_cert {
        Some(path) => format!("cannot use the CA certificate {}", path.display()),
        None => "cannot build the HTTP client".to_string(),
    })
}

/// Whether `command` calls the API as a signed-in user.
fn needs_token(command: &Commands) -> bool {
    match command {
        Commands::Runs { command } => !matches!(command, runs::RunCommands::ApproveLink { .. }),
        Commands::Run { .. }
        | Commands::Webhooks { .. }
        | Commands::Rules { .. }
        | Commands::Connections { .. }
        | Commands::Cron { .. } => true,
        Commands::Lint(_)
        | Commands::Schema { .. }
        | Commands::Auth { .. }
        | Commands::Login { .. } => false,
    }
}

/// Executes the specified CLI command.
///
/// This function sets up the API client and dispatches the command logic
/// to the appropriate handler based on the parsed CLI options.
pub async fn run_cli(cli: Cli) -> Result<()> {
    let mut final_url = cli.url.clone();
    let mut final_token = cli.token.clone();

    if let Ok(Some(profile)) = commands::auth_profiles::get_active_profile() {
        // A profile's token belongs to that profile's server: never send it to a URL
        // given on the command line that points somewhere else
        let same_server = final_url
            .as_deref()
            .is_none_or(|url| url.trim_end_matches('/') == profile.url.trim_end_matches('/'));
        if final_token.is_none() && same_server {
            final_token = profile.token;
        }
        if final_url.is_none() {
            final_url = Some(profile.url);
        }
    }

    let resolved_url = final_url.unwrap_or_else(|| "http://localhost:3000".to_string());
    if needs_token(&cli.command) {
        utils::require_token(final_token.as_deref())?;
    }
    let client = api_client(
        &resolved_url,
        final_token.as_deref(),
        cli.ca_cert.as_deref(),
    )?;

    match cli.command {
        Commands::Run {
            file,
            input,
            tail,
            watch,
        } => {
            run::handle(&client, file, input, tail, watch).await?;
        }

        Commands::Runs { command } => {
            runs::handle(&client, command).await?;
        }

        Commands::Webhooks { command } => {
            webhooks::handle(&client, command).await?;
        }

        Commands::Rules { command } => {
            rules::handle(&client, command).await?;
        }

        Commands::Connections { command } => {
            connections::handle(&client, command).await?;
        }

        Commands::Cron { command } => {
            cron::handle(&client, command).await?;
        }

        Commands::Lint(command) => {
            lint::handle(&client, command).await?;
        }

        Commands::Schema { command } => {
            schema::handle(command)?;
        }

        Commands::Auth { command } => {
            auth::handle(&client, command).await?;
        }

        Commands::Login {
            callback_port,
            print_token,
            issuer,
            client_id,
        } => {
            if issuer.is_some() || client_id.is_some() {
                eprintln!("--issuer and --client-id are ignored: login goes through the API.");
            }
            auth::handle_login(&client, &resolved_url, callback_port, print_token).await?;
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

    #[test]
    fn only_api_commands_need_a_token() {
        let parse = |args: &[&str]| {
            let mut argv = vec!["stormchaser"];
            argv.extend_from_slice(args);
            Cli::try_parse_from(argv).unwrap().command
        };
        let id = stormchaser_model::RunId::new_v4().to_string();
        assert!(needs_token(&parse(&["runs", "get", &id])));
        assert!(needs_token(&parse(&["webhooks", "list"])));
        assert!(!needs_token(&parse(&["runs", "approve-link", "sealed"])));
        assert!(!needs_token(&parse(&["schema", "generate"])));
    }

    #[test]
    fn a_ca_certificate_that_is_not_pem_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, "not a certificate").unwrap();
        let err = api_client("https://example.com", None, Some(&path)).unwrap_err();
        assert!(format!("{err:#}").contains("ca.pem"), "{err:#}");
        api_client(
            "https://example.com",
            None,
            Some(&dir.path().join("missing")),
        )
        .unwrap_err();
    }
}
