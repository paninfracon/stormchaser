use anyhow::Result;
use clap::Parser;
use stormchaser_cli::{run_cli, Cli};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_cli(cli).await
}
