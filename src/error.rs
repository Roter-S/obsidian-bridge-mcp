use std::path::PathBuf;

use rmcp::ErrorData;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ObsidianError {
    #[error("invalid vault spec `{0}`; expected NAME=/absolute/path")]
    InvalidVaultSpec(String),
    #[error("duplicate vault name `{0}`")]
    DuplicateVault(String),
    #[error("vault path must be absolute: {0}")]
    VaultPathNotAbsolute(PathBuf),
    #[error("vault path does not exist: {0}")]
    VaultPathNotFound(PathBuf),
    #[error("vault path is not a directory: {0}")]
    VaultPathNotDirectory(PathBuf),
    #[error("vault is missing .obsidian directory: {0}")]
    VaultMissingObsidianDir(PathBuf),
    #[error("unknown vault `{0}`")]
    UnknownVault(String),
    #[error("no vaults are configured")]
    NoVaultsConfigured,
    #[error(
        "multiple vaults are configured ({0}); pass vault with one of these exact names, or call list_vaults"
    )]
    AmbiguousVault(String),
    #[error("path cannot be empty")]
    EmptyPath,
    #[error("search query cannot be empty; use list_notes with folder to browse notes")]
    EmptyQuery,
    #[error("absolute paths are not allowed: {0}")]
    AbsolutePath(String),
    #[error("path traversal is not allowed: {0}")]
    PathTraversal(String),
    #[error("path must use explicit .md extension: {0}")]
    NonMarkdownPath(String),
    #[error("access to .obsidian is not allowed: {0}")]
    ObsidianConfigAccess(String),
    #[error("symlinks are not allowed: {0}")]
    SymlinkNotAllowed(PathBuf),
    #[error("note not found: {0}")]
    NoteNotFound(String),
    #[error("folder not found: {0}")]
    FolderNotFound(String),
    #[error("note already exists: {0}")]
    NoteAlreadyExists(String),
    #[error("text to replace cannot be empty")]
    EmptyPatchText,
    #[error("text to replace was not found in note `{path}`")]
    TextNotFoundInNote { path: String },
    #[error(
        "text to replace appears {occurrences} times in note `{path}`; provide more surrounding context to ensure uniqueness"
    )]
    AmbiguousPatchText { path: String, occurrences: usize },
    #[error("path is not a file: {0}")]
    NotAFile(PathBuf),
    #[error("path is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("i/o error while reading `{path}`: {source}")]
    ReadNote {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}

impl ObsidianError {
    pub fn into_mcp(self) -> ErrorData {
        match self {
            Self::NoteNotFound(_) | Self::FolderNotFound(_) | Self::UnknownVault(_) => {
                ErrorData::resource_not_found(self.to_string(), None)
            }
            Self::Io(_) => ErrorData::internal_error(self.to_string(), None),
            _ => ErrorData::invalid_params(self.to_string(), None),
        }
    }
}

impl From<ObsidianError> for ErrorData {
    fn from(value: ObsidianError) -> Self {
        value.into_mcp()
    }
}
