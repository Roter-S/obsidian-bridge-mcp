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

    async fn data(&mut self, name: &str, arguments: Value) -> Value {
        let (is_error, text) = self.text(name, arguments).await;
        assert!(!is_error, "{text}");
        serde_json::from_str(&text).expect("successful results are JSON text")
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

#[tokio::test]
async fn discovery_and_original_failures_are_actionable() {
    let mut c = Client::start(true).await;
    let instructions = c.info["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("BYTE") && instructions.contains("The Core"));
    assert!(instructions.contains("list_notes") && instructions.contains("non-empty"));
    assert_eq!(
        c.info["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
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
    let ambiguous = c
        .call("search_notes", json!({"query":"needle"}))
        .await
        .to_string();
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
    let unknown = c
        .call("list_notes", json!({"vault":"typo"}))
        .await
        .to_string();
    assert!(unknown.contains("unknown vault"));
    let missing = c
        .call("list_notes", json!({"vault":"BYTE", "folder":"missing"}))
        .await
        .to_string();
    assert!(missing.contains("folder not found"), "{missing}");
    assert!(unknown.contains("BYTE, The Core"), "{unknown}");
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
            let page = c.data("list_notes", json!({"folder":"Projects/SZAPETA", "limit":1, "offset":offset, "details":details})).await;
            assert_eq!(page["count"], 1);
            let path = if details {
                assert!(page.get("notes").is_none(), "{page}");
                assert!(
                    page["details"][0]["modified"]
                        .as_str()
                        .unwrap()
                        .ends_with('Z')
                );
                &page["details"][0]["path"]
            } else {
                assert!(page.get("details").is_none(), "{page}");
                &page["notes"][0]
            };
            paths.push(path.as_str().unwrap().to_owned());
            if page["has_more"] == false {
                assert!(page.get("next_offset").is_none());
                break;
            }
            offset = page["next_offset"].as_u64().unwrap();
        }
        paths.sort();
        paths.dedup();
        assert_eq!(paths.len(), 3);
    }
    for path_only in [false, true] {
        let query = if path_only { "SZAPETA" } else { "needle" };
        let all = c
            .data(
                "search_notes",
                json!({"query":query, "path_only":path_only}),
            )
            .await;
        let expected = flat_matches(&all);
        assert_eq!(all["count"], expected.len());
        assert_eq!(expected.len(), if path_only { 3 } else { 6 });
        let mut matches = Vec::new();
        for offset in 0..expected.len() {
            let page = c
                .data(
                    "search_notes",
                    json!({"query":query, "path_only":path_only, "limit":1, "offset":offset}),
                )
                .await;
            matches.extend(flat_matches(&page));
            assert_eq!(page["count"], 1);
            assert_eq!(page["has_more"], offset + 1 < expected.len());
            if path_only {
                assert_eq!(page["matches"][0], json!({"path": expected[offset].0}));
            }
        }
        assert_eq!(matches, expected);
    }
    let empty = c
        .data("search_notes", json!({"query":"needle", "offset":999}))
        .await;
    assert_eq!(empty["count"], 0);
    assert_eq!(empty["has_more"], false);
}

#[tokio::test]
async fn reads_are_paged_by_default_and_extreme_offsets_do_not_overflow() {
    let mut c = Client::start(false).await;
    let lines = (1..=650).map(|i| format!("line {i}")).collect::<Vec<_>>();
    c.note("large.md", &lines.join("\n"));
    let first = c.data("read_note", json!({"path":"large.md"})).await;
    assert_eq!(first["lines_read"], 200);
    assert_eq!(first["total_lines"], 650);
    assert_eq!(first["next_offset"], 201);
    let capped = c
        .data("read_note", json!({"path":"large.md", "limit":9999}))
        .await;
    assert_eq!(capped["lines_read"], 500);
    assert_eq!(capped["next_offset"], 501);
    assert_eq!(capped["content"], lines[..500].join("\n"));
    let next = c
        .data(
            "read_note",
            json!({"path":"large.md", "offset":first["next_offset"], "limit":9999}),
        )
        .await;
    assert_eq!(next["lines_read"], 450);
    assert_eq!(next["has_more"], false);
    assert_eq!(next["content"], lines[200..].join("\n"));
    let zero = c
        .data(
            "read_note",
            json!({"path":"large.md", "offset":0, "limit":0}),
        )
        .await;
    assert_eq!(zero["content"], "line 1");
    let end = c
        .data(
            "read_note",
            json!({"path":"large.md", "offset":u32::MAX, "limit":500}),
        )
        .await;
    assert_eq!(end["lines_read"], 0);
    assert_eq!(end["has_more"], false);
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
    let first = c.data("list_notes", json!({"limit":1})).await;
    assert_eq!(first["notes"], json!(["a.md"]));
    assert_eq!(first["has_more"], true);
    assert_eq!(first["next_offset"], 1);
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
        let first = c.data(tool, arguments.clone()).await;
        assert_eq!(first["count"], 1);
        assert_eq!(first["has_more"], true);
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
    let matches = c.data("search_notes", json!({"query":"needle"})).await;
    assert_eq!(matches["count"], 3);
    for item in matches["matches"].as_array().unwrap() {
        assert_eq!(
            item["lines"][0]["excerpt"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            241
        );
    }
    let backlinks = c
        .data("get_backlinks", json!({"path":"Target", "limit":1}))
        .await;
    assert_eq!(backlinks["count"], 1);
    assert_eq!(backlinks["next_offset"], 1);
    let next = c
        .data("get_backlinks", json!({"path":"Target", "offset":1}))
        .await;
    assert_eq!(next["count"], 2);
    assert_ne!(
        backlinks["backlinks"][0]["source_path"],
        next["backlinks"][0]["source_path"]
    );
    let tags = c.data("list_tags", json!({"limit":1})).await;
    assert_eq!(tags["count"], 1);
    assert_eq!(tags["next_offset"], 1);
    let rest = c.data("list_tags", json!({"limit":1,"offset":2})).await;
    assert_eq!(rest["count"], 1);
    assert_eq!(rest["has_more"], false);
}

// (path, line) per matching line; path-only matches report line 0.
fn flat_matches(page: &Value) -> Vec<(String, u64)> {
    let mut flat = Vec::new();
    for note in page["matches"].as_array().unwrap() {
        let path = note["path"].as_str().unwrap().to_owned();
        match note.get("lines") {
            None => flat.push((path, 0)),
            Some(lines) => {
                for line in lines.as_array().unwrap() {
                    assert!(line["excerpt"].is_string());
                    flat.push((path.clone(), line["line"].as_u64().unwrap()));
                }
            }
        }
    }
    flat
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
    }
    // An invented argument is rejected, not silently ignored.
    c.note("a.md", "text");
    let unknown = c
        .call("read_note", json!({"path":"a.md", "line_offset":3}))
        .await
        .to_string();
    assert!(unknown.contains("line_offset"), "{unknown}");
    assert!(!unknown.contains("\"content\":\"text\""), "{unknown}");
}

#[tokio::test]
async fn edits_are_exact_and_deleted_notes_leave_search() {
    let mut c = Client::start(false).await;
    c.note("crlf.md", "uno\r\ndos\r\ntres");
    let read = c.data("read_note", json!({"path":"crlf.md"})).await;
    assert_eq!(
        read,
        json!({"content":"uno\ndos\ntres", "total_lines":3, "lines_read":3, "has_more":false})
    );
    // The text a caller just read is a valid patch, whatever the line endings.
    let patched = c
        .data(
            "patch_note",
            json!({"path":"crlf.md", "old_text":"uno\ndos", "new_text":"canción"}),
        )
        .await;
    assert_eq!(patched, json!({"patched":true}));
    assert_eq!(
        c.data("append_note", json!({"path":"crlf.md", "content":"cuatro"}))
            .await,
        json!({"updated":true})
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
        c.data(
            "update_note",
            json!({"path":"crlf.md", "content":"canción", "expected_total_lines":3}),
        )
        .await,
        json!({"updated":true, "previous_total_lines":3, "total_lines":1})
    );

    let found = c.data("search_notes", json!({"query":"CANCION"})).await;
    assert_eq!(
        found["matches"],
        json!([{"path":"crlf.md", "lines":[{"line":1, "excerpt":"canción"}]}])
    );
    let tagged = c.data("list_notes", json!({"tag":"missing"})).await;
    assert_eq!(tagged, json!({"notes":[], "count":0, "has_more":false}));

    let moved = c
        .data(
            "move_note",
            json!({"path":"crlf.md", "new_path":"Projects/song.md"}),
        )
        .await;
    assert_eq!(moved, json!({"moved":true}));
    let deleted = c
        .data("delete_note", json!({"path":"Projects/song.md"}))
        .await;
    assert_eq!(
        deleted,
        json!({"deleted":true, "trashed_to":".trash/song.md"})
    );
    assert!(root.join(".trash/song.md").is_file());
    for (tool, arguments) in [
        ("search_notes", json!({"query":"cancion"})),
        ("search_notes", json!({"query":"song", "path_only":true})),
        ("list_notes", json!({})),
    ] {
        assert_eq!(c.data(tool, arguments).await["count"], 0);
    }
    assert_eq!(
        c.data("list_notes", json!({"folder":".trash"})).await["notes"],
        json!([".trash/song.md"])
    );
    let gone = c
        .failure("delete_note", json!({"path":"Projects/song.md"}))
        .await;
    assert!(gone.contains("note not found"), "{gone}");
}
