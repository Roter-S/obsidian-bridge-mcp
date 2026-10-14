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
        let reply = self.call(name, arguments).await;
        assert!(reply.get("error").is_none(), "{reply}");
        assert_ne!(reply["result"]["isError"], true, "{reply}");
        let data = reply["result"]["structuredContent"].clone();
        assert!(
            data.is_object(),
            "missing portable structured output: {reply}"
        );
        data
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
    let missing_note = c
        .call("read_note", json!({"vault":"BYTE", "path":"missing.md"}))
        .await;
    assert_eq!(missing_note["error"]["code"], -32002);
    assert!(missing_note.to_string().contains("note not found"));
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
            paths.push(page["notes"][0].as_str().unwrap().to_owned());
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
        let expected = all["matches"].as_array().unwrap();
        let mut matches = Vec::new();
        for offset in 0..expected.len() {
            let page = c
                .data(
                    "search_notes",
                    json!({"query":query, "path_only":path_only, "limit":1, "offset":offset}),
                )
                .await;
            matches.push(page["matches"][0].clone());
            assert_eq!(page["has_more"], offset + 1 < expected.len());
            if path_only {
                assert!(page["matches"][0].get("excerpt").is_none());
            }
        }
        assert_eq!(&matches, expected);
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

#[tokio::test]
async fn unsafe_paths_are_rejected_over_the_wire() {
    let mut c = Client::start(false).await;
    for path in [
        "../escape.md",
        "/tmp/escape.md",
        ".obsidian/config.md",
        "plain.txt",
    ] {
        let result = c
            .call("create_note", json!({"path":path,"content":"no"}))
            .await;
        assert!(
            result.get("error").is_some(),
            "unsafe input accepted: {result}"
        );
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
    for item in matches["matches"].as_array().unwrap() {
        assert!(item["excerpt"].as_str().unwrap().chars().count() <= 241);
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
