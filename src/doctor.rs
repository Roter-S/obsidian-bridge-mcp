use anyhow::{Context, Result};

use crate::{cli::Cli, config::Config};

pub fn run(cli: &Cli) -> Result<()> {
    let path = if cli.vaults.is_empty() {
        Some(crate::config::path(cli.config.clone())?)
    } else {
        None
    };
    if let Some(path) = &path {
        println!("Configuration: {}", path.display());
    } else {
        println!("Configuration: explicit --vault override");
    }
    println!("For Homebrew installations: brew update && brew upgrade obsidian-bridge-mcp");
    let config = if let Some(path) = &path {
        Config::load(path)?
    } else {
        Config {
            vaults: cli.parse_vault_specs()?,
        }
    };
    let registry = config.validate().context(
        "Invalid vault configuration; check paths and .obsidian directories, then run obm setup",
    )?;
    println!("OK: {} vault(s)", registry.list().len());
    for vault in registry.list() {
        println!("OK: {}", vault.name);
    }
    Ok(())
}
