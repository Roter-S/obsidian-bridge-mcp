use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::error::ObsidianError;

#[derive(Debug, Parser)]
#[command(
    name = "obm",
    version,
    about = "MCP server for Obsidian vaults; run obm setup to configure vaults"
)]
pub struct Cli {
    /// Vault definition in the form [NAME=]/absolute/path/to/vault
    #[arg(long = "vault", value_name = "[NAME=]PATH", global = true)]
    pub vaults: Vec<String>,
    /// Shared JSON configuration path (defaults to XDG_CONFIG_HOME or HOME/.config)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Interactively configure vaults; asks before replacing existing configuration
    Setup,
    /// Validate stored configuration or explicit --vault overrides, and show upgrade guidance
    Doctor,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VaultSpec {
    pub name: String,
    pub path: PathBuf,
}

impl Cli {
    pub fn parse_vault_specs(&self) -> Result<Vec<VaultSpec>, ObsidianError> {
        self.vaults
            .iter()
            .map(|raw| parse_vault_spec(raw))
            .collect()
    }
}

fn parse_vault_spec(raw: &str) -> Result<VaultSpec, ObsidianError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ObsidianError::InvalidVaultSpec(raw.to_string()));
    }

    let (name, path) = if let Some((name_part, path_part)) = raw.split_once('=') {
        (
            name_part.trim().to_string(),
            PathBuf::from(path_part.trim()),
        )
    } else {
        let path = PathBuf::from(raw);
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ObsidianError::InvalidVaultSpec(raw.to_string()))?
            .to_string();
        (name, path)
    };

    if name.is_empty() || path.as_os_str().is_empty() {
        return Err(ObsidianError::InvalidVaultSpec(raw.to_string()));
    }

    if !path.is_absolute() {
        return Err(ObsidianError::VaultPathNotAbsolute(path));
    }

    Ok(VaultSpec { name, path })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

    #[test]
    fn version_flags_use_package_version_and_exit_successfully() {
        for flag in ["--version", "-V"] {
            let result = Cli::try_parse_from(["obm", flag]).unwrap_err();
            assert_eq!(result.kind(), clap::error::ErrorKind::DisplayVersion);
            assert_eq!(result.exit_code(), 0);
            assert!(!result.use_stderr());
            assert_eq!(
                result.to_string(),
                format!("obm {}\n", env!("CARGO_PKG_VERSION"))
            );
        }
    }

    #[test]
    fn parses_multiple_vaults() {
        let cli = Cli::parse_from([
            "obsidian-mcp-rs",
            "--vault",
            "The Core=/tmp/the-core",
            "--vault",
            "BYTE=/tmp/byte",
        ]);

        let specs = cli.parse_vault_specs().expect("vault specs should parse");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "The Core");
        assert_eq!(specs[1].name, "BYTE");
    }

    #[test]
    fn infers_vault_name_from_folder_path_when_name_omitted() {
        let cli = Cli::parse_from([
            "obm",
            "--vault",
            "/tmp/The Core",
            "--vault",
            "Custom=/tmp/BYTE",
        ]);

        let specs = cli.parse_vault_specs().expect("vault specs should parse");

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].name, "The Core");
        assert_eq!(specs[0].path.to_str().unwrap(), "/tmp/The Core");
        assert_eq!(specs[1].name, "Custom");
        assert_eq!(specs[1].path.to_str().unwrap(), "/tmp/BYTE");
    }
}
