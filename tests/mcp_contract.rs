//! Exercise the public wire contract, independent of any agent SDK or skill.
use std::{process::Stdio, time::Duration};

use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    time::timeout,
};

struct Client {
    _vault: TempDir,
    _child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    id: u64,
    info: Value,
}

impl Client {
    async fn start(multiple: bool) -> Self {
        let vault = tempfile::tempdir().unwrap();
        for name in ["BYTE", "The Core"] {
            let root = vault.path().join(name);
            std::fs::create_dir_all(root.join(".obsidian")).unwrap();
            std::fs::create_dir_all(root.join("Projects/SZAPETA")).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_obm"));
        command
            .arg("--vault")
            .arg(format!("BYTE={}", vault.path().join("BYTE").display()));
        if multiple {
            command.arg("--vault").arg(format!(
                "The Core={}",
                vault.path().join("The Core").display()
            ));
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            _vault: vault,
            _child: child,
            input,
            output,
            id: 0,
            info: Value::Null,
        };
        client.info = client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-03-26", "capabilities": {},
                    "clientInfo": { "name": "rust-contract-test", "version": "1" }
                }),
            )
            .await;
        client
            .send(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
            .await;
        client
    }

    fn note(&self, path: &str, content: &str) {
        std::fs::write(self._vault.path().join("BYTE").join(path), content).unwrap();
    }

    async fn send(&mut self, value: Value) {
        let mut data = serde_json::to_vec(&value).unwrap();
        data.push(b'\n');
        self.input.write_all(&data).await.unwrap();
        self.input.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
            .await;
        timeout(Duration::from_secs(10), async {
            loop {
                let mut line = String::new();
                assert_ne!(
                    self.output.read_line(&mut line).await.unwrap(),
                    0,
                    "server closed stdout"
                );
                let value: Value =
                    serde_json::from_str(&line).expect("stdout must contain only JSON-RPC");
                if value["id"] == id {
                    return value;
                }
            }
        })
        .await
        .expect("MCP response timeout")
    }

    async fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name":name, "arguments":arguments}))
            .await
    }

    async fn ok(&mut self, name: &str, arguments: Value) -> String {
        let (is_error, text) = self.text(name, arguments).await;
        assert!(!is_error, "{text}");
        text
    }

    async fn page(&mut self, name: &str, arguments: Value) -> Page {
        Page::parse(&self.ok(name, arguments).await)
    }

    // Every tool call answers with exactly one text block and no duplicate
    // structured copy; failures are tool results the model can read.
    async fn text(&mut self, name: &str, arguments: Value) -> (bool, String) {
        let reply = self.call(name, arguments).await;
        assert!(reply.get("error").is_none(), "{reply}");
        let result = &reply["result"];
        assert!(result.get("structuredContent").is_none(), "{reply}");
        assert_eq!(result["content"].as_array().unwrap().len(), 1, "{reply}");
        assert_eq!(result["content"][0]["type"], "text", "{reply}");
        (
            result["isError"] == true,
            result["content"][0]["text"].as_str().unwrap().to_owned(),
        )
    }

    async fn failure(&mut self, name: &str, arguments: Value) -> String {
        let (is_error, text) = self.text(name, arguments).await;
        assert!(is_error, "expected a tool error: {text}");
        text
    }
}

// A result as a caller reads it: `[N things; more: offset=K]` or `[N things; end]`
// on the first line, then one line per item.
#[derive(Debug)]
struct Page {
    status: String,
    lines: Vec<String>,
    count: u64,
    next: Option<u64>,
}

