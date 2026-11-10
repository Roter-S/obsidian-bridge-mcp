use std::sync::Arc;

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ObsidianError;
use crate::vault::{BacklinkMatch, HeadingItem, NoteInfo, TagInfo, VaultInfo, VaultRegistry};

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
        description = "Return modified date and size per note instead of bare paths. Defaults to false for minimal token usage."
    )]
    pub details: Option<bool>,
    #[schemars(description = "Zero-based result offset; use next_offset from previous response")]
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
        description = "Maximum lines to read (default 200, max 500). Follow next_offset for more."
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
    #[schemars(description = "Zero-based match offset; use next_offset from previous response")]
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
        description = "total_lines from your latest read_note of this note. The write is rejected if the note has a different line count, which protects against replacing a note you only partly read."
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
    #[schemars(description = "Zero-based match offset; follow next_offset")]
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
    #[schemars(description = "Zero-based result offset; follow next_offset")]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
}

#[derive(Serialize)]
struct ListVaultsResult {
    vaults: Vec<VaultInfo>,
}

// Exactly one of `notes` (compact) or `details` is present.
#[derive(Serialize)]
struct ListNotesResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Vec<NoteInfo>>,
    #[serde(flatten)]
    page: Page,
}

#[derive(Serialize)]
struct Page {
    count: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

impl Page {
    fn new(offset: usize, count: usize, has_more: bool) -> Self {
        Self {
            count,
            has_more,
            next_offset: has_more.then(|| offset.saturating_add(count)),
        }
    }
}

#[derive(Serialize)]
struct ReadNoteResult {
    content: String,
    total_lines: usize,
    lines_read: usize,
    has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
}

#[derive(Serialize)]
struct GetNoteOutlineResult {
    headings: Vec<HeadingItem>,
    total_lines: usize,
}

// Matching lines are grouped under their note so its path is sent once.
#[derive(Serialize)]
struct SearchNote {
    path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    lines: Vec<SearchLine>,
}

#[derive(Serialize)]
struct SearchLine {
    line: usize,
    excerpt: String,
}

#[derive(Serialize)]
struct SearchNotesResult {
    matches: Vec<SearchNote>,
    #[serde(flatten)]
    page: Page,
}

#[derive(Serialize)]
struct UpdateNoteResult {
    updated: bool,
    previous_total_lines: usize,
    total_lines: usize,
}

#[derive(Serialize)]
struct GetBacklinksResult {
    backlinks: Vec<BacklinkMatch>,
    #[serde(flatten)]
    page: Page,
}

#[derive(Serialize)]
struct ListTagsResult {
    tags: Vec<TagInfo>,
    #[serde(flatten)]
    page: Page,
}

type ToolResult = Result<CallToolResult, ErrorData>;

// One compact JSON text block per result: every MCP client can show it, and
// the payload is not repeated as structured content. Failures are tool-level
// results (`isError`), which clients pass to the model; a JSON-RPC protocol
// error is often shown opaquely and cannot be corrected by the caller.
fn reply<T: Serialize>(result: Result<T, ObsidianError>) -> ToolResult {
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            return Ok(CallToolResult::error(vec![ContentBlock::text(
                error.to_string(),
            )]));
        }
    };
    let text = serde_json::to_string(&value)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

#[tool_router]
impl ObsidianService {
    #[tool(
        description = "List configured Obsidian vaults",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn list_vaults(&self) -> ToolResult {
        reply(Ok(ListVaultsResult {
            vaults: self.registry.list(),
        }))
    }

