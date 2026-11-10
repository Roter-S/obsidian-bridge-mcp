use std::path::PathBuf;

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
    #[error("unknown vault `{name}`; pass vault with one of these exact names: {available}")]
    UnknownVault { name: String, available: String },
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
    #[error("tag cannot be empty; omit tag to list every note, or call list_tags")]
    EmptyTag,
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
    #[error(
        "note not found: {0}; paths are vault-relative and end in .md. Find the path with search_notes path_only=true or list_notes"
    )]
    NoteNotFound(String),
    #[error("folder not found: {0}; omit folder for the vault root, or find paths with list_notes")]
    FolderNotFound(String),
    #[error("note already exists: {0}")]
    NoteAlreadyExists(String),
    #[error("note is already in the trash: {0}; use move_note to restore it")]
    AlreadyInTrash(String),
    #[error("text to replace cannot be empty")]
    EmptyPatchText,
    #[error(
        "text to replace was not found in note `{path}`; old_text must match the note exactly, including whitespace and punctuation. Read the lines again with read_note and copy them verbatim"
    )]
    TextNotFoundInNote { path: String },
    #[error(
        "text to replace appears {occurrences} times in note `{path}` (starting at lines {lines}); provide more surrounding context to ensure uniqueness"
    )]
    AmbiguousPatchText {
        path: String,
        occurrences: usize,
        lines: String,
    },
    #[error(
        "note `{path}` has {actual} lines, not the expected {expected}; nothing was written. Read every page with read_note before replacing the whole note, or use patch_note"
    )]
    LineCountMismatch {
        path: String,
        expected: usize,
        actual: usize,
    },
    #[error("path is not a file: {0}")]
    NotAFile(PathBuf),
    #[error("i/o error while reading `{path}`: {source}")]
    ReadNote {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
}
