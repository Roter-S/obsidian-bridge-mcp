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
    #[schemars(description = "Optional folder inside the vault to list notes from")]
    pub folder: Option<String>,
    #[schemars(description = "Maximum number of notes to return (default 100, max 500)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "Include detailed metadata (modified date, size). Defaults to false for minimal token usage."
    )]
    pub details: Option<bool>,
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
    #[schemars(description = "Optional maximum number of lines to read")]
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
    #[schemars(description = "Case-insensitive text query")]
    pub query: String,
    #[schemars(description = "Maximum number of matches to return (default 20, max 200)")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
    #[schemars(
        description = "If true, only searches note titles/paths (much faster, zero body I/O)"
    )]
    pub path_only: Option<bool>,
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
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTagsParams {
    #[schemars(description = "Configured vault name. Optional if only one vault is configured.")]
    pub vault: Option<String>,
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
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadNoteResult {
    pub path: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_read: Option<usize>,
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
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteNoteResult {
    pub path: String,
    pub created: Option<bool>,
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
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListTagsResult {
    pub tags: Vec<TagInfo>,
    pub count: usize,
}

#[tool_router]
impl ObsidianService {
    #[tool(description = "List configured Obsidian vaults")]
    async fn list_vaults(&self) -> Json<ListVaultsResult> {
        Json(ListVaultsResult {
            vaults: self.registry.list(),
        })
    }

    #[tool(
        description = "List markdown notes from a vault folder (compact paths by default, details=true for metadata)"
    )]
    async fn list_notes(
        &self,
        Parameters(params): Parameters<ListNotesParams>,
    ) -> Result<Json<ListNotesResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let limit = clamp_limit(params.limit, 100, 500);

        if params.details.unwrap_or(false) {
            let note_infos = vault
                .list_notes(params.folder.as_deref(), limit)
                .await
                .map_err(ErrorData::from)?;
            let paths = note_infos.iter().map(|n| n.path.clone()).collect();
            let count = note_infos.len();
            Ok(Json(ListNotesResult {
                notes: paths,
                details: Some(note_infos),
                count,
            }))
        } else {
            let paths = vault
                .list_note_paths(params.folder.as_deref(), limit)
                .await
                .map_err(ErrorData::from)?;
            let count = paths.len();
            Ok(Json(ListNotesResult {
                notes: paths,
                details: None,
                count,
            }))
        }
    }

    #[tool(
        description = "Read a markdown note from a vault (supports optional offset line and line limit)"
    )]
    async fn read_note(
        &self,
        Parameters(params): Parameters<ReadNoteParams>,
    ) -> Result<Json<ReadNoteResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;

        if params.offset.is_some() || params.limit.is_some() {
            let (content, total_lines, lines_read) = vault
                .read_note_range(&params.path, params.offset, params.limit)
                .await
                .map_err(ErrorData::from)?;

            Ok(Json(ReadNoteResult {
                path: params.path,
                content,
                total_lines: Some(total_lines),
                lines_read: Some(lines_read),
            }))
        } else {
            let content = vault
                .read_note(&params.path)
                .await
                .map_err(ErrorData::from)?;

            Ok(Json(ReadNoteResult {
                path: params.path,
                content,
                total_lines: None,
                lines_read: None,
            }))
        }
    }

    #[tool(
        description = "Get table of contents / headings of a note with line numbers without reading the full body"
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
        description = "Search case-insensitive text across markdown notes (content search) or note paths (path_only=true)"
    )]
    async fn search_notes(
        &self,
        Parameters(params): Parameters<SearchNotesParams>,
    ) -> Result<Json<SearchNotesResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let matches = vault
            .search_notes(
                params.folder.as_deref(),
                &params.query,
                clamp_limit(params.limit, 20, 200),
                params.path_only.unwrap_or(false),
            )
            .await
            .map_err(ErrorData::from)?;

        let count = matches.len();
        Ok(Json(SearchNotesResult { matches, count }))
    }

    #[tool(
        description = "Create a new markdown note (fails if note already exists; writes atomically)"
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

    #[tool(description = "Append markdown content to an existing note")]
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

    #[tool(description = "Replace all markdown content in an existing note (writes atomically)")]
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
        description = "Find and replace an exact text snippet in an existing note (token-efficient; requires unique match)"
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
        description = "Find all notes in the vault that contain wikilinks [[...]] pointing to a given note"
    )]
    async fn get_backlinks(
        &self,
        Parameters(params): Parameters<GetBacklinksParams>,
    ) -> Result<Json<GetBacklinksResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let backlinks = vault
            .get_backlinks(&params.path, clamp_limit(params.limit, 50, 200))
            .await
            .map_err(ErrorData::from)?;

        let count = backlinks.len();
        Ok(Json(GetBacklinksResult {
            target: params.path,
            backlinks,
            count,
        }))
    }

    #[tool(
        description = "List all tags (#tag and YAML frontmatter) across the vault sorted by usage count"
    )]
    async fn list_tags(
        &self,
        Parameters(params): Parameters<ListTagsParams>,
    ) -> Result<Json<ListTagsResult>, ErrorData> {
        let vault = self.registry.get_or_default(params.vault.as_deref())?;
        let tags = vault.list_tags().await.map_err(ErrorData::from)?;
        let count = tags.len();

        Ok(Json(ListTagsResult { tags, count }))
    }
}

#[tool_handler(
    name = "obsidian",
    version = "0.2.0",
    instructions = "Use this server to read, search, and update Obsidian vault notes. When only one vault is configured, 'vault' is optional. Use 'patch_note' for token-efficient targeted edits, 'get_note_outline' to inspect structure before reading large files, and 'read_note' with offset/limit for large documents."
)]
impl ServerHandler for ObsidianService {}

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
