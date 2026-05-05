use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::cli::VaultSpec;
use crate::error::ObsidianError;

#[derive(Debug, Clone)]
pub struct VaultRegistry {
    vaults: HashMap<String, Vault>,
}

#[derive(Debug, Clone)]
pub struct Vault {
    root: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct VaultInfo {
    pub name: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct NoteInfo {
    pub path: String,
    pub title: String,
    pub modified: String,
    pub size: u64,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchMatch {
    pub path: String,
    pub line: usize,
    pub excerpt: String,
}

impl VaultRegistry {
    pub fn from_specs(specs: Vec<VaultSpec>) -> Result<Self, ObsidianError> {
        let mut vaults = HashMap::with_capacity(specs.len());

        for spec in specs {
            if vaults.contains_key(&spec.name) {
                return Err(ObsidianError::DuplicateVault(spec.name));
            }

            let vault = Vault::new(spec.name.clone(), spec.path)?;
            vaults.insert(spec.name, vault);
        }

        Ok(Self { vaults })
    }

    pub fn list(&self) -> Vec<VaultInfo> {
        let mut vaults: Vec<_> = self
            .vaults
            .keys()
            .cloned()
            .map(|name| VaultInfo { name })
            .collect();
        vaults.sort_by(|left, right| left.name.cmp(&right.name));
        vaults
    }

    pub fn get(&self, name: &str) -> Result<&Vault, ObsidianError> {
        self.vaults
            .get(name)
            .ok_or_else(|| ObsidianError::UnknownVault(name.to_string()))
    }
}

impl Vault {
    pub fn new(_name: String, root: PathBuf) -> Result<Self, ObsidianError> {
        if !root.exists() {
            return Err(ObsidianError::VaultPathNotFound(root));
        }

        if !root.is_dir() {
            return Err(ObsidianError::VaultPathNotDirectory(root));
        }

        let canonical_root = root.canonicalize()?;
        if !canonical_root.join(".obsidian").is_dir() {
            return Err(ObsidianError::VaultMissingObsidianDir(canonical_root));
        }

        Ok(Self {
            root: canonical_root,
        })
    }

    pub async fn list_notes(
        &self,
        folder: Option<&str>,
        limit: usize,
    ) -> Result<Vec<NoteInfo>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let mut notes = Vec::new();

        for entry in self.walk_markdown_files(&base)? {
            let metadata = entry
                .metadata()
                .map_err(|error| ObsidianError::Io(std::io::Error::other(error)))?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            notes.push(NoteInfo {
                path: self.relative_string(entry.path())?,
                title: entry
                    .path()
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_string(),
                modified: DateTime::<Utc>::from(modified).to_rfc3339(),
                size: metadata.len(),
            });
        }

        notes.sort_by(|left, right| {
            right
                .modified
                .cmp(&left.modified)
                .then(left.path.cmp(&right.path))
        });
        notes.truncate(limit);

        Ok(notes)
    }

    pub async fn read_note(&self, path: &str) -> Result<String, ObsidianError> {
        let path = self.resolve_note_path(path, true)?;
        Ok(fs::read_to_string(path).await?)
    }

    pub async fn search_notes(
        &self,
        folder: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchMatch>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(ObsidianError::EmptyPath);
        }

        let needle = query.to_lowercase();
        let mut matches = Vec::new();

        for entry in self.walk_markdown_files(&base)? {
            let content = fs::read_to_string(entry.path()).await?;
            for (index, line) in content.lines().enumerate() {
                if line.to_lowercase().contains(&needle) {
                    matches.push(SearchMatch {
                        path: self.relative_string(entry.path())?,
                        line: index + 1,
                        excerpt: line.trim().to_string(),
                    });
                    if matches.len() >= limit {
                        return Ok(matches);
                    }
                }
            }
        }

        Ok(matches)
    }

    pub async fn create_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, false)?;
        if fs::try_exists(&path).await? {
            return Err(ObsidianError::NoteAlreadyExists(path_to_string(&path)));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(path, content).await?;
        Ok(())
    }

