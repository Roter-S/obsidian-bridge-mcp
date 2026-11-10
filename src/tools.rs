use std::sync::Arc;

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::error::ObsidianError;
use crate::vault::VaultRegistry;

#[derive(Debug, Clone)]
pub struct ObsidianService {
    registry: Arc<VaultRegistry>,
}

impl ObsidianService {
    pub fn new(registry: VaultRegistry) -> Self {
        Self {
            registry: Arc::new(registry),
        }
    }
}

// Unknown arguments are rejected instead of ignored, so a caller that invents
// or misspells a parameter learns about it rather than assuming it applied.

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListNotesParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Vault-relative folder, searched recursively. Omit for vault root.")]
    pub folder: Option<String>,
    #[schemars(
        description = "Only notes with this tag, inline or frontmatter, including nested tags (dev matches dev/backend). Reads note bodies."
    )]
    pub tag: Option<String>,
    #[schemars(description = "Maximum number of notes to return (default 100, max 500)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "Return `modified bytes path` per note instead of bare paths. Defaults to false for minimal token usage."
    )]
    pub details: Option<bool>,
    #[schemars(
        description = "Zero-based result offset; use the offset from the previous result's first line"
    )]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "Optional 1-indexed line number to start reading from")]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
    #[schemars(
        description = "Maximum lines to read (default 200, max 500). The first line gives the next offset."
    )]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotePathParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchNotesParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Optional folder inside the vault to constrain search")]
    pub folder: Option<String>,
    #[schemars(
        description = "Non-empty literal text, matched ignoring case and accents. To browse without a query, use list_notes."
    )]
    #[schemars(length(min = 1))]
    pub query: String,
    #[schemars(description = "Maximum number of matches to return (default 20, max 200)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "If true, only searches note titles/paths (much faster, zero body I/O)"
    )]
    pub path_only: Option<bool>,
    #[schemars(
        description = "Zero-based match offset; use the offset from the previous result's first line"
    )]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "Markdown content to write")]
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "The complete new note. Everything not included is lost.")]
    pub content: String,
    #[schemars(
        description = "The note's line count from your latest read_note (the number after `of` in its first line). The write is rejected if the note has a different line count, which protects against replacing a note you only partly read."
    )]
    #[schemars(with = "Option<u32>")]
    pub expected_total_lines: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(
        description = "Exact existing text to be replaced, copied verbatim from read_note (must be unique in the note)"
    )]
    pub old_text: String,
    #[schemars(description = "New replacement text")]
    pub new_text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Current relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "New relative .md path inside the same vault; must not exist")]
    pub new_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetBacklinksParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path or note title to find backlinks for")]
    pub path: String,
    #[schemars(description = "Maximum number of backlinks to return (default 50, max 200)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "Zero-based match offset; use the offset from the previous result's first line"
    )]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTagsParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Maximum tags to return (default 100, max 500)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "Zero-based result offset; use the offset from the previous result's first line"
    )]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
}

type ToolResult = Result<CallToolResult, ErrorData>;

// Results are plain text, the form that costs a model the fewest tokens and
// that every MCP client can show. The first line is a bracketed status, so it
// survives a client that truncates long output. Failures are tool-level results
// (`isError`), which clients pass to the model; a JSON-RPC protocol error is
// often shown opaquely and cannot be corrected by the caller.
fn reply(result: Result<String, ObsidianError>) -> ToolResult {
    Ok(match result {
        Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
        Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
    })
}

// `[20 notes; more: offset=20]` or `[3 notes; end]`.
fn status(count: usize, unit: &str, offset: usize, has_more: bool) -> String {
    let plural = if count == 1 { "" } else { "s" };
    if has_more {
        let next = offset.saturating_add(count);
        format!("[{count} {unit}{plural}; more: offset={next}]")
    } else {
        format!("[{count} {unit}{plural}; end]")
    }
}

