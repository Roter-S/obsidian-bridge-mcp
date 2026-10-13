use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::{cli::VaultSpec, vault::VaultRegistry};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub vaults: Vec<VaultSpec>,
}

pub fn path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|v| PathBuf::from(v).join(".config"))
        })
        .context("Cannot locate configuration; use --config PATH and run obm setup")?;
    Ok(base.join("obsidian-bridge-mcp/config.json"))
}

impl Config {
    pub fn validate(&self) -> Result<VaultRegistry> {
        if self.vaults.is_empty() {
            bail!("configuration requires at least one vault");
        }
        for spec in &self.vaults {
            if spec.name.trim().is_empty() {
                bail!("vault name cannot be empty");
            }
            if !spec.path.is_absolute() {
                bail!("vault path must be absolute: {}", spec.path.display());
            }
        }
        Ok(VaultRegistry::from_specs(self.vaults.clone())?)
    }

    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path).with_context(|| {
            format!(
                "Cannot read configuration {}. Run obm setup --config {}",
                path.display(),
                path.display()
            )
        })?;
        serde_json::from_slice(&data).with_context(|| {
            format!(
                "Invalid configuration {}. Run obm setup --config {}",
                path.display(),
                path.display()
            )
        })
    }
}