impl Page {
    fn parse(text: &str) -> Self {
        let mut lines = text.lines().map(str::to_owned);
        let status = lines.next().expect("status line");
        assert!(status.starts_with('[') && status.ends_with(']'), "{text}");
        let count = status[1..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .expect("status starts with a count");
        let next = status
            .trim_end_matches(']')
            .split_once("more: offset=")
            .map(|(_, offset)| offset.parse().expect("numeric offset"));
        assert!(next.is_some() || status.ends_with("; end]"), "{status}");
        Self {
            status,
            lines: lines.collect(),
            count,
            next,
        }
    }

    // (path, line) per matching line; a path-only match reports line 0.
    fn matches(&self) -> Vec<(String, u64)> {
        let mut flat = Vec::new();
        let mut path = String::new();
        let mut path_had_lines = true;
        for line in &self.lines {
            if let Some(hit) = line.strip_prefix("  ") {
                let (number, _excerpt) = hit.split_once(": ").expect("line: text");
                flat.push((path.clone(), number.parse().unwrap()));
                path_had_lines = true;
            } else {
                if !path_had_lines {
                    flat.push((path.clone(), 0));
                }
                path = line.clone();
                path_had_lines = false;
            }
        }
        if !path_had_lines {
            flat.push((path, 0));
        }
        flat
    }
}

#[tokio::test]
async fn discovery_and_original_failures_are_actionable() {
    let mut c = Client::start(true).await;
    let instructions = c.info["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("BYTE") && instructions.contains("The Core"));
    assert!(instructions.contains("list_notes") && instructions.contains("non-empty"));
    assert!(instructions.contains("more: offset=N") && instructions.contains("`end`"));
    assert_eq!(
        c.info["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        c.ok("list_vaults", json!({})).await,
        "[2 vaults; end]\nBYTE\nThe Core"
    );
    let tools = c.request("tools/list", json!({})).await;
    let search = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "search_notes")
        .unwrap();
    assert_eq!(search["inputSchema"]["properties"]["query"]["minLength"], 1);
    assert_eq!(search["annotations"]["readOnlyHint"], true);
    assert_eq!(search["annotations"]["destructiveHint"], false);
    let update = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "update_note")
        .unwrap();
    assert_eq!(update["annotations"]["readOnlyHint"], false);
    assert_eq!(update["annotations"]["destructiveHint"], true);
    assert!(
        search["description"]
            .as_str()
            .unwrap()
            .contains("list_notes")
    );
    let ambiguous = c.failure("search_notes", json!({"query":"needle"})).await;
    assert!(
        ambiguous.contains("BYTE") && ambiguous.contains("The Core") && ambiguous.contains("vault")
    );
    for query in ["", "   "] {
        let error = c.call("search_notes", json!({"vault":"BYTE", "folder":"Projects/SZAPETA", "query":query, "path_only":true})).await.to_string();
        assert!(
            error.contains("search query cannot be empty") && error.contains("list_notes"),
            "{error}"
        );
        assert!(!error.contains("path cannot be empty"));
    }
    let unknown = c.failure("list_notes", json!({"vault":"typo"})).await;
    assert!(unknown.contains("unknown vault"));
    assert!(unknown.contains("BYTE, The Core"), "{unknown}");
    let missing = c
        .failure("list_notes", json!({"vault":"BYTE", "folder":"missing"}))
        .await;
    assert!(missing.contains("folder not found"), "{missing}");
    let missing_note = c
        .failure("read_note", json!({"vault":"BYTE", "path":"missing.md"}))
        .await;
    assert!(missing_note.contains("note not found: missing.md"));
    assert!(missing_note.contains("list_notes"), "{missing_note}");
}

#[tokio::test]
async fn browsing_and_search_have_stable_complete_pages() {
    let mut c = Client::start(false).await;
    for name in ["c", "a", "b"] {
        c.note(
            &format!("Projects/SZAPETA/{name}.md"),
            "needle first\nneedle second",
        );
    }
    for details in [false, true] {
        let mut paths = Vec::new();
        let mut offset = 0;
        loop {
            let page = c.page("list_notes", json!({"folder":"Projects/SZAPETA", "limit":1, "offset":offset, "details":details})).await;
            assert_eq!(page.count, 1, "{page:?}");
            assert_eq!(page.lines.len(), 1, "{page:?}");
            let line = &page.lines[0];
            let path = if details {
                // `modified bytes path`: a UTC time in whole seconds, then the size.
                let fields: Vec<_> = line.splitn(3, ' ').collect();
                assert!(fields[0].ends_with('Z') && fields[0].len() == 20, "{line}");
                assert_eq!(fields[1], "26", "{line}");
                fields[2].to_owned()
            } else {
                line.clone()
            };
            assert!(path.starts_with("Projects/SZAPETA/"), "{line}");
            paths.push(path);
            match page.next {
                Some(next) => offset = next,
                None => break,
            }
        }
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 3);
    }
    for path_only in [false, true] {
        let query = if path_only { "SZAPETA" } else { "needle" };
        let all = c
            .page(
                "search_notes",
                json!({"query":query, "path_only":path_only}),
            )
            .await;
        let expected = all.matches();
        assert_eq!(all.count, expected.len() as u64);
        assert_eq!(expected.len(), if path_only { 3 } else { 6 });
        assert_eq!(all.next, None);
        let mut matches = Vec::new();
        for offset in 0..expected.len() {
            let page = c
                .page(
                    "search_notes",
                    json!({"query":query, "path_only":path_only, "limit":1, "offset":offset}),
                )
                .await;
            matches.extend(page.matches());
            assert_eq!(page.count, 1);
            let more = offset + 1 < expected.len();
            assert_eq!(page.next, more.then_some(offset as u64 + 1), "{page:?}");
            // A path-only match is the path alone; a content match adds one line.
            assert_eq!(page.lines.len(), if path_only { 1 } else { 2 });
            assert_eq!(page.lines[0], expected[offset].0);
        }
        assert_eq!(matches, expected);
    }
    assert_eq!(
        c.ok("search_notes", json!({"query":"needle", "limit":3}))
            .await,
        "[3 matching lines; more: offset=3]\nProjects/SZAPETA/a.md\n  1: needle first\n  2: needle second\nProjects/SZAPETA/b.md\n  1: needle first"
    );
    assert_eq!(
        c.ok("search_notes", json!({"query":"needle", "offset":999}))
            .await,
        "[0 matching lines; end]"
    );
}

