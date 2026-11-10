//! Opt-in comparison of what an agent reads back from this server versus from
//! the shell commands it would otherwise run on the vault folder.
//! Run: cargo test --release tools::benchmarks::shell_comparison -- --ignored --nocapture
//!
//! Both sides answer the same question on the same generated vault. Sizes are
//! the exact bytes returned; tokens are estimated as bytes / 4 and depend on
//! the model. Server calls are timed in-process (no stdio round trip); shell
//! commands are timed including process start. `grep`, `find`, `sed`, `sort`,
//! `uniq` and, when installed, `rg` are the system's own and are only measured, never depended on.

use std::future::Future;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use rmcp::handler::server::wrapper::Parameters;

use super::*;
use crate::cli::VaultSpec;
use crate::vault::benchmarks::{LINK_EVERY, RARE_EVERY, TOPICS, build_vault};

const NOTES: usize = 20_000;
const SAMPLES: usize = 5;
const LONG_NOTE_LINES: usize = 2_000;

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn report(how: &str, bytes: usize, results: usize, milliseconds: f64) {
    println!(
        "  {how:66} {bytes:>10} B  ~{:>9} tok  {results:>7} results  {milliseconds:>9.2} ms",
        bytes.div_ceil(4)
    );
}

// Bytes and lines an agent would read from the command's standard output.
fn shell(root: &Path, command: &str) {
    let run = || {
        let start = Instant::now();
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(command)
            .current_dir(root)
            .output()
            .expect("shell command runs");
        (start.elapsed().as_secs_f64() * 1000.0, output.stdout)
    };
    run(); // Warm filesystem caches, as for the server calls.
    let mut samples = Vec::new();
    let mut stdout = Vec::new();
    for _ in 0..SAMPLES {
        let (elapsed, output) = run();
        samples.push(elapsed);
        stdout = output;
    }
    let lines = stdout.iter().filter(|&&byte| byte == b'\n').count();
    report(command, stdout.len(), lines, median(samples));
}

// ripgrep is what many agents search with. It is measured only when installed.
fn ripgrep(root: &Path, command: &str) {
    let installed = Command::new("/bin/sh")
        .args(["-c", "command -v rg"])
        .output()
        .is_ok_and(|output| output.status.success());
    if installed {
        shell(root, command);
    } else {
        println!("  {command:66} skipped: rg is not installed");
    }
}

// The single text block the tool returns, which is what the model reads.
async fn server<F: Future<Output = ToolResult>>(
    how: &str,
    results: impl Fn(&str) -> usize,
    call: impl Fn() -> F,
) {
    let run = || async {
        let start = Instant::now();
        let result = call().await.expect("tool result");
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let value = serde_json::to_value(result).unwrap();
        assert_ne!(value["isError"], true, "{value}");
        (
            elapsed,
            value["content"][0]["text"].as_str().unwrap().to_owned(),
        )
    };
    run().await;
    let mut samples = Vec::new();
    let mut text = String::new();
    for _ in 0..SAMPLES {
        let (elapsed, output) = run().await;
        samples.push(elapsed);
        text = output;
    }
    report(how, text.len(), results(&text), median(samples));
}

// The leading number of the status line, such as `[20 notes; more: offset=20]`.
fn count(text: &str) -> usize {
    let digits: String = text[1..].chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap()
}

fn search(query: &str) -> Parameters<SearchNotesParams> {
    Parameters(SearchNotesParams {
        vault: None,
        folder: None,
        query: query.into(),
        limit: None,
        path_only: None,
        offset: None,
    })
}