    #[tool(
        description = "Browse markdown paths recursively, no query needed. Use for folder discovery, not search_notes with an empty query. Returns notes (paths) by default; stops after the requested page. details=true returns details (path, modified, size) instead, scanning all note metadata, newest first. tag keeps only notes with that tag. Hidden folders such as .trash are skipped unless named as folder. Follow next_offset while has_more.",
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
                let (notes, details, count, has_more) = if params.details.unwrap_or(false) {
                    let (page, has_more) =
                        take_page(vault.list_notes(folder, fetch, tag).await?, offset, limit);
                    let count = page.len();
                    (None, Some(page), count, has_more)
                } else {
                    let (page, has_more) = take_page(
                        vault.list_note_paths(folder, fetch, tag).await?,
                        offset,
                        limit,
                    );
                    let count = page.len();
                    (Some(page), None, count, has_more)
                };
                Ok(ListNotesResult {
                    notes,
                    details,
                    page: Page::new(offset, count, has_more),
                })
            }
            .await,
        )
    }

    #[tool(
        description = "Read a note, default 200 lines, max 500 per call. Offset is 1-based. Follow next_offset while has_more. Use get_note_outline first only when structure is needed.",
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
                let (content, total_lines, lines_read) = vault
                    .read_note_range(&params.path, Some(offset), Some(limit))
                    .await?;
                let next = offset.saturating_add(lines_read);
                let has_more = next <= total_lines;
                Ok(ReadNoteResult {
                    content,
                    total_lines,
                    lines_read,
                    has_more,
                    next_offset: has_more.then_some(next),
                })
            }
            .await,
        )
    }

    #[tool(
        description = "Return headings with line numbers instead of the note body. Scans the file internally. Use when structure is needed before choosing lines to read.",
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
                Ok(GetNoteOutlineResult {
                    headings,
                    total_lines,
                })
            }
            .await,
        )
    }

    #[tool(
        description = "Find non-empty literal text in note content or paths (path_only=true, no body reads), ignoring case and accents. To list a folder use list_notes. Matching lines are grouped per note, ordered by path/line; limit, offset and count are in matching lines; snippets capped at 240 characters. Hidden folders such as .trash are skipped. Follow next_offset while has_more.",
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
                let count = found.len();
                let mut matches: Vec<SearchNote> = Vec::new();
                for item in found {
                    if matches.last().is_none_or(|note| note.path != item.path) {
                        matches.push(SearchNote {
                            path: item.path,
                            lines: Vec::new(),
                        });
                    }
                    if !path_only && let Some(note) = matches.last_mut() {
                        note.lines.push(SearchLine {
                            line: item.line,
                            excerpt: item.excerpt,
                        });
                    }
                }
                Ok(SearchNotesResult {
                    matches,
                    page: Page::new(offset, count, has_more),
                })
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
                Ok(serde_json::json!({ "created": true }))
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
                Ok(serde_json::json!({ "updated": true }))
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
                let (previous_total_lines, total_lines) = vault
                    .update_note(&params.path, &params.content, params.expected_total_lines)
                    .await?;
                Ok(UpdateNoteResult {
                    updated: true,
                    previous_total_lines,
                    total_lines,
                })
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
                Ok(serde_json::json!({ "patched": true }))
            }
            .await,
        )
    }

    #[tool(
        description = "Delete a note by moving it to the vault's .trash folder, as Obsidian does. Returns trashed_to; restore with move_note from that path. Links to the note are not changed.",
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
                Ok(serde_json::json!({ "deleted": true, "trashed_to": trashed_to }))
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
                Ok(serde_json::json!({ "moved": true }))
            }
            .await,
        )
    }

    #[tool(
        description = "Find wikilinks [[...]] pointing to a note. Ordered by path/line; snippets capped at 240 characters. Follow next_offset while has_more. Basename links may be ambiguous when titles repeat.",
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
                let count = backlinks.len();
                Ok(GetBacklinksResult {
                    backlinks,
                    page: Page::new(offset, count, has_more),
                })
            }
            .await,
        )
    }

    #[tool(
        description = "List tags (#tag and YAML frontmatter), sorted by usage then name. Scans the vault; reuse results. Default 100, max 500; follow next_offset while has_more. To find the notes with a tag, use list_notes with tag.",
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
                let tags: Vec<_> = tags.into_iter().skip(offset).take(limit).collect();
                let count = tags.len();
                Ok(ListTagsResult {
                    tags,
                    page: Page::new(offset, count, has_more),
                })
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
            "Obsidian notes. Configured vaults: {vaults}. Pass exact vault name when multiple; optional with one. Never guess a vault for writes. Browse folders with list_notes (no query); search_notes requires non-empty literal text, path_only avoids body reads. Paths/folders are vault-relative; notes end in .md. Read only needed pages (read_note defaults to 200 lines); follow next_offset while has_more. List/search offsets are zero-based; read offsets are 1-based. Use patch_note for small unique edits and append_note to add at the end; update_note replaces the whole note. delete_note moves a note to .trash; move_note renames. Results are JSON text; failed calls return isError with a message saying how to fix the call. No skill required."
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

    fn data(result: ToolResult) -> serde_json::Value {
        let (is_error, text) = text(result);
        assert!(!is_error, "{text}");
        serde_json::from_str(&text).expect("JSON result")
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

    #[tokio::test]
    async fn test_list_and_read_notes_default_vault() {
        let (_temp, service) = test_service();
        data(
            service
                .create_note(write("test.md", "# Hello\nLine 2\nLine 3"))
                .await,
        );

        let list = data(
            service
                .list_notes(Parameters(ListNotesParams {
                    vault: None,
                    folder: None,
                    tag: None,
                    limit: None,
                    details: None,
                    offset: None,
                }))
                .await,
        );
        assert_eq!(list["notes"], serde_json::json!(["test.md"]));
        assert!(list.get("details").is_none());

        let read = data(service.read_note(read("test.md", Some(2), Some(1))).await);
        assert_eq!(read["content"], "Line 2");
        assert_eq!(read["total_lines"], 3);
        assert_eq!(read["lines_read"], 1);
    }

    #[tokio::test]
    async fn test_patch_and_outline() {
        let (_temp, service) = test_service();
        data(
            service
                .create_note(write(
                    "doc.md",
                    "# Header\nTarget text\n## Subheader\nOther",
                ))
                .await,
        );

        let outline = data(service.get_note_outline(note("doc.md")).await);
        assert_eq!(outline["headings"].as_array().unwrap().len(), 2);

        let patch = data(
            service
                .patch_note(Parameters(PatchNoteParams {
                    vault: None,
                    path: "doc.md".into(),
                    old_text: "Target text".into(),
                    new_text: "Patched text".into(),
                }))
                .await,
        );
        assert_eq!(patch["patched"], true);

        let read = data(service.read_note(read("doc.md", None, None)).await);
        assert!(read["content"].as_str().unwrap().contains("Patched text"));
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
            data(
                service
                    .create_note(write(&format!("{name}.md"), "needle 1\nother\nneedle 3"))
                    .await,
            );
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
        let page = data(search(3, false).await);
        assert_eq!(
            page,
            serde_json::json!({
                "matches": [
                    {"path": "a.md", "lines": [
                        {"line": 1, "excerpt": "needle 1"},
                        {"line": 3, "excerpt": "needle 3"},
                    ]},
                    {"path": "b.md", "lines": [{"line": 1, "excerpt": "needle 1"}]},
                ],
                "count": 3, "has_more": true, "next_offset": 3,
            })
        );
        assert_eq!(
            data(search(5, true).await),
            serde_json::json!({
                "matches": [{"path": "a.md"}, {"path": "b.md"}],
                "count": 2, "has_more": false,
            })
        );
    }

    #[tokio::test]
    async fn update_reports_line_counts_and_delete_moves_to_trash() {
        let (temp, service) = test_service();
        data(service.create_note(write("a.md", "1\n2\n3")).await);
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
            "1\n2\n3"
        );
        assert_eq!(
            data(update("only", Some(3)).await),
            serde_json::json!({"updated": true, "previous_total_lines": 3, "total_lines": 1})
        );

        let deleted = data(service.delete_note(note("a.md")).await);
        assert_eq!(deleted["trashed_to"], ".trash/a.md");
        assert!(!temp.path().join("a.md").exists());
        let moved = service
            .move_note(Parameters(MoveNoteParams {
                vault: None,
                path: ".trash/a.md".into(),
                new_path: "restored/a.md".into(),
            }))
            .await;
        assert_eq!(data(moved)["moved"], true);
        assert_eq!(
            fs::read_to_string(temp.path().join("restored/a.md")).unwrap(),
            "only"
        );
    }
}