#[tokio::test]
async fn reads_are_paged_by_default_and_extreme_offsets_do_not_overflow() {
    let mut c = Client::start(false).await;
    let lines = (1..=650).map(|i| format!("line {i}")).collect::<Vec<_>>();
    c.note("large.md", &lines.join("\n"));
    assert_eq!(
        c.ok("read_note", json!({"path":"large.md"})).await,
        format!(
            "[lines 1-200 of 650; more: offset=201]\n{}",
            lines[..200].join("\n")
        )
    );
    assert_eq!(
        c.ok("read_note", json!({"path":"large.md", "limit":9999}))
            .await,
        format!(
            "[lines 1-500 of 650; more: offset=501]\n{}",
            lines[..500].join("\n")
        )
    );
    assert_eq!(
        c.ok(
            "read_note",
            json!({"path":"large.md", "offset":201, "limit":9999})
        )
        .await,
        format!("[lines 201-650 of 650; end]\n{}", lines[200..].join("\n"))
    );
    assert_eq!(
        c.ok(
            "read_note",
            json!({"path":"large.md", "offset":0, "limit":0})
        )
        .await,
        "[lines 1-1 of 650; more: offset=2]\nline 1"
    );
    assert_eq!(
        c.ok(
            "read_note",
            json!({"path":"large.md", "offset":u32::MAX, "limit":500})
        )
        .await,
        format!("[no lines at offset {}; note has 650 lines; end]", u32::MAX)
    );
    // Quotes, backslashes and tabs reach the caller exactly as stored.
    c.note("raw.md", "say \"hi\"\\n\tend");
    assert_eq!(
        c.ok("read_note", json!({"path":"raw.md"})).await,
        "[lines 1-1 of 1; end]\nsay \"hi\"\\n\tend"
    );
    assert_eq!(
        c.ok("get_note_outline", json!({"path":"large.md"})).await,
        "[0 headings; note has 650 lines]"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn partial_pages_stop_before_unvisited_subtrees_and_surface_later_errors() {
    let mut c = Client::start(false).await;
    for path in ["a.md", "b.md"] {
        c.note(path, "match [[Target]]");
    }
    let root = c._vault.path().join("BYTE");
    std::fs::create_dir(root.join("z")).unwrap();
    std::os::unix::fs::symlink(root.join("a.md"), root.join("z/link.md")).unwrap();
    assert_eq!(
        c.ok("list_notes", json!({"limit":1})).await,
        "[1 note; more: offset=1]\na.md"
    );
    for (tool, arguments) in [
        ("list_notes", json!({"limit":1,"offset":1})),
        ("list_notes", json!({"limit":1,"details":true})),
        ("list_tags", json!({})),
    ] {
        let error = c.failure(tool, arguments).await;
        assert!(error.contains("symlinks are not allowed"), "{error}");
    }
    for (tool, arguments) in [
        ("search_notes", json!({"query":"match","limit":1})),
        ("get_backlinks", json!({"path":"Target","limit":1})),
    ] {
        let first = c.page(tool, arguments.clone()).await;
        assert_eq!(first.count, 1);
        assert_eq!(first.next, Some(1));
        let mut next = arguments;
        next["offset"] = json!(1);
        let error = c.failure(tool, next).await;
        assert!(error.contains("symlinks are not allowed"), "{error}");
    }
}

#[tokio::test]
async fn unsafe_paths_are_rejected_over_the_wire() {
    let mut c = Client::start(false).await;
    for path in [
        "../escape.md",
        "/tmp/escape.md",
        ".obsidian/config.md",
        "plain.txt",
    ] {
        c.failure("create_note", json!({"path":path,"content":"no"}))
            .await;
        for tool in ["delete_note", "read_note"] {
            c.failure(tool, json!({"path":path})).await;
        }
        c.failure("move_note", json!({"path":path,"new_path":"moved.md"}))
            .await;
    }
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), c._vault.path().join("BYTE/linked")).unwrap();
        let result = c
            .call("read_note", json!({"path":"linked/secret.md"}))
            .await;
        assert!(result.to_string().contains("symlinks are not allowed"));
    }
}