#[tool_router]
impl ObsidianService {
    #[tool(
        description = "List configured Obsidian vaults, one name per line",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn list_vaults(&self) -> ToolResult {
        let vaults = self.registry.list();
        let mut text = status(vaults.len(), "vault", 0, false);
        for vault in vaults {
            text.push('\n');
            text.push_str(&vault.name);
        }
        reply(Ok(text))
    }

    #[tool(
        description = "Browse markdown paths recursively, no query needed. Use for folder discovery, not search_notes with an empty query. Returns one path per line; stops after the requested page. details=true returns `modified bytes path` per line instead, scanning all note metadata, newest first. tag keeps only notes with that tag. Hidden folders such as .trash are skipped unless named as folder. The first line says `more: offset=N` when more remain.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn list_notes(&self, Parameters(params): Parameters<ListNotesParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let limit = clamp_limit(params.limit, 100, 500);
                let offset = params.offset.unwrap_or(0);
                let fetch = offset.saturating_add(limit).saturating_add(1);
                let (folder, tag) = (params.folder.as_deref(), params.tag.as_deref());
                let (lines, has_more) = if params.details.unwrap_or(false) {
                    let (page, has_more) =
                        take_page(vault.list_notes(folder, fetch, tag).await?, offset, limit);
                    let lines = page
                        .into_iter()
                        .map(|note| format!("{} {} {}", note.modified, note.size, note.path))
                        .collect();
                    (lines, has_more)
                } else {
                    take_page(
                        vault.list_note_paths(folder, fetch, tag).await?,
                        offset,
                        limit,
                    )
                };
                Ok(with_lines(
                    status(lines.len(), "note", offset, has_more),
                    &lines,
                ))
            }
            .await,
        )
    }

    #[tool(
        description = "Read a note, default 200 lines, max 500 per call. Offset is 1-based. The first line is a status such as `[lines 1-200 of 950; more: offset=201]`; the note text starts on the second line, exactly as stored. Use get_note_outline first only when structure is needed.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn read_note(&self, Parameters(params): Parameters<ReadNoteParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let offset = params.offset.unwrap_or(1).max(1);
                let limit = clamp_limit(params.limit, 200, 500);
                let (content, total, read) = vault
                    .read_note_range(&params.path, Some(offset), Some(limit))
                    .await?;
                if read == 0 {
                    return Ok(format!(
                        "[no lines at offset {offset}; note has {total} lines; end]"
                    ));
                }
                let next = offset.saturating_add(read);
                let tail = if next <= total {
                    format!("more: offset={next}")
                } else {
                    "end".to_owned()
                };
                Ok(format!(
                    "[lines {offset}-{} of {total}; {tail}]\n{content}",
                    next - 1
                ))
            }
            .await,
        )
    }

    #[tool(
        description = "Return headings as `line: ## Heading` instead of the note body. Scans the file internally. Use when structure is needed before choosing lines to read.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn get_note_outline(&self, Parameters(params): Parameters<NotePathParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let (headings, total_lines) = vault.get_note_outline(&params.path).await?;
                let plural = if headings.len() == 1 { "" } else { "s" };
                let mut text = format!(
                    "[{} heading{plural}; note has {total_lines} lines]",
                    headings.len()
                );
                for heading in headings {
                    let marks = "#".repeat(heading.level.into());
                    text.push_str(&format!("\n{}: {marks} {}", heading.line, heading.text));
                }
                Ok(text)
            }
            .await,
        )
    }

    #[tool(
        description = "Find non-empty literal text in note content or paths (path_only=true, no body reads), ignoring case and accents. To list a folder use list_notes. Each matching note is a path line followed by indented `line: text` lines, ordered by path/line; limit, offset and the count are in matching lines; snippets capped at 240 characters. Hidden folders such as .trash are skipped. The first line says `more: offset=N` when more remain.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn search_notes(&self, Parameters(params): Parameters<SearchNotesParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let limit = clamp_limit(params.limit, 20, 200);
                let offset = params.offset.unwrap_or(0);
                let path_only = params.path_only.unwrap_or(false);
                let mut found = vault
                    .search_notes(
                        params.folder.as_deref(),
                        &params.query,
                        limit + 1,
                        path_only,
                        offset,
                    )
                    .await?;
                let has_more = found.len() > limit;
                found.truncate(limit);
                let unit = if path_only { "path" } else { "matching line" };
                let lines = found
                    .into_iter()
                    .map(|item| (item.path, (!path_only).then_some((item.line, item.excerpt))));
                Ok(grouped(status(lines.len(), unit, offset, has_more), lines))
            }
            .await,
        )
    }

    #[tool(
        description = "Create a new markdown note (fails if note already exists; writes atomically)",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_note(&self, Parameters(params): Parameters<WriteNoteParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                vault.create_note(&params.path, &params.content).await?;
                Ok("created".to_owned())
            }
            .await,
        )
    }

    #[tool(
        description = "Append markdown content to the end of an existing note. The content starts on a new line automatically; do not add a leading newline for that.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn append_note(&self, Parameters(params): Parameters<WriteNoteParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                vault.append_note(&params.path, &params.content).await?;
                Ok("appended".to_owned())
            }
            .await,
        )
    }

    #[tool(
        description = "Replace the WHOLE content of an existing note (writes atomically). content must be the complete new note: read every page first and pass expected_total_lines. For a small change use patch_note; to add at the end use append_note.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_note(&self, Parameters(params): Parameters<UpdateNoteParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let (previous, current) = vault
                    .update_note(&params.path, &params.content, params.expected_total_lines)
                    .await?;
                Ok(format!("updated; was {previous} lines, now {current}"))
            }
            .await,
        )
    }

    #[tool(
        description = "Find and replace an exact text snippet in an existing note (token-efficient; requires unique match). Copy old_text verbatim from read_note; it may span lines.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn patch_note(&self, Parameters(params): Parameters<PatchNoteParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                vault
                    .patch_note(&params.path, &params.old_text, &params.new_text)
                    .await?;
                Ok("patched".to_owned())
            }
            .await,
        )
    }

    #[tool(
        description = "Delete a note by moving it to the vault's .trash folder, as Obsidian does. The result names its new path; restore with move_note from that path. Links to the note are not changed.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_note(&self, Parameters(params): Parameters<NotePathParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let trashed_to = vault.delete_note(&params.path).await?;
                Ok(format!("deleted; moved to {trashed_to}"))
            }
            .await,
        )
    }

    #[tool(
        description = "Move or rename a note within its vault. Fails if new_path exists; creates parent folders. Wikilinks in other notes are NOT rewritten: check get_backlinks and patch them if needed.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn move_note(&self, Parameters(params): Parameters<MoveNoteParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                vault.move_note(&params.path, &params.new_path).await?;
                Ok("moved".to_owned())
            }
            .await,
        )
    }

    #[tool(
        description = "Find wikilinks [[...]] pointing to a note. Each linking note is a path line followed by indented `line: text` lines, ordered by path/line; the count is in linking lines; snippets capped at 240 characters. The first line says `more: offset=N` when more remain. Basename links may be ambiguous when titles repeat.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn get_backlinks(
        &self,
        Parameters(params): Parameters<GetBacklinksParams>,
    ) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let limit = clamp_limit(params.limit, 50, 200);
                let offset = params.offset.unwrap_or(0);
                let mut backlinks = vault.get_backlinks(&params.path, limit + 1, offset).await?;
                let has_more = backlinks.len() > limit;
                backlinks.truncate(limit);
                let lines = backlinks
                    .into_iter()
                    .map(|link| (link.source_path, Some((link.line, link.excerpt))));
                Ok(grouped(
                    status(lines.len(), "linking line", offset, has_more),
                    lines,
                ))
            }
            .await,
        )
    }

    #[tool(
        description = "List tags (#tag and YAML frontmatter) as `tag notes` per line, sorted by usage then name. Scans the vault; reuse results. Default 100, max 500; the first line says `more: offset=N` when more remain. To find the notes with a tag, use list_notes with tag.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn list_tags(&self, Parameters(params): Parameters<ListTagsParams>) -> ToolResult {
        reply(
            async {
                let vault = self.registry.get_or_default(params.vault.as_deref())?;
                let tags = vault.list_tags().await?;
                let offset = params.offset.unwrap_or(0);
                let limit = clamp_limit(params.limit, 100, 500);
                let has_more = tags.len().saturating_sub(offset) > limit;
                let lines: Vec<_> = tags
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|tag| format!("{} {}", tag.tag, tag.count))
                    .collect();
                Ok(with_lines(
                    status(lines.len(), "tag", offset, has_more),
                    &lines,
                ))
            }
            .await,
        )
    }
}

