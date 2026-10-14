use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use flate2::Compression;
use flate2::write::GzEncoder;
use sha2::{Digest, Sha256};
use tar::{Builder, Header};

const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-gnu",
];

#[derive(Parser)]
#[command(name = "dist", about = "Distribution tooling for Obsidian Bridge MCP")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Package a native release binary into a .tar.gz archive and write a SHA256 sidecar
    Package {
        tag: String,
        target: String,
        binary: PathBuf,
        output_dir: PathBuf,
        #[arg(long, default_value = "Cargo.toml")]
        manifest: PathBuf,
    },
    /// Render a Homebrew formula from four release archives and their SHA256 sidecars
    Formula {
        version: String,
        artifact_dir: PathBuf,
        output_file: PathBuf,
    },
}

fn parse_version_from_manifest(manifest_path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read manifest at {}", manifest_path.display()))?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("version") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                let rest = rest.trim();
                let version = rest.trim_matches('"').trim_matches('\'');
                if !version.is_empty() {
                    return Ok(version.to_string());
                }
            }
        }
    }
    bail!(
        "package.version not found in manifest: {}",
        manifest_path.display()
    );
}

fn sha256_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let mut hex = String::with_capacity(result.len() * 2);
    for b in result {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

fn package_binary(
    tag: &str,
    target: &str,
    binary: &Path,
    output_dir: &Path,
    manifest: &Path,
) -> Result<()> {
    if !tag.starts_with('v') || tag.len() <= 1 {
        bail!("tag must be vMAJOR.MINOR.PATCH");
    }
    let expected_version = &tag[1..];
    let manifest_version = parse_version_from_manifest(manifest)?;
    if manifest_version != expected_version {
        bail!(
            "tag version ({}) does not match manifest version ({})",
            expected_version,
            manifest_version
        );
    }

    if !binary.is_file() {
        bail!("binary must be an executable file: {}", binary.display());
    }

    let mut binary_file = File::open(binary)
        .with_context(|| format!("cannot open binary at {}", binary.display()))?;
    let mut binary_bytes = Vec::new();
    binary_file.read_to_end(&mut binary_bytes)?;

    let archive_name = format!("obm-{expected_version}-{target}.tar.gz");
    let sha_name = format!("{archive_name}.sha256");

    let archive_path = output_dir.join(&archive_name);
    let sha_path = output_dir.join(&sha_name);

    if archive_path.exists() || sha_path.exists() {
        bail!("output artifacts already exist in {}", output_dir.display());
    }

    std::fs::create_dir_all(output_dir)?;

    let gz_file = File::create(&archive_path)?;
    let enc = GzEncoder::new(gz_file, Compression::default());
    let mut tar_builder = Builder::new(enc);

    let mut header = Header::new_gnu();
    header.set_size(binary_bytes.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();

    tar_builder.append_data(&mut header, "obm", binary_bytes.as_slice())?;
    let enc = tar_builder.into_inner()?;
    enc.finish()?;

    let archive_bytes = std::fs::read(&archive_path)?;
    let digest = sha256_digest(&archive_bytes);
    std::fs::write(&sha_path, format!("{digest}  {archive_name}\n"))?;

    Ok(())
}

fn render_formula(version: &str, artifact_dir: &Path, output_file: &Path) -> Result<()> {
    let mut hashes = std::collections::HashMap::new();

    for target in TARGETS {
        let name = format!("obm-{version}-{target}.tar.gz");
        let archive_path = artifact_dir.join(&name);
        let sha_path = artifact_dir.join(format!("{name}.sha256"));

        if !archive_path.exists() {
            bail!("missing archive: {}", archive_path.display());
        }
        if !sha_path.exists() {
            bail!("missing checksum sidecar: {}", sha_path.display());
        }

        let archive_bytes = std::fs::read(&archive_path)?;
        let digest = sha256_digest(&archive_bytes);
        let expected_sha = std::fs::read_to_string(&sha_path)?;

        let expected_line = format!("{digest}  {name}\n");
        if expected_sha.trim() != expected_line.trim() {
            bail!("checksum mismatch for {name}");
        }

        hashes.insert(target, digest);
    }

    let formula = format!(
        r#"class ObsidianBridgeMcp < Formula
  desc "Obsidian MCP bridge"
  homepage "https://github.com/Roter-S/obsidian-bridge-mcp"
  version "{version}"
  on_macos do
    on_arm do
      url "https://github.com/Roter-S/obsidian-bridge-mcp/releases/download/v{version}/obm-{version}-aarch64-apple-darwin.tar.gz"
      sha256 "{arm_macos}"
    end
    on_intel do
      url "https://github.com/Roter-S/obsidian-bridge-mcp/releases/download/v{version}/obm-{version}-x86_64-apple-darwin.tar.gz"
      sha256 "{intel_macos}"
    end
  end
  on_linux do
    on_arm do
      url "https://github.com/Roter-S/obsidian-bridge-mcp/releases/download/v{version}/obm-{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "{arm_linux}"
    end
    on_intel do
      url "https://github.com/Roter-S/obsidian-bridge-mcp/releases/download/v{version}/obm-{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "{intel_linux}"
    end
  end

  def install
    bin.install "obm"
  end
end
"#,
        version = version,
        arm_macos = hashes["aarch64-apple-darwin"],
        intel_macos = hashes["x86_64-apple-darwin"],
        arm_linux = hashes["aarch64-unknown-linux-gnu"],
        intel_linux = hashes["x86_64-unknown-linux-gnu"],
    );

    if let Some(parent) = output_file.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_file, formula)?;

    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Package {
            tag,
            target,
            binary,
            output_dir,
            manifest,
        } => package_binary(&tag, &target, &binary, &output_dir, &manifest),
        Command::Formula {
            version,
            artifact_dir,
            output_file,
        } => render_formula(&version, &artifact_dir, &output_file),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}
