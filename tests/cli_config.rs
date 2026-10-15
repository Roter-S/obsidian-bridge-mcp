use std::io::Write;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

fn run(root: &TempDir, binary: &str, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(binary)
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("xdg"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
const OLD: &str = env!("CARGO_BIN_EXE_obsidian-mcp-rs");
const OBM: &str = env!("CARGO_BIN_EXE_obm");
fn vault(root: &TempDir, name: &str) -> String {
    let path = root.path().join(name);
    std::fs::create_dir_all(path.join(".obsidian")).unwrap();
    format!("{name}={}", path.display())
}
#[test]
fn version_and_help_exit_without_loading_or_creating_configuration() {
    let root = TempDir::new().unwrap();
    let config = root.path().join("invalid.json");
    std::fs::write(&config, "not json").unwrap();
    for binary in [OBM, OLD] {
        for flag in ["--version", "-V"] {
            for args in [vec![flag], vec!["--config", config.to_str().unwrap(), flag]] {
                let out = run(&root, binary, &args, "");
                assert_eq!(out.status.code(), Some(0), "{out:?}");
                assert!(out.stderr.is_empty(), "{out:?}");
                assert_eq!(
                    String::from_utf8(out.stdout).unwrap(),
                    format!("obm {}\n", env!("CARGO_PKG_VERSION"))
                );
            }
        }
        let out = run(&root, binary, &["--help"], "");
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty());
        assert!(
            String::from_utf8(out.stdout)
                .unwrap()
                .contains("-V, --version")
        );
    }
    assert!(!root.path().join("xdg").exists());
    assert_eq!(std::fs::read_to_string(config).unwrap(), "not json");
}

#[test]
fn missing_config_is_actionable_and_noninteractive() {
    let root = TempDir::new().unwrap();
    for binary in [OLD, OBM] {
        let out = run(&root, binary, &[], "");
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert!(String::from_utf8_lossy(&out.stderr).contains("obm setup"));
        assert!(!root.path().join("xdg").exists());
    }
}
#[test]
fn setup_multiple_doctor_and_rejected_replacement() {
    let root = TempDir::new().unwrap();
    let config = root.path().join("config.json");
    let path = config.to_str().unwrap();
    let input = format!("{}\n{}\n\n", vault(&root, "one"), vault(&root, "two"));
    let out = run(&root, OLD, &["--config", path, "setup"], &input);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!(
            "Enter one vault per line (NAME=/absolute/path or /absolute/path); blank line finishes:\nSaved configuration to {path}\n"
        )
    );
    let original = std::fs::read(&config).unwrap();
    for binary in [OLD, OBM] {
        let out = run(&root, binary, &["--config", path, "doctor"], "");
        assert_eq!(out.status.code(), Some(0));
        assert!(out.stderr.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!(
                "Configuration: {path}\nFor Homebrew installations: brew update && brew upgrade obsidian-bridge-mcp\nOK: 2 vault(s)\nOK: one\nOK: two\n"
            )
        );
    }
    for input in [
        "n\n",
        "",
        "y\nbad\n\n",
        "y\n",
        "y\n\n",
        "y\na=relative\n\n",
        "y\na=/nonexistent-obm-test-vault\n\n",
    ] {
        let out = run(&root, OBM, &["--config", path, "setup"], input);
        assert_eq!(out.status.code(), Some(1));
        assert!(!out.stderr.is_empty());
        assert!(
            String::from_utf8_lossy(&out.stdout)
                .starts_with(&format!("Replace configuration {path}? [y/N]\n"))
        );
        assert_eq!(std::fs::read(&config).unwrap(), original);
    }
    let duplicate = format!("y\n{0}\n{0}\n\n", vault(&root, "duplicate"));
    let out = run(&root, OBM, &["--config", path, "setup"], &duplicate);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("duplicate vault"));
    assert_eq!(std::fs::read(&config).unwrap(), original);
}
#[test]
fn invalid_config_and_explicit_override() {
    let root = TempDir::new().unwrap();
    let config = root.path().join("config.json");
    let path = config.to_str().unwrap();
    for data in [
        "not json",
        "{\"vaults\":[]}",
        "{\"vaults\":[{\"name\":\"bad\",\"path\":\"relative\"}]}",
    ] {
        std::fs::write(&config, data).unwrap();
        let out = run(&root, OBM, &["--config", path, "doctor"], "");
        assert_eq!(out.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&out.stderr).contains("obm setup"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), data);
    }
    std::fs::write(&config, "not json").unwrap();
    let spec = vault(&root, "valid");
    for binary in [OLD, OBM] {
        let out = run(&root, binary, &["--config", path, "--vault", &spec], "");
        assert!(out.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&out.stderr).contains("configuration"));
        // EOF without MCP initialization preserves the server's prior error exit.
        assert_eq!(out.status.code(), Some(1));
    }
    assert_eq!(std::fs::read_to_string(config).unwrap(), "not json");
}
#[test]
fn doctor_explicit_override_validates_without_loading_config() {
    let root = TempDir::new().unwrap();
    let config = root.path().join("config.json");
    let path = config.to_str().unwrap();
    let stored = vault(&root, "stored");
    let setup = run(
        &root,
        OLD,
        &["--config", path, "setup", "--vault", &stored],
        "",
    );
    assert_eq!(setup.status.code(), Some(0));
    let original = std::fs::read(&config).unwrap();
    let guidance = "For Homebrew installations: brew update && brew upgrade obsidian-bridge-mcp\n";
    for binary in [OLD, OBM] {
        let out = run(
            &root,
            binary,
            &["--config", path, "doctor", "--vault", "bad=relative"],
            "",
        );
        assert_eq!(out.status.code(), Some(1), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("Configuration: explicit --vault override\n{guidance}")
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("vault path must be absolute: relative")
        );
        assert_eq!(std::fs::read(&config).unwrap(), original);
        let missing = root.path().join("missing.json");
        let override_spec = vault(&root, "override");
        let out = run(
            &root,
            binary,
            &[
                "--config",
                missing.to_str().unwrap(),
                "doctor",
                "--vault",
                &override_spec,
            ],
            "",
        );
        assert_eq!(out.status.code(), Some(0), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!(
                "Configuration: explicit --vault override\n{guidance}OK: 1 vault(s)\nOK: override\n"
            )
        );
        assert!(out.stderr.is_empty());
        assert!(!missing.exists());
        assert_eq!(std::fs::read(&config).unwrap(), original);
    }
}

#[test]
fn default_location_and_confirmed_replacement() {
    let root = TempDir::new().unwrap();
    let spec = vault(&root, "one");
    let out = run(&root, OBM, &["setup", "--vault", &spec], "");
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    let config = root.path().join("xdg/obsidian-bridge-mcp/config.json");
    let before = std::fs::read(&config).unwrap();
    let second = vault(&root, "two");
    let out = run(&root, OBM, &["setup", "--vault", &second], "y\n");
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stderr.is_empty());
    assert_ne!(std::fs::read(&config).unwrap(), before);
    let out = run(&root, OBM, &["doctor"], "");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK: two\n"));
}

#[test]
fn setup_bare_path_infers_name() {
    let root = TempDir::new().unwrap();
    let config = root.path().join("config.json");
    let path = config.to_str().unwrap();
    let vault_dir = root.path().join("The Core");
    std::fs::create_dir_all(vault_dir.join(".obsidian")).unwrap();

    // The user supplies only the directory path without "The Core="
    let input = format!("{}\n\n", vault_dir.display());
    let out = run(&root, OBM, &["--config", path, "setup"], &input);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty());

    let out = run(&root, OBM, &["--config", path, "doctor"], "");
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("OK: 1 vault(s)\nOK: The Core\n"));
}