#[tool_handler]
impl ServerHandler for ObsidianService {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        let names: Vec<_> = self.registry.list().into_iter().map(|v| v.name).collect();
        let vaults = serde_json::to_string(&names).expect("vault names serialize");
        rmcp::model::ServerInfo::new(
            rmcp::model::ServerCapabilities::builder().enable_tools().build(),
        )
        .with_server_info(rmcp::model::Implementation::new("obsidian", env!("CARGO_PKG_VERSION")))
        .with_instructions(format!(
            "Obsidian notes. Configured vaults: {vaults}. Pass exact vault name when multiple; optional with one. Never guess a vault for writes. Browse folders with list_notes (no query); search_notes requires non-empty literal text, path_only avoids body reads. Paths/folders are vault-relative; notes end in .md. Results are plain text whose first line is a bracketed status: `more: offset=N` means more remain (call again with offset N), `end` means that was all. Read only needed pages (read_note defaults to 200 lines). List/search offsets are zero-based; read offsets are 1-based. Use patch_note for small unique edits and append_note to add at the end; update_note replaces the whole note. delete_note moves a note to .trash; move_note renames. Failed calls return isError with a message saying how to fix the call. No skill required."
        ))
    }
}

fn clamp_limit(limit: Option<usize>, default: usize, max: usize) -> usize {
    limit.unwrap_or(default).clamp(1, max)
}

