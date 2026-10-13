use anyhow::Result;
use clap::Parser;
use tracing::Level;

use crate::{
    cli::{Cli, Command},
    config::Config,
    tools::ObsidianService,
    vault::VaultRegistry,
};

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Setup) => {
            let path = crate::config::path(cli.config.clone())?;
            crate::setup::run(&path, &cli)
        }
        Some(Command::Doctor) => crate::doctor::run(&cli),
        None => {
            let registry = if cli.vaults.is_empty() {
                let path = crate::config::path(cli.config.clone())?;
                Config::load(&path)?.validate()?
            } else {
                VaultRegistry::from_specs(cli.parse_vault_specs()?)?
            };
            tracing_subscriber::fmt()
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .with_max_level(Level::INFO)
                .init();
            crate::server::serve(ObsidianService::new(registry)).await
        }
    }
}
