use std::io::Read;
use std::process::Command;
use tempfile::TempDir;

const DIST: &str = env!("CARGO_BIN_EXE_dist");

#[test]
fn test_dist_package_and_formula_end_to_end() {
    let root = TempDir::new().unwrap();
    let bin_path = root.path().join("fake_binary");
    std::fs::write(&bin_path, b"#!/bin/sh\necho hello\n").unwrap();

    let manifest = root.path().join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"test\"\nversion = \"0.1.1\"\n",
    )
    .unwrap();

    let out_dir = root.path().join("artifacts");

    let targets = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-gnu",
    ];

    for target in targets {
        let status = Command::new(DIST)
            .args([
                "package",
                "v0.1.1",
                target,
                bin_path.to_str().unwrap(),
                out_dir.to_str().unwrap(),
                "--manifest",
                manifest.to_str().unwrap(),
            ])
            .status()
            .unwrap();

        assert!(status.success(), "failed to package {target}");

        let archive_path = out_dir.join(format!("obm-0.1.1-{target}.tar.gz"));
        let sha_path = out_dir.join(format!("obm-0.1.1-{target}.tar.gz.sha256"));
        assert!(archive_path.exists());
        assert!(sha_path.exists());

        // Verify tar.gz content
        let file = std::fs::File::open(&archive_path).unwrap();
        let gz = flate2::read::GzDecoder::new(file);
        let mut tar = tar::Archive::new(gz);
        let mut entries = tar.entries().unwrap();
        let mut entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_str().unwrap(), "obm");
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"#!/bin/sh\necho hello\n");
        assert!(entries.next().is_none());
    }

    // Now test formula rendering in pure Rust
    let formula_file = root.path().join("obsidian-bridge-mcp.rb");
    let status = Command::new(DIST)
        .args([
            "formula",
            "0.1.1",
            out_dir.to_str().unwrap(),
            formula_file.to_str().unwrap(),
        ])
        .status()
        .unwrap();

    assert!(status.success(), "failed to render formula");
    assert!(formula_file.exists());

    let content = std::fs::read_to_string(&formula_file).unwrap();
    assert!(content.contains("class ObsidianBridgeMcp < Formula"));
    assert!(content.contains("version \"0.1.1\""));
    assert!(content.contains("bin.install \"obm\""));
    assert!(content.contains("on_macos do"));
    assert!(content.contains("on_linux do"));
    assert!(content.contains("obm-0.1.1-aarch64-apple-darwin.tar.gz"));
    assert!(content.contains("obm-0.1.1-x86_64-apple-darwin.tar.gz"));
    assert!(content.contains("obm-0.1.1-aarch64-unknown-linux-gnu.tar.gz"));
    assert!(content.contains("obm-0.1.1-x86_64-unknown-linux-gnu.tar.gz"));
}

#[test]
fn test_dist_rejects_version_mismatch() {
    let root = TempDir::new().unwrap();
    let bin_path = root.path().join("fake_binary");
    std::fs::write(&bin_path, b"test").unwrap();

    let manifest = root.path().join("Cargo.toml");
    std::fs::write(
        &manifest,
        "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();

    let out_dir = root.path().join("artifacts");

    let output = Command::new(DIST)
        .args([
            "package",
            "v0.1.1",
            "aarch64-apple-darwin",
            bin_path.to_str().unwrap(),
            out_dir.to_str().unwrap(),
            "--manifest",
            manifest.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("does not match manifest version"));
}
