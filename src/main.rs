mod cli;
mod error;
mod server;
mod tools;
mod vault;

use anyhow::Result;
use clap::Parser;
use tracing::Level;

use crate::cli::Cli;
use crate::tools::ObsidianService;
use crate::vault::VaultRegistry;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_max_level(Level::INFO)
        .init();

    let cli = Cli::parse();
    let specs = cli.parse_vault_specs()?;
    let registry = VaultRegistry::from_specs(specs)?;

    let service = ObsidianService::new(registry);
    server::serve(service).await
}
