use std::sync::Arc;

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::vault::{
    BacklinkMatch, HeadingItem, NoteInfo, SearchMatch, TagInfo, VaultInfo, VaultRegistry,
};

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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNotesParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Vault-relative folder, searched recursively. Omit for vault root.")]
    pub folder: Option<String>,
    #[schemars(description = "Maximum number of notes to return (default 100, max 500)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "Include detailed metadata (modified date, size). Defaults to false for minimal token usage."
    )]
    pub details: Option<bool>,
    #[schemars(description = "Zero-based result offset; use next_offset from previous response")]
    #[schemars(with = "Option<u32>")]
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
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
pub struct GetNoteOutlineParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchNotesParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Optional folder inside the vault to constrain search")]
    pub folder: Option<String>,
    #[schemars(
        description = "Non-empty case-insensitive literal text. To browse without a query, use list_notes."
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
pub struct WriteNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "Markdown content to write")]
    pub content: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PatchNoteParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "Exact existing text to be replaced (must be unique in the note)")]
    pub old_text: String,
    #[schemars(description = "New replacement text")]
    pub new_text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
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

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListVaultsResult {
    pub vaults: Vec<VaultInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNotesResult {
    #[schemars(description = "List of note paths relative to the vault")]
    pub notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Detailed note info when details=true")]
    pub details: Option<Vec<NoteInfo>>,
    pub count: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadNoteResult {
    pub path: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_read: Option<usize>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetNoteOutlineResult {
    pub path: String,
    pub headings: Vec<HeadingItem>,
    pub total_lines: usize,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchNotesResult {
    pub matches: Vec<SearchMatch>,
    pub count: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteNoteResult {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<bool>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PatchNoteResult {
    pub path: String,
    pub patched: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetBacklinksResult {
    pub target: String,
    pub backlinks: Vec<BacklinkMatch>,
    pub count: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListTagsResult {
    pub tags: Vec<TagInfo>,
    pub count: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<usize>,
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
    async fn list_vaults(&self) -> Json<ListVaultsResult> {
        Json(ListVaultsResult {
            vaults: self.registry.list(),
        })
    }

    #[tool(
        description = "Browse markdown paths recursively, no query needed. Use for folder discovery, not search_notes with an empty query. Compact paths by default; stops after the requested page. Follow next_offset while has_more. Stable path order (details=true: scans all note metadata, newest first).",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn list_notes(
        &self,
        Parameters(params): Parameters<ListNotesParams>,
    ) -> Result<Json<ListNotesResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let limit = clamp_limit(params.limit, 100, 500);

        let offset = params.offset.unwrap_or(0);
        let fetch = offset.saturating_add(limit).saturating_add(1);
        let (notes, details, has_more) = if params.details.unwrap_or(false) {
            let infos = vault.list_notes(params.folder.as_deref(), fetch).await?;
            let mut page: Vec<_> = infos.into_iter().skip(offset).collect();
            let has_more = page.len() > limit;
            page.truncate(limit);
            (
                page.iter().map(|n| n.path.clone()).collect(),
                Some(page),
                has_more,
            )
        } else {
            let paths = vault
                .list_note_paths(params.folder.as_deref(), fetch)
                .await?;
            let mut page: Vec<_> = paths.into_iter().skip(offset).collect();
            let has_more = page.len() > limit;
            page.truncate(limit);
            (page, None, has_more)
        };
        let count = notes.len();
        Ok(Json(ListNotesResult {
            notes,
            details,
            count,
            has_more,
            next_offset: has_more.then(|| offset.saturating_add(count)),
        }))
    }

    #[tool(
        description = "Read a note, default 200 lines, max 500 per call. Offset is 1-based. Follow next_offset while has_more. Use get_note_outline first only when structure is needed.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn read_note(
        &self,
        Parameters(params): Parameters<ReadNoteParams>,
    ) -> Result<Json<ReadNoteResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;

        let offset = params.offset.unwrap_or(1).max(1);
        let limit = clamp_limit(params.limit, 200, 500);
        let (content, total_lines, lines_read) = vault
            .read_note_range(&params.path, Some(offset), Some(limit))
            .await?;
        let next = offset.saturating_add(lines_read);
        let has_more = next <= total_lines;
        Ok(Json(ReadNoteResult {
            path: params.path,
            content,
            total_lines: Some(total_lines),
            lines_read: Some(lines_read),
            has_more,
            next_offset: has_more.then_some(next),
        }))
    }

    #[tool(
        description = "Return headings with line numbers instead of the note body. Scans the file internally. Use when structure is needed before choosing lines to read.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn get_note_outline(
        &self,
        Parameters(params): Parameters<GetNoteOutlineParams>,
    ) -> Result<Json<GetNoteOutlineResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let (headings, total_lines) = vault
            .get_note_outline(&params.path)
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(GetNoteOutlineResult {
            path: params.path,
            headings,
            total_lines,
        }))
    }

    #[tool(
        description = "Find non-empty literal text in note content or paths (path_only=true, no body reads). To list a folder use list_notes. Results ordered by path/line; snippets capped at 240 characters. Follow next_offset while has_more.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn search_notes(
        &self,
        Parameters(params): Parameters<SearchNotesParams>,
    ) -> Result<Json<SearchNotesResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let limit = clamp_limit(params.limit, 20, 200);
        let offset = params.offset.unwrap_or(0);
        let mut matches = vault
            .search_notes(
                params.folder.as_deref(),
                &params.query,
                limit + 1,
                params.path_only.unwrap_or(false),
                offset,
            )
            .await
            .map_err(ErrorData::from)?;

        let has_more = matches.len() > limit;
        matches.truncate(limit);
        let count = matches.len();
        Ok(Json(SearchNotesResult {
            matches,
            count,
            has_more,
            next_offset: has_more.then(|| offset.saturating_add(count)),
        }))
    }

    #[tool(
        description = "Create a new markdown note (fails if note already exists; writes atomically)",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_note(
        &self,
        Parameters(params): Parameters<WriteNoteParams>,
    ) -> Result<Json<WriteNoteResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        vault
            .create_note(&params.path, &params.content)
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(WriteNoteResult {
            path: params.path,
            created: Some(true),
            updated: None,
        }))
    }

    #[tool(
        description = "Append markdown content to an existing note",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn append_note(
        &self,
        Parameters(params): Parameters<WriteNoteParams>,
    ) -> Result<Json<WriteNoteResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        vault
            .append_note(&params.path, &params.content)
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(WriteNoteResult {
            path: params.path,
            created: None,
            updated: Some(true),
        }))
    }

    #[tool(
        description = "Replace all markdown content in an existing note (writes atomically)",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_note(
        &self,
        Parameters(params): Parameters<WriteNoteParams>,
    ) -> Result<Json<WriteNoteResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        vault
            .update_note(&params.path, &params.content)
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(WriteNoteResult {
            path: params.path,
            created: None,
            updated: Some(true),
        }))
    }

    #[tool(
        description = "Find and replace an exact text snippet in an existing note (token-efficient; requires unique match)",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn patch_note(
        &self,
        Parameters(params): Parameters<PatchNoteParams>,
    ) -> Result<Json<PatchNoteResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        vault
            .patch_note(&params.path, &params.old_text, &params.new_text)
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(PatchNoteResult {
            path: params.path,
            patched: true,
        }))
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
    ) -> Result<Json<GetBacklinksResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let limit = clamp_limit(params.limit, 50, 200);
        let offset = params.offset.unwrap_or(0);
        let mut backlinks = vault
            .get_backlinks(&params.path, limit + 1, offset)
            .await
            .map_err(ErrorData::from)?;

        let has_more = backlinks.len() > limit;
        backlinks.truncate(limit);
        let count = backlinks.len();
        Ok(Json(GetBacklinksResult {
            target: params.path,
            backlinks,
            count,
            has_more,
            next_offset: has_more.then(|| offset.saturating_add(count)),
        }))
    }

    #[tool(
        description = "List tags (#tag and YAML frontmatter), sorted by usage then name. Scans the vault; reuse results. Default 100, max 500; follow next_offset while has_more.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn list_tags(
        &self,
        Parameters(params): Parameters<ListTagsParams>,
    ) -> Result<Json<ListTagsResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let tags = vault.list_tags().await.map_err(ErrorData::from)?;
        let offset = params.offset.unwrap_or(0);
        let limit = clamp_limit(params.limit, 100, 500);
        let has_more = tags.len().saturating_sub(offset) > limit;
        let tags: Vec<_> = tags.into_iter().skip(offset).take(limit).collect();
        let count = tags.len();

        Ok(Json(ListTagsResult {
            tags,
            count,
            has_more,
            next_offset: has_more.then(|| offset.saturating_add(count)),
        }))
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
            "Obsidian notes. Configured vaults: {vaults}. Pass exact vault name when multiple; optional with one. Never guess a vault for writes. Browse folders with list_notes (no query); search_notes requires non-empty literal text, path_only avoids body reads. Paths/folders are vault-relative; notes end in .md. Read only needed pages (read_note defaults to 200 lines); follow next_offset while has_more. List/search offsets are zero-based; read offsets are 1-based. Use patch_note for small unique edits, not full rewrites. No skill required."
        ))
    }
}