#[tokio::test]
async fn snippets_and_auxiliary_results_are_bounded_and_pageable() {
    let mut c = Client::start(false).await;
    let long_line = format!("needle [[Target]] {}", "界".repeat(2000));
    for name in ["a", "b", "c"] {
        c.note(&format!("{name}.md"), &format!("{long_line}\n#tag{name}"));
    }
    let matches = c.page("search_notes", json!({"query":"needle"})).await;
    assert_eq!(matches.count, 3);
    assert_eq!(matches.lines.len(), 6);
    for line in matches.lines.iter().skip(1).step_by(2) {
        let excerpt = line.strip_prefix("  1: ").expect("indented line: text");
        assert_eq!(excerpt.chars().count(), 241);
        assert!(excerpt.ends_with('…'));
    }
    let backlinks = c
        .page("get_backlinks", json!({"path":"Target", "limit":1}))
        .await;
    assert_eq!(backlinks.status, "[1 linking line; more: offset=1]");
    assert_eq!(backlinks.matches(), [("a.md".to_owned(), 1)]);
    let next = c
        .page("get_backlinks", json!({"path":"Target", "offset":1}))
        .await;
    assert_eq!(next.status, "[2 linking lines; end]");
    assert_eq!(
        next.matches(),
        [("b.md".to_owned(), 1), ("c.md".to_owned(), 1)]
    );
    assert_eq!(
        c.ok("list_tags", json!({"limit":1})).await,
        "[1 tag; more: offset=1]\ntaga 1"
    );
    assert_eq!(
        c.ok("list_tags", json!({"limit":1,"offset":2})).await,
        "[1 tag; end]\ntagc 1"
    );
    assert_eq!(
        c.ok("get_note_outline", json!({"path":"a.md"})).await,
        "[0 headings; note has 2 lines]"
    );
}

