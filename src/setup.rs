use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::{cli::Cli, config::Config};

fn line(input: &mut impl BufRead) -> Result<String> {
    let mut value = String::new();
    if input.read_line(&mut value)? == 0 {
        bail!("Setup cancelled: end of input; configuration unchanged");
    }
    Ok(value.trim().to_owned())
}

pub fn run(path: &Path, cli: &Cli) -> Result<()> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout().lock();
    if path.try_exists()? {
        writeln!(output, "Replace configuration {}? [y/N]", path.display())?;
        output.flush()?;
        if !line(&mut input)?.eq_ignore_ascii_case("y") {
            bail!("Setup cancelled: replacement declined; configuration unchanged");
        }
    }
    let mut vaults = cli.vaults.clone();
    if vaults.is_empty() {
        writeln!(
            output,
            "Enter one vault per line (NAME=/absolute/path or /absolute/path); blank line finishes:"
        )?;
        output.flush()?;
        loop {
            let value = line(&mut input)?;
            if value.is_empty() {
                break;
            }
            vaults.push(value);
        }
    }
    let specs = Cli {
        vaults,
        config: None,
        command: None,
    }
    .parse_vault_specs()?;
    let config = Config { vaults: specs };
    config
        .validate()
        .context("Invalid configuration; configuration unchanged")?;
    let data = serde_json::to_vec_pretty(&config)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    // A create-new sibling prevents truncating the existing config before all data is written.
    let pending = path.with_extension("json.pending");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .context("Cannot create pending configuration; check for a stale .json.pending file")?;
    file.write_all(&data)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&pending, path)?;
    writeln!(output, "Saved configuration to {}", path.display())?;
    Ok(())
}
