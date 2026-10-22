//! Thin binary wrapper for the Stormchaser CLI.
//!
//! This executable simply imports and runs the core logic from the `stormchaser_cli` library.

use anyhow::Result;
use clap::Parser;
use stormchaser_cli::utils::for_stderr;
use stormchaser_cli::{run_cli, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let result = run_cli(cli).await;
    if let Err(e) = &result {
        // An error the server answered with is shown as the server put it
        if let Some(stormchaser_client::Error::Status { status, body }) = e.downcast_ref() {
            eprintln!("Error ({}): {}", status, for_stderr(body));
            std::process::exit(1);
        }
    }
    result
}