#[tokio::test]
async fn schemas_are_strict_and_results_are_not_duplicated() {
    let mut c = Client::start(false).await;
    let tools = c.request("tools/list", json!({})).await;
    let tools = tools["result"]["tools"].as_array().unwrap();
    let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for name in ["delete_note", "move_note", "update_note", "list_notes"] {
        assert!(names.contains(&name), "{names:?}");
    }
    for tool in tools {
        assert!(tool.get("outputSchema").is_none(), "{tool}");
        if tool["name"] != "list_vaults" {
            assert_eq!(tool["inputSchema"]["additionalProperties"], false, "{tool}");
        }
        if ["delete_note", "move_note"].contains(&tool["name"].as_str().unwrap()) {
            assert_eq!(tool["annotations"]["readOnlyHint"], false);
            assert_eq!(tool["annotations"]["destructiveHint"], true);
        }
        // Descriptions must not point at fields the plain-text results no longer have.
        let description = tool.to_string();
        assert!(!description.contains("next_offset"), "{tool}");
        assert!(!description.contains("has_more"), "{tool}");
    }
    // An invented argument is rejected, not silently ignored.
    c.note("a.md", "text");
    let unknown = c
        .call("read_note", json!({"path":"a.md", "line_offset":3}))
        .await
        .to_string();
    assert!(unknown.contains("line_offset"), "{unknown}");
    assert!(!unknown.contains("[lines"), "{unknown}");
}

#[tokio::test]
async fn edits_are_exact_and_deleted_notes_leave_search() {
    let mut c = Client::start(false).await;
    c.note("crlf.md", "uno\r\ndos\r\ntres");
    assert_eq!(
        c.ok("read_note", json!({"path":"crlf.md"})).await,
        "[lines 1-3 of 3; end]\nuno\ndos\ntres"
    );
    // The text a caller just read is a valid patch, whatever the line endings.
    assert_eq!(
        c.ok(
            "patch_note",
            json!({"path":"crlf.md", "old_text":"uno\ndos", "new_text":"canción"}),
        )
        .await,
        "patched"
    );
    assert_eq!(
        c.ok("append_note", json!({"path":"crlf.md", "content":"cuatro"}))
            .await,
        "appended"
    );
    let root = c._vault.path().join("BYTE");
    assert_eq!(
        std::fs::read_to_string(root.join("crlf.md")).unwrap(),
        "canción\r\ntres\ncuatro"
    );
    let missing = c
        .failure(
            "patch_note",
            json!({"path":"crlf.md", "old_text":"absent", "new_text":"x"}),
        )
        .await;
    assert!(missing.contains("read_note"), "{missing}");

    let stale = c
        .failure(
            "update_note",
            json!({"path":"crlf.md", "content":"short", "expected_total_lines":2}),
        )
        .await;
    assert!(stale.contains("has 3 lines"), "{stale}");
    assert_eq!(
        c.ok(
            "update_note",
            json!({"path":"crlf.md", "content":"canción", "expected_total_lines":3}),
        )
        .await,
        "updated; was 3 lines, now 1"
    );

    assert_eq!(
        c.ok("search_notes", json!({"query":"CANCION"})).await,
        "[1 matching line; end]\ncrlf.md\n  1: canción"
    );
    assert_eq!(
        c.ok("list_notes", json!({"tag":"missing"})).await,
        "[0 notes; end]"
    );

    assert_eq!(
        c.ok(
            "move_note",
            json!({"path":"crlf.md", "new_path":"Projects/song.md"}),
        )
        .await,
        "moved"
    );
    assert_eq!(
        c.ok("delete_note", json!({"path":"Projects/song.md"}))
            .await,
        "deleted; moved to .trash/song.md"
    );
    assert!(root.join(".trash/song.md").is_file());
    for (tool, arguments) in [
        ("search_notes", json!({"query":"cancion"})),
        ("search_notes", json!({"query":"song", "path_only":true})),
        ("list_notes", json!({})),
    ] {
        assert_eq!(c.page(tool, arguments).await.count, 0);
    }
    assert_eq!(
        c.ok("list_notes", json!({"folder":".trash"})).await,
        "[1 note; end]\n.trash/song.md"
    );
    let gone = c
        .failure("delete_note", json!({"path":"Projects/song.md"}))
        .await;
    assert!(gone.contains("note not found"), "{gone}");
}
