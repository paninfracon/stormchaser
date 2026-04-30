//! Thin binary wrapper for the Stormchaser CLI.
//!
//! This executable simply imports and runs the core logic from the `stormchaser_cli` library.

use anyhow::Result;
use clap::Parser;
use stormchaser_cli::{run_cli, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_cli(cli).await
}