// `items` holds up to offset + limit + 1 results; the extra one signals more.
fn take_page<T>(items: Vec<T>, offset: usize, limit: usize) -> (Vec<T>, bool) {
    let mut page: Vec<T> = items.into_iter().skip(offset).collect();
    let has_more = page.len() > limit;
    page.truncate(limit);
    (page, has_more)
}

fn with_lines(mut text: String, lines: &[String]) -> String {
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    text
}

// A note's path is written once, followed by its indented `line: text` lines.
fn grouped(
    mut text: String,
    items: impl Iterator<Item = (String, Option<(usize, String)>)>,
) -> String {
    let mut current: Option<String> = None;
    for (path, line) in items {
        if current.as_deref() != Some(path.as_str()) {
            text.push('\n');
            text.push_str(&path);
            current = Some(path);
        }
        if let Some((number, excerpt)) = line {
            text.push_str(&format!("\n  {number}: {excerpt}"));
        }
    }
    text
}

#[cfg(test)]
#[path = "tools_bench.rs"]
mod benchmarks;

#[cfg(test)]
mod tests {
    use std::fs;
    use tempfile::TempDir;

    use super::*;
    use crate::cli::VaultSpec;

    fn test_service() -> (TempDir, ObsidianService) {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join(".obsidian")).expect("obsidian dir");
        let registry = VaultRegistry::from_specs(vec![VaultSpec {
            name: "TestVault".into(),
            path: temp.path().to_path_buf(),
        }])
        .expect("registry");
        (temp, ObsidianService::new(registry))
    }

    // (is_error, text) of the single text block every tool returns.
    fn text(result: ToolResult) -> (bool, String) {
        let value = serde_json::to_value(result.expect("tool result")).expect("serialize");
        assert!(value.get("structuredContent").is_none(), "{value}");
        assert_eq!(value["content"].as_array().expect("content").len(), 1);
        (
            value["isError"] == true,
            value["content"][0]["text"].as_str().expect("text").into(),
        )
    }

    fn ok(result: ToolResult) -> String {
        let (is_error, text) = text(result);
        assert!(!is_error, "{text}");
        text
    }

    fn write(path: &str, content: &str) -> Parameters<WriteNoteParams> {
        Parameters(WriteNoteParams {
            vault: None,
            path: path.into(),
            content: content.into(),
        })
    }

    fn note(path: &str) -> Parameters<NotePathParams> {
        Parameters(NotePathParams {
            vault: None,
            path: path.into(),
        })
    }

    fn read(path: &str, offset: Option<usize>, limit: Option<usize>) -> Parameters<ReadNoteParams> {
        Parameters(ReadNoteParams {
            vault: None,
            path: path.into(),
            offset,
            limit,
        })
    }

    fn list(details: bool, limit: Option<usize>) -> Parameters<ListNotesParams> {
        Parameters(ListNotesParams {
            vault: None,
            folder: None,
            tag: None,
            limit,
            details: Some(details),
            offset: None,
        })
    }

    #[tokio::test]
    async fn test_list_and_read_notes_default_vault() {
        let (_temp, service) = test_service();
        assert_eq!(
            ok(service
                .create_note(write("test.md", "# Hello\nLine 2\nLine 3"))
                .await),
            "created"
        );
        assert_eq!(ok(service.list_vaults().await), "[1 vault; end]\nTestVault");
        assert_eq!(
            ok(service.list_notes(list(false, None)).await),
            "[1 note; end]\ntest.md"
        );
        let detailed = ok(service.list_notes(list(true, None)).await);
        let (status, line) = detailed.split_once('\n').unwrap();
        assert_eq!(status, "[1 note; end]");
        let fields: Vec<_> = line.split(' ').collect();
        assert!(fields[0].ends_with('Z'), "{line}");
        assert_eq!(fields[1..], ["21", "test.md"]);

        assert_eq!(
            ok(service.read_note(read("test.md", Some(2), Some(1))).await),
            "[lines 2-2 of 3; more: offset=3]\nLine 2"
        );
        // The note text follows the status line exactly as stored, unescaped.
        assert_eq!(
            ok(service.read_note(read("test.md", None, None)).await),
            "[lines 1-3 of 3; end]\n# Hello\nLine 2\nLine 3"
        );
        assert_eq!(
            ok(service.read_note(read("test.md", Some(9), None)).await),
            "[no lines at offset 9; note has 3 lines; end]"
        );
    }

    #[tokio::test]
    async fn test_patch_and_outline() {
        let (_temp, service) = test_service();
        ok(service
            .create_note(write(
                "doc.md",
                "# Header\nTarget \"text\"\n## Subheader\nOther",
            ))
            .await);
        assert_eq!(
            ok(service.get_note_outline(note("doc.md")).await),
            "[2 headings; note has 4 lines]\n1: # Header\n3: ## Subheader"
        );

        // Text copied from read_note patches without any unescaping.
        let read_back = ok(service.read_note(read("doc.md", Some(2), Some(1))).await);
        let old_text = read_back.split_once('\n').unwrap().1;
        assert_eq!(old_text, "Target \"text\"");
        let patch = service
            .patch_note(Parameters(PatchNoteParams {
                vault: None,
                path: "doc.md".into(),
                old_text: old_text.into(),
                new_text: "Patched text".into(),
            }))
            .await;
        assert_eq!(ok(patch), "patched");
        assert!(ok(service.read_note(read("doc.md", None, None)).await).contains("Patched text"));
        assert_eq!(
            ok(service.append_note(write("doc.md", "more")).await),
            "appended"
        );
    }

    #[tokio::test]
    async fn failures_are_tool_results_the_caller_can_read() {
        let (_temp, service) = test_service();
        let (is_error, message) = text(service.read_note(read("missing.md", None, None)).await);
        assert!(is_error);
        assert!(message.contains("note not found: missing.md") && message.contains("list_notes"));
        assert!(
            serde_json::from_str::<ReadNoteParams>(r#"{"path":"a.md","bogus":1}"#)
                .unwrap_err()
                .to_string()
                .contains("unknown field `bogus`")
        );
    }

    #[tokio::test]
    async fn search_groups_lines_under_each_note_and_counts_lines() {
        let (_temp, service) = test_service();
        for name in ["a", "b"] {
            ok(service
                .create_note(write(
                    &format!("{name}.md"),
                    "needle 1\nother [[Target]]\nneedle 3",
                ))
                .await);
        }
        let search = |limit, path_only| {
            service.search_notes(Parameters(SearchNotesParams {
                vault: None,
                folder: None,
                query: if path_only { ".MD" } else { "NEEDLE" }.into(),
                limit: Some(limit),
                path_only: Some(path_only),
                offset: None,
            }))
        };
        assert_eq!(
            ok(search(3, false).await),
            "[3 matching lines; more: offset=3]\na.md\n  1: needle 1\n  3: needle 3\nb.md\n  1: needle 1"
        );
        assert_eq!(ok(search(5, true).await), "[2 paths; end]\na.md\nb.md");
        assert_eq!(ok(search(1, true).await), "[1 path; more: offset=1]\na.md");

        let backlinks = service
            .get_backlinks(Parameters(GetBacklinksParams {
                vault: None,
                path: "Target".into(),
                limit: None,
                offset: None,
            }))
            .await;
        assert_eq!(
            ok(backlinks),
            "[2 linking lines; end]\na.md\n  2: other [[Target]]\nb.md\n  2: other [[Target]]"
        );
        let tags = service
            .list_tags(Parameters(ListTagsParams {
                vault: None,
                limit: None,
                offset: None,
            }))
            .await;
        assert_eq!(ok(tags), "[0 tags; end]");
    }

    #[tokio::test]
    async fn update_reports_line_counts_and_delete_moves_to_trash() {
        let (temp, service) = test_service();
        ok(service.create_note(write("a.md", "1\n2\n3 #done")).await);
        let update = |content: &str, expected| {
            service.update_note(Parameters(UpdateNoteParams {
                vault: None,
                path: "a.md".into(),
                content: content.into(),
                expected_total_lines: expected,
            }))
        };
        let (is_error, message) = text(update("partial", Some(2)).await);
        assert!(is_error && message.contains("has 3 lines"), "{message}");
        assert_eq!(
            fs::read_to_string(temp.path().join("a.md")).unwrap(),
            "1\n2\n3 #done"
        );
        assert_eq!(
            ok(update("only #done", Some(3)).await),
            "updated; was 3 lines, now 1"
        );
        let tags = service
            .list_tags(Parameters(ListTagsParams {
                vault: None,
                limit: None,
                offset: None,
            }))
            .await;
        assert_eq!(ok(tags), "[1 tag; end]\ndone 1");

        assert_eq!(
            ok(service.delete_note(note("a.md")).await),
            "deleted; moved to .trash/a.md"
        );
        assert!(!temp.path().join("a.md").exists());
        let moved = service
            .move_note(Parameters(MoveNoteParams {
                vault: None,
                path: ".trash/a.md".into(),
                new_path: "restored/a.md".into(),
            }))
            .await;
        assert_eq!(ok(moved), "moved");
        assert_eq!(
            fs::read_to_string(temp.path().join("restored/a.md")).unwrap(),
            "only #done"
        );
    }
}