fn list(tag: Option<&str>) -> Parameters<ListNotesParams> {
    Parameters(ListNotesParams {
        vault: None,
        folder: None,
        tag: tag.map(str::to_owned),
        limit: None,
        details: None,
        offset: None,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "opt-in measurement; creates a temporary synthetic vault and runs system shell tools"]
async fn shell_comparison() {
    let (temp, lines) = build_vault(NOTES);
    let root = temp.path();
    let long_note: String = (1..=LONG_NOTE_LINES)
        .map(|line| format!("Línea {line}: texto de una nota larga sobre el diseño del sistema.\n"))
        .collect();
    std::fs::write(root.join("long-note.md"), &long_note).unwrap();
    let registry = VaultRegistry::from_specs(vec![VaultSpec {
        name: "benchmark".into(),
        path: root.to_path_buf(),
    }])
    .unwrap();
    let service = &ObsidianService::new(registry);
    println!(
        "{NOTES} generated notes, {lines} lines, plus one {LONG_NOTE_LINES}-line note; median of {SAMPLES}, warmed caches"
    );

    println!("List the notes in the vault");
    shell(root, "find . -name '*.md'");
    shell(root, "find . -name '*.md' | head -100");
    ripgrep(root, "rg --files -g '*.md'");
    ripgrep(root, "rg --files -g '*.md' | head -100");
    server("list_notes", count, || service.list_notes(list(None))).await;

    println!("Find a common word");
    shell(root, "grep -rni 'project' .");
    shell(root, "grep -rni 'project' . | head -20");
    ripgrep(root, "rg -ni 'project'");
    ripgrep(root, "rg -ni 'project' | head -20");
    server("search_notes query=project", count, || {
        service.search_notes(search("project"))
    })
    .await;

    println!(
        "Find a phrase typed without its accent ({} notes contain \"canción rara\")",
        NOTES / RARE_EVERY
    );
    shell(root, "grep -rni 'cancion rara' .");
    ripgrep(root, "rg -ni 'cancion rara'");
    server("search_notes query='cancion rara'", count, || {
        service.search_notes(search("cancion rara"))
    })
    .await;

    println!("Read a {LONG_NOTE_LINES}-line note");
    shell(root, "cat long-note.md");
    shell(root, "sed -n '1,200p' long-note.md");
    server(
        "read_note",
        |text| text.lines().count() - 1,
        || {
            service.read_note(Parameters(ReadNoteParams {
                vault: None,
                path: "long-note.md".into(),
                offset: None,
                limit: None,
            }))
        },
    )
    .await;

    println!(
        "Find the notes that link to a note ({} do)",
        NOTES / LINK_EVERY
    );
    shell(root, r"grep -rn '\[\[Target' .");
    shell(root, r"grep -rn '\[\[Target' . | head -50");
    ripgrep(root, r"rg -n '\[\[Target'");
    ripgrep(root, r"rg -n '\[\[Target' | head -50");
    server("get_backlinks path=Target", count, || {
        service.get_backlinks(Parameters(GetBacklinksParams {
            vault: None,
            path: "Target".into(),
            limit: None,
            offset: None,
        }))
    })
    .await;

    println!("Count the tags ({TOPICS} inline tags and 7 frontmatter tags exist)");
    shell(
        root,
        "grep -rhoE '#[A-Za-z][A-Za-z0-9_/-]*' . | sort | uniq -c | sort -rn",
    );
    ripgrep(
        root,
        "rg -oIN '#[A-Za-z][A-Za-z0-9_/-]*' | sort | uniq -c | sort -rn",
    );
    server("list_tags", count, || {
        service.list_tags(Parameters(ListTagsParams {
            vault: None,
            limit: None,
            offset: None,
        }))
    })
    .await;

    println!(
        "Find the notes with a tag ({} have #topic-7)",
        NOTES / TOPICS
    );
    shell(root, "grep -rlw '#topic-7' .");
    shell(root, "grep -rlw '#topic-7' . | head -100");
    ripgrep(root, r"rg -l '#topic-7\b'");
    ripgrep(root, r"rg -l '#topic-7\b' | head -100");
    server("list_notes tag=topic-7", count, || {
        service.list_notes(list(Some("topic-7")))
    })
    .await;
}