fn clamp_limit(limit: Option<usize>, default: usize, max: usize) -> usize {
    limit.unwrap_or(default).clamp(1, max)
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

    #[tokio::test]
    async fn test_list_and_read_notes_default_vault() {
        let (_temp, service) = test_service();

        service
            .create_note(Parameters(WriteNoteParams {
                vault: None,
                path: "test.md".into(),
                content: "# Hello\nLine 2\nLine 3".into(),
            }))
            .await
            .expect("create");

        let res = service
            .list_notes(Parameters(ListNotesParams {
                vault: None,
                folder: None,
                limit: None,
                details: None,
                offset: None,
            }))
            .await
            .expect("list");
        assert_eq!(res.0.notes, vec!["test.md"]);
        assert!(res.0.details.is_none());

        let read = service
            .read_note(Parameters(ReadNoteParams {
                vault: None,
                path: "test.md".into(),
                offset: Some(2),
                limit: Some(1),
            }))
            .await
            .expect("read");
        assert_eq!(read.0.content, "Line 2");
        assert_eq!(read.0.total_lines, Some(3));
        assert_eq!(read.0.lines_read, Some(1));
    }

    #[tokio::test]
    async fn test_patch_and_outline() {
        let (_temp, service) = test_service();

        service
            .create_note(Parameters(WriteNoteParams {
                vault: None,
                path: "doc.md".into(),
                content: "# Header\nTarget text\n## Subheader\nOther".into(),
            }))
            .await
            .expect("create");

        let outline = service
            .get_note_outline(Parameters(GetNoteOutlineParams {
                vault: None,
                path: "doc.md".into(),
            }))
            .await
            .expect("outline");
        assert_eq!(outline.0.headings.len(), 2);

        let patch = service
            .patch_note(Parameters(PatchNoteParams {
                vault: None,
                path: "doc.md".into(),
                old_text: "Target text".into(),
                new_text: "Patched text".into(),
            }))
            .await
            .expect("patch");
        assert!(patch.0.patched);

        let read = service
            .read_note(Parameters(ReadNoteParams {
                vault: None,
                path: "doc.md".into(),
                offset: None,
                limit: None,
            }))
            .await
            .expect("read");
        assert!(read.0.content.contains("Patched text"));
    }
}
