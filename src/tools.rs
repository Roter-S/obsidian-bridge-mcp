use std::sync::Arc;

use rmcp::{
    ErrorData, ServerHandler,
    handler::server::wrapper::{Json, Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::vault::{NoteInfo, SearchMatch, VaultInfo, VaultRegistry};

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
    #[schemars(description = "Configured vault name")]
    pub vault: String,
    #[schemars(description = "Optional folder inside the vault")]
    pub folder: Option<String>,
    #[schemars(description = "Maximum number of notes to return")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadNoteParams {
    #[schemars(description = "Configured vault name")]
    pub vault: String,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchNotesParams {
    #[schemars(description = "Configured vault name")]
    pub vault: String,
    #[schemars(description = "Optional folder inside the vault")]
    pub folder: Option<String>,
    #[schemars(description = "Case-insensitive text query")]
    pub query: String,
    #[schemars(description = "Maximum number of matches to return")]
    #[schemars(with = "Option<u32>")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteNoteParams {
    #[schemars(description = "Configured vault name")]
    pub vault: String,
    #[schemars(description = "Relative .md path inside the vault")]
    pub path: String,
    #[schemars(description = "Markdown content to write")]
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListVaultsResult {
    pub vaults: Vec<VaultInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListNotesResult {
    pub notes: Vec<NoteInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadNoteResult {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchNotesResult {
    pub matches: Vec<SearchMatch>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteNoteResult {
    pub path: String,
    pub created: Option<bool>,
    pub updated: Option<bool>,
}

#[tool_router]
impl ObsidianService {
    #[tool(description = "List configured Obsidian vaults")]
    async fn list_vaults(&self) -> Json<ListVaultsResult> {
        Json(ListVaultsResult {
            vaults: self.registry.list(),
        })
    }

    #[tool(description = "List markdown notes from a vault folder")]
    async fn list_notes(
        &self,
        Parameters(params): Parameters<ListNotesParams>,
    ) -> Result<Json<ListNotesResult>, ErrorData> {
        let vault = self.registry.get(&params.vault)?;
        let notes = vault
            .list_notes(
                params.folder.as_deref(),
                clamp_limit(params.limit, 100, 500),
            )
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(ListNotesResult { notes }))
    }

    #[tool(description = "Read a markdown note from a vault")]
    async fn read_note(
        &self,
        Parameters(params): Parameters<ReadNoteParams>,
    ) -> Result<Json<ReadNoteResult>, ErrorData> {
        let vault = self.registry.get(&params.vault)?;
        let content = vault
            .read_note(&params.path)
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(ReadNoteResult {
            path: params.path,
            content,
        }))
    }

    #[tool(description = "Search case-insensitive text across markdown notes")]
    async fn search_notes(
        &self,
        Parameters(params): Parameters<SearchNotesParams>,
    ) -> Result<Json<SearchNotesResult>, ErrorData> {
        let vault = self.registry.get(&params.vault)?;
        let matches = vault
            .search_notes(
                params.folder.as_deref(),
                &params.query,
                clamp_limit(params.limit, 20, 200),
            )
            .await
            .map_err(ErrorData::from)?;

        Ok(Json(SearchNotesResult { matches }))
    }

    #[tool(description = "Create a new markdown note")]
    async fn create_note(
        &self,
        Parameters(params): Parameters<WriteNoteParams>,
    ) -> Result<Json<WriteNoteResult>, ErrorData> {
        let vault = self.registry.get(&params.vault)?;
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
        let vault = self.registry.get(&params.vault)?;
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

    #[tool(description = "Replace all markdown content in an existing note")]
    async fn update_note(
        &self,
        Parameters(params): Parameters<WriteNoteParams>,
    ) -> Result<Json<WriteNoteResult>, ErrorData> {
        let vault = self.registry.get(&params.vault)?;
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
}

#[tool_handler(
    name = "obsidian",
    version = "0.1.0",
    instructions = "Use this server to read and update markdown notes in configured Obsidian vaults. Every tool call must include the configured vault name and explicit .md paths when working with notes."
)]
impl ServerHandler for ObsidianService {}

fn clamp_limit(limit: Option<usize>, default: usize, max: usize) -> usize {
    limit.unwrap_or(default).clamp(1, max)
}
