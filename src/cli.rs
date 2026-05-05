use std::path::PathBuf;

use clap::Parser;

use crate::error::ObsidianError;

#[derive(Debug, Parser)]
#[command(name = "obsidian-mcp-rs")]
#[command(about = "MCP server for Obsidian vaults")]
pub struct Cli {
    /// Vault definition in the form NAME=/absolute/path/to/vault
    #[arg(long = "vault", value_name = "NAME=PATH", required = true)]
    pub vaults: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    let (name, path) = raw
        .split_once('=')
        .ok_or_else(|| ObsidianError::InvalidVaultSpec(raw.to_string()))?;

    let name = name.trim();
    let path = path.trim();

    if name.is_empty() || path.is_empty() {
        return Err(ObsidianError::InvalidVaultSpec(raw.to_string()));
    }

    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(ObsidianError::VaultPathNotAbsolute(path));
    }

    Ok(VaultSpec {
        name: name.to_string(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Cli;

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
}