    pub async fn append_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, true)?;
        let mut file = fs::OpenOptions::new().append(true).open(path).await?;
        file.write_all(content.as_bytes()).await?;
        Ok(())
    }

    pub async fn update_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, true)?;
        fs::write(path, content).await?;
        Ok(())
    }

    fn resolve_folder_path(&self, folder: Option<&str>) -> Result<PathBuf, ObsidianError> {
        let Some(folder) = folder else {
            return Ok(self.root.clone());
        };

        let relative = normalize_relative_path(folder, false)?;
        let path = self.checked_join(&relative, true)?;

        if !path.is_dir() {
            return Err(ObsidianError::FolderNotFound(folder.to_string()));
        }

        Ok(path)
    }

    fn resolve_note_path(&self, path: &str, must_exist: bool) -> Result<PathBuf, ObsidianError> {
        let relative = normalize_relative_path(path, true)?;
        let full_path = self.checked_join(&relative, must_exist)?;

        if must_exist && !full_path.exists() {
            return Err(ObsidianError::NoteNotFound(path.to_string()));
        }

        if must_exist && !full_path.is_file() {
            return Err(ObsidianError::NotAFile(full_path));
        }

        Ok(full_path)
    }

    fn checked_join(&self, relative: &Path, must_exist: bool) -> Result<PathBuf, ObsidianError> {
        let mut current = self.root.clone();
        for component in relative.components() {
            let segment = component.as_os_str();
            current.push(segment);

            match std::fs::symlink_metadata(&current) {
                Ok(metadata) => {
                    if metadata.file_type().is_symlink() {
                        return Err(ObsidianError::SymlinkNotAllowed(current));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if must_exist {
                        return Err(ObsidianError::Io(error));
                    }
                }
                Err(error) => return Err(ObsidianError::Io(error)),
            }
        }

        Ok(current)
    }

    fn walk_markdown_files(&self, base: &Path) -> Result<Vec<walkdir::DirEntry>, ObsidianError> {
        let mut files = Vec::new();

        for entry in WalkDir::new(base)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !is_ignored_dir(entry.path()))
        {
            let entry = entry.map_err(|error| ObsidianError::Io(std::io::Error::other(error)))?;
            if entry.file_type().is_symlink() {
                return Err(ObsidianError::SymlinkNotAllowed(entry.path().to_path_buf()));
            }

            if entry.file_type().is_file() && is_markdown(entry.path()) {
                files.push(entry);
            }
        }

        Ok(files)
    }

    fn relative_string(&self, path: &Path) -> Result<String, ObsidianError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| ObsidianError::PathTraversal(path_to_string(path)))?;
        Ok(path_to_string(relative))
    }
}

fn normalize_relative_path(raw: &str, require_markdown: bool) -> Result<PathBuf, ObsidianError> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(ObsidianError::EmptyPath);
    }

    let input = Path::new(raw);
    if input.is_absolute() {
        return Err(ObsidianError::AbsolutePath(raw.to_string()));
    }

    let mut normalized = PathBuf::new();
    for component in input.components() {
        match component {
            Component::Normal(segment) => {
                if segment == OsStr::new(".obsidian") {
                    return Err(ObsidianError::ObsidianConfigAccess(raw.to_string()));
                }
                normalized.push(segment);
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(ObsidianError::PathTraversal(raw.to_string()));
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ObsidianError::EmptyPath);
    }

    if require_markdown && normalized.extension() != Some(OsStr::new("md")) {
        return Err(ObsidianError::NonMarkdownPath(raw.to_string()));
    }

    Ok(normalized)
}

fn path_to_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str().map(ToOwned::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn is_markdown(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("md"))
}

fn is_ignored_dir(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new(".obsidian"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{Vault, VaultRegistry, normalize_relative_path};
    use crate::cli::VaultSpec;
    use crate::error::ObsidianError;

    fn temp_vault(name: &str) -> (TempDir, Vault) {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join(".obsidian")).expect("obsidian dir");
        let vault = Vault::new(name.to_string(), temp.path().to_path_buf()).expect("vault");
        (temp, vault)
    }

    #[tokio::test]
    async fn create_read_append_and_update_note() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("Projects/demo.md", "# Demo")
            .await
            .expect("create note");
        assert_eq!(
            vault.read_note("Projects/demo.md").await.expect("read"),
            "# Demo"
        );

        vault
            .append_note("Projects/demo.md", "\nMore")
            .await
            .expect("append note");
        assert_eq!(
            vault.read_note("Projects/demo.md").await.expect("read"),
            "# Demo\nMore"
        );

        vault
            .update_note("Projects/demo.md", "# Updated")
            .await
            .expect("update note");
        assert_eq!(
            vault.read_note("Projects/demo.md").await.expect("read"),
            "# Updated"
        );
    }

    #[tokio::test]
    async fn search_notes_returns_matches() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("Projects/demo.md", "line one\nPending auth\nline three")
            .await
            .expect("create note");

        let matches = vault
            .search_notes(Some("Projects"), "pending", 10)
            .await
            .expect("search notes");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 2);
    }

    #[test]
    fn rejects_path_traversal() {
        let error = normalize_relative_path("../secret.md", true).expect_err("must reject");
        assert!(matches!(error, ObsidianError::PathTraversal(_)));
    }

    #[test]
    fn rejects_obsidian_access() {
        let error =
            normalize_relative_path(".obsidian/workspace.md", true).expect_err("must reject");
        assert!(matches!(error, ObsidianError::ObsidianConfigAccess(_)));
    }

    #[test]
    fn rejects_non_markdown_notes() {
        let error = normalize_relative_path("Projects/demo.txt", true).expect_err("must reject");
        assert!(matches!(error, ObsidianError::NonMarkdownPath(_)));
    }

    #[test]
    fn validates_vault_registry() {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join(".obsidian")).expect("obsidian dir");

        let registry = VaultRegistry::from_specs(vec![VaultSpec {
            name: "The Core".into(),
            path: temp.path().to_path_buf(),
        }])
        .expect("registry");

        assert_eq!(registry.list().len(), 1);
    }
}
