use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use walkdir::WalkDir;

use crate::cli::VaultSpec;
use crate::error::ObsidianError;

#[cfg(test)]
#[path = "vault_bench.rs"]
mod benchmarks;

#[derive(Debug, Clone)]
pub struct VaultRegistry {
    vaults: HashMap<String, Vault>,
}

#[derive(Debug, Clone)]
pub struct Vault {
    #[allow(dead_code)]
    name: String,
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
    #[schemars(with = "i64")]
    pub size: u64,
}

// A max-heap keeps the worst retained candidate on top: oldest modified string,
// then greatest path. Match the existing response ordering, including tie breaks.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct NoteCandidate {
    modified: Reverse<String>,
    path: String,
    size: u64,
}

#[derive(Clone, Copy)]
enum TraversalOrder {
    Components,
    Lexical,
    Unordered,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchMatch {
    pub path: String,
    #[schemars(with = "i32")]
    pub line: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub excerpt: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct HeadingItem {
    pub level: u8,
    pub text: String,
    #[schemars(with = "i32")]
    pub line: usize,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct BacklinkMatch {
    pub source_path: String,
    #[schemars(with = "i32")]
    pub line: usize,
    pub excerpt: String,
}

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct TagInfo {
    pub tag: String,
    #[schemars(with = "i64")]
    pub count: usize,
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

    pub fn get_or_default(&self, name: Option<&str>) -> Result<&Vault, ObsidianError> {
        match name {
            Some(name) => self.get(name),
            None => {
                if self.vaults.len() == 1 {
                    Ok(self.vaults.values().next().expect("single vault"))
                } else if self.vaults.is_empty() {
                    Err(ObsidianError::NoVaultsConfigured)
                } else {
                    let available = self
                        .list()
                        .into_iter()
                        .map(|v| v.name)
                        .collect::<Vec<_>>()
                        .join(", ");
                    Err(ObsidianError::AmbiguousVault(available))
                }
            }
        }
    }
}

impl Vault {
    pub fn new(name: String, root: PathBuf) -> Result<Self, ObsidianError> {
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
            name,
            root: canonical_root,
        })
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn list_notes(
        &self,
        folder: Option<&str>,
        limit: usize,
    ) -> Result<Vec<NoteInfo>, ObsidianError> {
        let folder = folder.map(str::to_owned);
        self.blocking(move |vault, cancelled| {
            vault.list_notes_blocking(folder.as_deref(), limit, cancelled)
        })
        .await
    }

    fn list_notes_blocking(
        &self,
        folder: Option<&str>,
        limit: usize,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Vec<NoteInfo>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let mut notes: BinaryHeap<NoteCandidate> = BinaryHeap::new();

        // Newest-first requires examining every note, but only retain the best
        // `limit` candidates. Do not allocate capacity from an untrusted limit.
        for entry in self.walk_markdown_files(&base, cancelled, TraversalOrder::Unordered) {
            let entry = entry?;
            let metadata = entry
                .metadata()
                .map_err(|error| ObsidianError::Io(std::io::Error::other(error)))?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let candidate = NoteCandidate {
                modified: Reverse(DateTime::<Utc>::from(modified).to_rfc3339()),
                path: self.relative_string(entry.path())?,
                size: metadata.len(),
            };
            if notes.len() < limit {
                notes.push(candidate);
            } else if let Some(mut worst) = notes.peek_mut()
                && candidate < *worst
            {
                *worst = candidate;
            }
        }

        Ok(notes
            .into_sorted_vec()
            .into_iter()
            .map(|note| NoteInfo {
                title: Path::new(&note.path)
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_owned(),
                path: note.path,
                modified: note.modified.0,
                size: note.size,
            })
            .collect())
    }

    pub async fn list_note_paths(
        &self,
        folder: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, ObsidianError> {
        let folder = folder.map(str::to_owned);
        self.blocking(move |vault, cancelled| {
            vault.list_note_paths_blocking(folder.as_deref(), limit, cancelled)
        })
        .await
    }

    fn list_note_paths_blocking(
        &self,
        folder: Option<&str>,
        limit: usize,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Vec<String>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let mut paths = Vec::new();

        for entry in self
            .walk_markdown_files(&base, cancelled, TraversalOrder::Lexical)
            .take(limit)
        {
            paths.push(self.relative_string(entry?.path())?);
        }
        Ok(paths)
    }

    #[cfg(test)]
    pub async fn read_note(&self, path: &str) -> Result<String, ObsidianError> {
        let path = self.resolve_note_path(path, true)?;
        fs::read_to_string(&path)
            .await
            .map_err(|source| ObsidianError::ReadNote { path, source })
    }

    pub async fn read_note_range(
        &self,
        path: &str,
        offset: Option<usize>,
        limit: Option<usize>,
    ) -> Result<(String, usize, usize), ObsidianError> {
        let path = path.to_owned();
        self.blocking(move |vault, cancelled| {
            let full_path = vault.resolve_note_path(&path, true)?;
            let start = offset.unwrap_or(1).max(1);
            let count = limit.unwrap_or(usize::MAX);
            let mut selected = String::new();
            let mut lines_read = 0;
            let total_lines = scan_note_lines(&full_path, cancelled, |number, line| {
                if number >= start && lines_read < count {
                    if lines_read > 0 {
                        selected.push('\n');
                    }
                    selected.push_str(line);
                    lines_read += 1;
                }
                true
            })?;
            Ok((selected, total_lines, lines_read))
        })
        .await
    }

    pub async fn get_note_outline(
        &self,
        path: &str,
    ) -> Result<(Vec<HeadingItem>, usize), ObsidianError> {
        let path = path.to_owned();
        self.blocking(move |vault, cancelled| {
            let full_path = vault.resolve_note_path(&path, true)?;
            let mut headings = Vec::new();
            let mut in_code_block = false;
            let total_lines = scan_note_lines(&full_path, cancelled, |number, line| {
                let trimmed = line.trim();
                if trimmed.starts_with("```") {
                    in_code_block = !in_code_block;
                    return true;
                }
                if in_code_block {
                    return true;
                }
                let trimmed_start = line.trim_start();
                let hashes = trimmed_start.bytes().take_while(|&c| c == b'#').count();
                if (1..=6).contains(&hashes) {
                    let rest = &trimmed_start[hashes..];
                    if rest.starts_with(' ') || rest.is_empty() {
                        headings.push(HeadingItem {
                            level: hashes as u8,
                            text: rest.trim().to_string(),
                            line: number,
                        });
                    }
                }
                true
            })?;
            Ok((headings, total_lines))
        })
        .await
    }

    /// Ordered by path, then line. Only the requested page is retained.
    pub async fn search_notes(
        &self,
        folder: Option<&str>,
        query: &str,
        limit: usize,
        path_only: bool,
        offset: usize,
    ) -> Result<Vec<SearchMatch>, ObsidianError> {
        let folder = folder.map(str::to_owned);
        let query = query.to_owned();
        self.blocking(move |vault, cancelled| {
            vault.search_notes_blocking(
                folder.as_deref(),
                &query,
                limit,
                path_only,
                offset,
                cancelled,
            )
        })
        .await
    }

    fn search_notes_blocking(
        &self,
        folder: Option<&str>,
        query: &str,
        limit: usize,
        path_only: bool,
        offset: usize,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Vec<SearchMatch>, ObsidianError> {
        let query = query.trim();
        if query.is_empty() {
            return Err(ObsidianError::EmptyQuery);
        }
        let base = self.resolve_folder_path(folder)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let needle = query.to_lowercase();
        let mut matches = Vec::new();
        let mut skipped = 0;

        for entry in self.walk_markdown_files(&base, cancelled, TraversalOrder::Components) {
            let entry = entry?;
            let rel = self.relative_string(entry.path())?;
            if path_only {
                if contains_ignore_case(&rel, &needle) {
                    if skipped < offset {
                        skipped += 1;
                        continue;
                    }
                    matches.push(SearchMatch {
                        path: rel,
                        line: 1,
                        excerpt: String::new(),
                    });
                }
            } else {
                scan_note_lines(entry.path(), cancelled, |line_number, line| {
                    if contains_ignore_case(line, &needle) {
                        if skipped < offset {
                            skipped += 1;
                            return true;
                        }
                        matches.push(SearchMatch {
                            path: rel.clone(),
                            line: line_number,
                            excerpt: compact_excerpt(line.trim()),
                        });
                    }
                    matches.len() < limit
                })?;
            }
            if matches.len() >= limit {
                break;
            }
        }
        Ok(matches)
    }

    pub async fn create_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, false)?;
        if fs::try_exists(&path).await? {
            return Err(ObsidianError::NoteAlreadyExists(
                self.relative_string(&path)?,
            ));
        }

        self.atomic_write(&path, content, false).await?;
        Ok(())
    }

    pub async fn append_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, true)?;
        let mut file = fs::OpenOptions::new().append(true).open(path).await?;
        file.write_all(content.as_bytes()).await?;
        file.sync_all().await?;
        Ok(())
    }

    pub async fn update_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, true)?;
        self.atomic_write(&path, content, true).await?;
        Ok(())
    }

    pub async fn patch_note(
        &self,
        path: &str,
        old_text: &str,
        new_text: &str,
    ) -> Result<(), ObsidianError> {
        if old_text.is_empty() {
            return Err(ObsidianError::EmptyPatchText);
        }

        let full_path = self.resolve_note_path(path, true)?;
        let content =
            fs::read_to_string(&full_path)
                .await
                .map_err(|source| ObsidianError::ReadNote {
                    path: full_path.clone(),
                    source,
                })?;

        let matches = content.matches(old_text).count();
        if matches == 0 {
            return Err(ObsidianError::TextNotFoundInNote {
                path: path.to_string(),
            });
        }
        if matches > 1 {
            return Err(ObsidianError::AmbiguousPatchText {
                path: path.to_string(),
                occurrences: matches,
            });
        }

        let patched_content = content.replacen(old_text, new_text, 1);
        self.atomic_write(&full_path, &patched_content, true)
            .await?;
        Ok(())
    }

    pub async fn get_backlinks(
        &self,
        target_path_or_title: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<BacklinkMatch>, ObsidianError> {
        let target = target_path_or_title.to_owned();
        self.blocking(move |vault, cancelled| {
            vault.get_backlinks_blocking(&target, limit, offset, cancelled)
        })
        .await
    }

    fn get_backlinks_blocking(
        &self,
        target_path_or_title: &str,
        limit: usize,
        offset: usize,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Vec<BacklinkMatch>, ObsidianError> {
        let target = target_path_or_title.trim();
        if target.is_empty() {
            return Err(ObsidianError::EmptyPath);
        }

        let target_path = Path::new(target);
        let stem = target_path
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or(target)
            .to_lowercase();
        let target_norm = target
            .strip_suffix(".md")
            .unwrap_or(target)
            .trim_matches('/')
            .to_lowercase();

        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut backlinks = Vec::new();
        let mut skipped = 0;
        for entry in self.walk_markdown_files(&self.root, cancelled, TraversalOrder::Components) {
            let entry = entry?;
            let rel = self.relative_string(entry.path())?;
            let rel_norm = rel.strip_suffix(".md").unwrap_or(&rel).to_lowercase();
            let rel_stem = Path::new(&rel)
                .file_stem()
                .and_then(OsStr::to_str)
                .unwrap_or("")
                .to_lowercase();
            if rel_stem == stem || rel_norm == target_norm {
                continue;
            }
            scan_note_lines(entry.path(), cancelled, |line_number, line| {
                if line.contains("[[") && line_contains_wikilink_to(line, &stem, &target_norm) {
                    if skipped < offset {
                        skipped += 1;
                        return true;
                    }
                    backlinks.push(BacklinkMatch {
                        source_path: rel.clone(),
                        line: line_number,
                        excerpt: compact_excerpt(line.trim()),
                    });
                }
                backlinks.len() < limit
            })?;
            if backlinks.len() >= limit {
                return Ok(backlinks);
            }
        }
        Ok(backlinks)
    }

    pub async fn list_tags(&self) -> Result<Vec<TagInfo>, ObsidianError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(cancelled.clone());
        let mut entries =
            self.walk_markdown_files(&self.root, &cancelled, TraversalOrder::Unordered);
        let mut pending = Vec::new().into_iter();
        let mut join_set = tokio::task::JoinSet::new();
        let mut discovery_done = false;
        let mut tag_counts: HashMap<String, usize> = HashMap::new();
        while !discovery_done || pending.len() > 0 || !join_set.is_empty() {
            if pending.len() == 0 && !discovery_done {
                // A finite batch never waits on another blocking task. A producer
                // blocked on a full channel can starve a single-thread pool.
                let (remaining, paths) = tokio::task::spawn_blocking(move || {
                    let paths = entries
                        .by_ref()
                        .take(32)
                        .map(|entry| entry.map(walkdir::DirEntry::into_path))
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok::<_, ObsidianError>((entries, paths))
                })
                .await
                .map_err(|error| ObsidianError::Io(std::io::Error::other(error)))??;
                entries = remaining;
                discovery_done = paths.len() < 32;
                pending = paths.into_iter();
            }
            while join_set.len() < 32 {
                let Some(path) = pending.next() else { break };
                let cancelled = cancelled.clone();
                join_set.spawn_blocking(move || {
                    let mut parser = TagParser::default();
                    scan_note_lines(&path, &cancelled, |_, line| {
                        parser.push_line(line);
                        true
                    })?;
                    Ok::<_, ObsidianError>(parser.tags)
                });
            }
            if let Some(result) = join_set.join_next().await {
                let tags =
                    result.map_err(|error| ObsidianError::Io(std::io::Error::other(error)))??;
                for tag in tags {
                    *tag_counts.entry(tag).or_insert(0) += 1;
                }
            }
        }

        let mut result: Vec<TagInfo> = tag_counts
            .into_iter()
            .map(|(tag, count)| TagInfo { tag, count })
            .collect();

        result.sort_by(|a, b| b.count.cmp(&a.count).then(a.tag.cmp(&b.tag)));
        Ok(result)
    }

    async fn atomic_write(
        &self,
        path: &Path,
        content: &str,
        overwrite: bool,
    ) -> Result<(), ObsidianError> {
        let parent = path.parent().ok_or_else(|| {
            ObsidianError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "path has no parent directory",
            ))
        })?;

        fs::create_dir_all(parent).await?;

        let file_name = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or("note.md");
        let random_suffix = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_name = format!(".{file_name}.tmp-{}-{random_suffix}", std::process::id());
        let temp_path = parent.join(temp_name);

        let write_res = async {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp_path)
                .await?;
            file.write_all(content.as_bytes()).await?;
            file.sync_all().await?;
            Ok::<(), std::io::Error>(())
        }
        .await;

        if let Err(e) = write_res {
            let _ = fs::remove_file(&temp_path).await;
            return Err(ObsidianError::Io(e));
        }

        if overwrite {
            if let Err(error) = fs::rename(&temp_path, path).await {
                let _ = fs::remove_file(&temp_path).await;
                return Err(ObsidianError::Io(error));
            }
        } else {
            // Publishing a hard link is atomic and fails if the destination exists.
            // An existence check followed by rename could overwrite another creator.
            let result = fs::hard_link(&temp_path, path).await;
            let _ = fs::remove_file(&temp_path).await;
            if let Err(error) = result {
                if error.kind() == std::io::ErrorKind::AlreadyExists {
                    return Err(ObsidianError::NoteAlreadyExists(
                        self.relative_string(path)?,
                    ));
                }
                return Err(ObsidianError::Io(error));
            }
        }

        Ok(())
    }

    fn resolve_folder_path(&self, folder: Option<&str>) -> Result<PathBuf, ObsidianError> {
        let Some(folder) = folder else {
            return Ok(self.root.clone());
        };

        let relative = normalize_relative_path(folder, false)?;
        let path = self.checked_join(&relative)?;

        if !path.is_dir() {
            return Err(ObsidianError::FolderNotFound(folder.to_string()));
        }

        Ok(path)
    }

    fn resolve_note_path(&self, path: &str, must_exist: bool) -> Result<PathBuf, ObsidianError> {
        let relative = normalize_relative_path(path, true)?;
        let full_path = self.checked_join(&relative)?;

        if must_exist && !full_path.exists() {
            return Err(ObsidianError::NoteNotFound(path.to_string()));
        }

        if must_exist && !full_path.is_file() {
            return Err(ObsidianError::NotAFile(full_path));
        }

        Ok(full_path)
    }

    fn checked_join(&self, relative: &Path) -> Result<PathBuf, ObsidianError> {
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
                // Existence is validated by the caller, which knows whether this is
                // a folder or note and can return a useful resource-not-found error.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(ObsidianError::Io(error)),
            }
        }

        Ok(current)
    }

    // Batch filesystem work off the async workers, including directory traversal.
    async fn blocking<T, F>(&self, work: F) -> Result<T, ObsidianError>
    where
        T: Send + 'static,
        F: FnOnce(Self, &Arc<AtomicBool>) -> Result<T, ObsidianError> + Send + 'static,
    {
        let vault = self.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let _guard = CancelOnDrop(cancelled.clone());
        tokio::task::spawn_blocking(move || work(vault, &cancelled))
            .await
            .map_err(|error| ObsidianError::Io(std::io::Error::other(error)))?
    }

    fn walk_markdown_files(
        &self,
        base: &Path,
        cancelled: &Arc<AtomicBool>,
        order: TraversalOrder,
    ) -> impl Iterator<Item = Result<walkdir::DirEntry, ObsidianError>> + Send + 'static + use<>
    {
        let cancelled = cancelled.clone();
        let walker = WalkDir::new(base).follow_links(false);
        let walker = match order {
            TraversalOrder::Unordered => walker,
            _ => walker.sort_by(move |left, right| match order {
                TraversalOrder::Lexical => {
                    // Include '/' in directory sort keys. Otherwise DFS puts
                    // a/child.md before a.md, unlike the compact String ordering.
                    left.file_name()
                        .as_encoded_bytes()
                        .iter()
                        .copied()
                        .chain(left.file_type().is_dir().then_some(b'/'))
                        .cmp(
                            right
                                .file_name()
                                .as_encoded_bytes()
                                .iter()
                                .copied()
                                .chain(right.file_type().is_dir().then_some(b'/')),
                        )
                }
                _ => left.file_name().cmp(right.file_name()),
            }),
        };
        let mut entries = walker
            .into_iter()
            .filter_entry(|entry| !is_ignored_dir(entry.path()));
        let mut finished = false;
        std::iter::from_fn(move || {
            if finished {
                return None;
            }
            let result = (|| loop {
                // Check before advancing: advancing may read/sort a directory.
                check_cancelled(&cancelled)?;
                let Some(entry) = entries.next() else {
                    return Ok(None);
                };
                let entry =
                    entry.map_err(|error| ObsidianError::Io(std::io::Error::other(error)))?;
                if entry.file_type().is_symlink() {
                    return Err(ObsidianError::SymlinkNotAllowed(entry.path().to_path_buf()));
                }
                if entry.file_type().is_file() && is_markdown(entry.path()) {
                    return Ok(Some(entry));
                }
            })();
            match result {
                Ok(Some(entry)) => Some(Ok(entry)),
                Ok(None) => {
                    finished = true;
                    None
                }
                Err(error) => {
                    finished = true;
                    Some(Err(error))
                }
            }
        })
    }

    fn relative_string(&self, path: &Path) -> Result<String, ObsidianError> {
        let relative = path
            .strip_prefix(&self.root)
            .map_err(|_| ObsidianError::PathTraversal(path_to_string(path)))?;
        Ok(path_to_string(relative))
    }
}

struct CancelOnDrop(Arc<AtomicBool>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), ObsidianError> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(ObsidianError::Io(std::io::Error::new(
            std::io::ErrorKind::Interrupted,
            "vault scan cancelled",
        )));
    }
    Ok(())
}

// Reuse one UTF-8 line buffer. Returning false stops before the rest of the note.
fn scan_note_lines(
    path: &Path,
    cancelled: &AtomicBool,
    mut visit: impl FnMut(usize, &str) -> bool,
) -> Result<usize, ObsidianError> {
    let read_error = |source| ObsidianError::ReadNote {
        path: path.to_path_buf(),
        source,
    };
    check_cancelled(cancelled)?;
    let file = std::fs::File::open(path).map_err(read_error)?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut number = 0;
    loop {
        check_cancelled(cancelled)?;
        line.clear();
        if reader.read_line(&mut line).map_err(read_error)? == 0 {
            return Ok(number);
        }
        if line.ends_with('\n') {
            line.pop();
            if line.ends_with('\r') {
                line.pop();
            }
        }
        number += 1;
        if !visit(number, &line) {
            return Ok(number);
        }
    }
}

/// Bound returned snippets without splitting UTF-8; full text is available via read_note.
fn compact_excerpt(text: &str) -> String {
    let mut chars = text.chars();
    let mut result: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

fn contains_ignore_case(haystack: &str, needle_lower: &str) -> bool {
    if needle_lower.is_empty() {
        return true;
    }
    if haystack.is_ascii() && needle_lower.is_ascii() {
        let n = needle_lower.as_bytes();
        haystack
            .as_bytes()
            .windows(n.len())
            .any(|w| w.eq_ignore_ascii_case(n))
    } else {
        haystack.to_lowercase().contains(needle_lower)
    }
}

fn line_contains_wikilink_to(line: &str, target_stem_lower: &str, target_norm_lower: &str) -> bool {
    let mut rest = line;
    while let Some(start_idx) = rest.find("[[") {
        let after_start = &rest[start_idx + 2..];
        if let Some(end_idx) = after_start.find("]]") {
            let inner = &after_start[..end_idx];
            let link_target = inner.split(['|', '#']).next().unwrap_or("").trim();
            let link_target_clean = link_target
                .strip_suffix(".md")
                .unwrap_or(link_target)
                .trim_matches('/')
                .to_lowercase();
            let link_stem = Path::new(&link_target_clean)
                .file_stem()
                .and_then(OsStr::to_str)
                .unwrap_or(&link_target_clean);

            if link_stem == target_stem_lower || link_target_clean == target_norm_lower {
                return true;
            }
            rest = &after_start[end_idx + 2..];
        } else {
            break;
        }
    }
    false
}

// State spans lines, not note bodies. Retain only distinct tags per note so a
// repeated tag counts once without retaining every occurrence before deduplication.
#[derive(Default)]
struct TagParser {
    tags: HashSet<String>,
    started: bool,
    in_frontmatter: bool,
    in_tags_list: bool,
    in_code_block: bool,
}

impl TagParser {
    fn insert(&mut self, candidate: &str) {
        if is_valid_tag_name(candidate) {
            self.tags.insert(candidate.to_lowercase());
        }
    }

    fn push_line(&mut self, line: &str) {
        let trimmed = line.trim();
        if !self.started {
            self.started = true;
            if trimmed == "---" {
                self.in_frontmatter = true;
                return;
            }
        }
        if self.in_frontmatter {
            if trimmed == "---" {
                self.in_frontmatter = false;
                return;
            }
            if trimmed.starts_with("tags:") || trimmed.starts_with("tag:") {
                let rest = trimmed.split_once(':').map(|(_, r)| r.trim()).unwrap_or("");
                if rest.starts_with('[') && rest.ends_with(']') {
                    for item in rest[1..rest.len() - 1].split(',') {
                        self.insert(item.trim().trim_matches(['\'', '"', '#']));
                    }
                } else if !rest.is_empty() {
                    self.insert(rest.trim_matches(['\'', '"', '#']));
                } else {
                    self.in_tags_list = true;
                }
            } else if self.in_tags_list {
                if trimmed.starts_with('-') {
                    self.insert(
                        trimmed
                            .trim_start_matches('-')
                            .trim()
                            .trim_matches(['\'', '"', '#']),
                    );
                } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    self.in_tags_list = false;
                }
            }
            return;
        }
        if trimmed.starts_with("```") {
            self.in_code_block = !self.in_code_block;
            return;
        }
        if self.in_code_block {
            return;
        }

        // Find ASCII delimiters by byte; decode Unicode only around actual tags.
        for (index, _) in line.match_indices('#') {
            let prev_ok = line[..index]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || c == '(' || c == '[');
            if !prev_ok {
                continue;
            }
            let rest = &line[index + 1..];
            let end = rest
                .char_indices()
                .find(|&(_, c)| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '/'))
                .map_or(rest.len(), |(index, _)| index);
            self.insert(&rest[..end]);
        }
    }
}

#[cfg(test)]
fn extract_tags_from_content(content: &str) -> Vec<String> {
    let mut parser = TagParser::default();
    for line in content.lines() {
        parser.push_line(line);
    }
    let mut tags: Vec<_> = parser.tags.into_iter().collect();
    tags.sort();
    tags
}

fn is_valid_tag_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars().any(|c| c.is_alphabetic())
        && !s.starts_with('/')
        && !s.ends_with('/')
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
    let mut result = String::new();
    for component in path.components() {
        if let Component::Normal(value) = component
            && let Some(value) = value.to_str()
        {
            if !result.is_empty() {
                result.push('/');
            }
            result.push_str(value);
        }
    }
    result
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
    async fn patch_note_replaces_unique_text() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("demo.md", "hello world\nline 2")
            .await
            .expect("create");

        vault
            .patch_note("demo.md", "world", "rust")
            .await
            .expect("patch");

        assert_eq!(
            vault.read_note("demo.md").await.expect("read"),
            "hello rust\nline 2"
        );

        // Text not found
        let err = vault
            .patch_note("demo.md", "nonexistent", "x")
            .await
            .expect_err("should fail");
        assert!(matches!(err, ObsidianError::TextNotFoundInNote { .. }));

        // Ambiguous text
        vault
            .append_note("demo.md", "\nhello again")
            .await
            .expect("append");
        let err = vault
            .patch_note("demo.md", "hello", "hi")
            .await
            .expect_err("should fail ambiguous");
        assert!(matches!(err, ObsidianError::AmbiguousPatchText { .. }));
    }

    #[tokio::test]
    async fn read_note_range_slices_lines() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("lines.md", "one\ntwo\nthree\nfour\nfive")
            .await
            .expect("create");

        let (text, total, read) = vault
            .read_note_range("lines.md", Some(2), Some(2))
            .await
            .expect("range");
        assert_eq!(text, "two\nthree");
        assert_eq!(total, 5);
        assert_eq!(read, 2);

        let (text, total, read) = vault
            .read_note_range("lines.md", Some(10), Some(2))
            .await
            .expect("range oob");
        assert_eq!(text, "");
        assert_eq!(total, 5);
        assert_eq!(read, 0);
    }

    #[tokio::test]
    async fn get_note_outline_extracts_headings() {
        let (_temp, vault) = temp_vault("The Core");

        let content =
            "# Title\nSome text\n## Section 1\n```\n# Not a heading\n```\n### Subsection\n#tag";
        vault.create_note("doc.md", content).await.expect("create");

        let (headings, total_lines) = vault.get_note_outline("doc.md").await.expect("outline");
        assert_eq!(total_lines, 8);
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0].level, 1);
        assert_eq!(headings[0].text, "Title");
        assert_eq!(headings[0].line, 1);
        assert_eq!(headings[1].level, 2);
        assert_eq!(headings[1].text, "Section 1");
        assert_eq!(headings[1].line, 3);
        assert_eq!(headings[2].level, 3);
        assert_eq!(headings[2].text, "Subsection");
        assert_eq!(headings[2].line, 7);
    }

    #[tokio::test]
    async fn search_stops_early_and_reports_read_failures_when_reached() {
        let (temp, vault) = temp_vault("Test");
        vault.create_note("a.md", "needle").await.unwrap();
        std::fs::write(temp.path().join("z.md"), [0xff, 0xfe]).unwrap();
        let first = vault
            .search_notes(None, "needle", 1, false, 0)
            .await
            .unwrap();
        assert_eq!(first[0].path, "a.md");
        let error = vault
            .search_notes(None, "needle", 2, false, 0)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            crate::error::ObsidianError::ReadNote { .. }
        ));
        // Path-only discovery must not read or decode note bodies.
        let paths = vault.search_notes(None, ".md", 10, true, 0).await.unwrap();
        assert_eq!(paths.len(), 2);
    }

    #[tokio::test]
    async fn tag_scan_replenishes_bounded_workers() {
        let (_temp, vault) = temp_vault("Test");
        for i in 0..70 {
            vault
                .create_note(&format!("{i:03}.md"), "#shared")
                .await
                .unwrap();
        }
        let tags = vault.list_tags().await.unwrap();
        assert_eq!(tags.len(), 1);
        assert_eq!(tags[0].tag, "shared");
        assert_eq!(tags[0].count, 70);
    }

    #[tokio::test]
    async fn concurrent_create_never_overwrites_the_winner() {
        let (_temp, vault) = temp_vault("Test");
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..32 {
            let vault = vault.clone();
            tasks.spawn(async move {
                let content = format!("writer {i}");
                let result = vault.create_note("same.md", &content).await;
                (content, result)
            });
        }
        let mut winners = Vec::new();
        while let Some(result) = tasks.join_next().await {
            let (content, result) = result.expect("task");
            match result {
                Ok(()) => winners.push(content),
                Err(crate::error::ObsidianError::NoteAlreadyExists(_)) => {}
                Err(error) => panic!("unexpected create error: {error}"),
            }
        }
        assert_eq!(winners.len(), 1);
        assert_eq!(vault.read_note("same.md").await.unwrap(), winners[0]);
    }

    #[tokio::test]
    async fn search_notes_returns_matches() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("Projects/demo.md", "line one\nPending auth\nline three")
            .await
            .expect("create note");

        let matches = vault
            .search_notes(Some("Projects"), "pending", 10, false, 0)
            .await
            .expect("search notes");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 2);

        let path_matches = vault
            .search_notes(None, "demo", 10, true, 0)
            .await
            .expect("path search");
        assert_eq!(path_matches.len(), 1);
        assert_eq!(path_matches[0].path, "Projects/demo.md");
    }

    #[tokio::test]
    async fn get_backlinks_finds_wikilinks() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("Target.md", "# Target Note")
            .await
            .expect("create target");
        vault
            .create_note(
                "SourceA.md",
                "Referencing [[Target]] here and [[Target#Section|alias]] here.",
            )
            .await
            .expect("create a");
        vault
            .create_note("SourceB.md", "No links here.")
            .await
            .expect("create b");

        let backlinks = vault
            .get_backlinks("Target.md", 10, 0)
            .await
            .expect("backlinks");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].source_path, "SourceA.md");
        assert_eq!(backlinks[0].line, 1);
    }

    #[tokio::test]
    async fn list_tags_finds_frontmatter_and_inline() {
        let (_temp, vault) = temp_vault("The Core");

        let note1 = "---\ntags: [rust, mcp]\n---\n#notes and some inline #dev/backend";
        let note2 = "Simple text with #rust tag.";
        vault.create_note("note1.md", note1).await.expect("note1");
        vault.create_note("note2.md", note2).await.expect("note2");

        let tags = vault.list_tags().await.expect("tags");
        assert!(tags.iter().any(|t| t.tag == "rust" && t.count == 2));
        assert!(tags.iter().any(|t| t.tag == "mcp" && t.count == 1));
        assert!(tags.iter().any(|t| t.tag == "dev/backend" && t.count == 1));
        assert!(tags.iter().any(|t| t.tag == "notes" && t.count == 1));
    }

    #[tokio::test]
    async fn streaming_read_preserves_line_endings_and_reports_invalid_utf8() {
        let (temp, vault) = temp_vault("streaming");
        for content in ["", "\n", "a\r\nb\n\nlast\r", "café\n日本語\n", "\r\n\r\n"] {
            fs::write(temp.path().join("lines.md"), content).unwrap();
            let expected: Vec<_> = content.lines().collect();
            for offset in [0, 1, 2, 99, usize::MAX] {
                for limit in [0, 1, 10] {
                    let actual = vault
                        .read_note_range("lines.md", Some(offset), Some(limit))
                        .await
                        .unwrap();
                    let selected: Vec<_> = expected
                        .iter()
                        .skip(offset.max(1) - 1)
                        .take(limit)
                        .copied()
                        .collect();
                    assert_eq!(
                        actual,
                        (selected.join("\n"), expected.len(), selected.len())
                    );
                }
            }
        }
        fs::write(temp.path().join("lines.md"), b"match\n\xff\n").unwrap();
        let matches = vault
            .search_notes(None, "match", 1, false, 0)
            .await
            .unwrap();
        assert_eq!(matches.len(), 1); // Do not validate unread later lines.
        assert!(matches!(
            vault.read_note_range("lines.md", Some(1), Some(1)).await,
            Err(ObsidianError::ReadNote { .. })
        ));
        assert!(matches!(
            vault.get_note_outline("lines.md").await,
            Err(ObsidianError::ReadNote { .. })
        ));
        assert!(matches!(
            vault.get_backlinks("Absent", 1, 0).await,
            Err(ObsidianError::ReadNote { .. })
        ));
        assert!(matches!(
            vault.list_tags().await,
            Err(ObsidianError::ReadNote { .. })
        ));
    }

    #[test]
    fn streaming_scan_stops_when_cancelled() {
        let (temp, _) = temp_vault("cancel");
        let path = temp.path().join("lines.md");
        fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let cancelled = super::AtomicBool::new(false);
        let mut visited = 0;
        let result = super::scan_note_lines(&path, &cancelled, |_, _| {
            visited += 1;
            cancelled.store(true, super::Ordering::Relaxed);
            true
        });
        assert_eq!(visited, 1);
        assert!(
            matches!(result, Err(ObsidianError::Io(error)) if error.kind() == std::io::ErrorKind::Interrupted)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_scans_yield_to_runtime_and_cancel_on_drop() {
        let (_temp, vault) = temp_vault("cancel");
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
        let scan = tokio::spawn(async move {
            vault
                .blocking(move |_, cancelled| {
                    let _ = started_tx.send(());
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                    while !cancelled.load(super::Ordering::Relaxed)
                        && std::time::Instant::now() < deadline
                    {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    let _ = stopped_tx.send(cancelled.load(super::Ordering::Relaxed));
                    Ok(())
                })
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx)
            .await
            .unwrap()
            .unwrap();
        scan.abort();
        assert!(scan.await.unwrap_err().is_cancelled());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), stopped_rx)
                .await
                .unwrap()
                .unwrap()
        );
    }

    #[test]
    fn tag_parser_preserves_unicode_boundaries_frontmatter_and_code_fences() {
        let tags = super::extract_tags_from_content(
            "---\ntags:\n  - Rust\n  - 'mcp'\n---\n# Title\n#rust (#CAFÉ) [#日本語] x#not-a-tag\n\u{2003}#dev/backend #123 #/bad #bad/ ##no\n```rust\n#ignored\n```\n#tag-name #tag_name\n",
        );
        assert_eq!(
            tags,
            [
                "café",
                "dev/backend",
                "mcp",
                "rust",
                "tag-name",
                "tag_name",
                "日本語"
            ]
        );
        assert_eq!(
            super::extract_tags_from_content("---\ntag: '#Solo'\n---\n#solo"),
            ["solo"]
        );
        assert!(super::extract_tags_from_content("---\n#unclosed-frontmatter").is_empty());
    }

    #[test]
    fn tag_parser_retains_only_distinct_tags_across_many_lines() {
        let mut parser = super::TagParser::default();
        for _ in 0..10_000 {
            parser.push_line("#rust #Rust #dev/backend #日本語");
            assert_eq!(parser.tags.len(), 3);
        }
    }

    #[tokio::test]
    async fn lazy_traversal_preserves_string_and_component_order_across_pages() {
        let (temp, vault) = temp_vault("ordering");
        let paths = [
            "a.md",
            "a/child.md",
            "a/sub/deep.md",
            "a!/child.md",
            "a-.md",
            "a./child.md",
            "a0.md",
            "a_/child.md",
            "a..md",
            "a.z.md",
            "B.md",
            "é.md",
            "é/child.md",
            "日本語.md",
            "日本語/child.md",
            "nested/a.md",
            "nested/a/child.md",
            "nested/a0.md",
        ];
        for path in paths {
            let full = temp.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "match [[Target]]").unwrap();
        }
        let mut lexical: Vec<_> = paths.iter().map(|p| p.to_string()).collect();
        lexical.sort();
        let mut components = lexical.clone();
        components.sort_by(|a, b| std::path::Path::new(a).cmp(std::path::Path::new(b)));
        assert_ne!(lexical, components); // Exercise the subtle a.md / a/ ordering.
        for limit in [0, 1, 5, paths.len(), paths.len() + 1, usize::MAX] {
            assert_eq!(
                vault.list_note_paths(None, limit).await.unwrap(),
                lexical.iter().take(limit).cloned().collect::<Vec<_>>()
            );
            for offset in [0, 1, 5, paths.len(), usize::MAX] {
                let expected: Vec<_> = components
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .cloned()
                    .collect();
                for path_only in [false, true] {
                    let query = if path_only { ".md" } else { "match" };
                    let actual = vault
                        .search_notes(None, query, limit, path_only, offset)
                        .await
                        .unwrap();
                    assert_eq!(
                        actual.into_iter().map(|m| m.path).collect::<Vec<_>>(),
                        expected
                    );
                }
                let actual = vault.get_backlinks("Target", limit, offset).await.unwrap();
                assert_eq!(
                    actual
                        .into_iter()
                        .map(|m| m.source_path)
                        .collect::<Vec<_>>(),
                    expected
                );
            }
        }
        let nested = vault
            .list_note_paths(Some("nested"), usize::MAX)
            .await
            .unwrap();
        assert_eq!(nested, ["nested/a.md", "nested/a/child.md", "nested/a0.md"]);
    }

    #[tokio::test]
    async fn detailed_top_k_matches_exhaustive_sort_with_timestamp_ties() {
        let (temp, vault) = temp_vault("top-k");
        let mut expected = Vec::new();
        for index in 0..150 {
            let path = format!("note-{index:03}.md");
            let full = temp.path().join(&path);
            fs::write(&full, "x".repeat(index + 1)).unwrap();
            let modified = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::new(100 + (index % 11) as u64, (index % 3) as u32);
            fs::File::open(&full)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
            let metadata = fs::metadata(&full).unwrap();
            expected.push(super::NoteInfo {
                title: format!("note-{index:03}"),
                path,
                modified: chrono::DateTime::<chrono::Utc>::from(metadata.modified().unwrap())
                    .to_rfc3339(),
                size: metadata.len(),
            });
        }
        expected.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.path.cmp(&b.path)));
        for limit in [0, 1, 5, 50, 150, usize::MAX] {
            let actual = vault.list_notes(None, limit).await.unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected.iter().take(limit).collect::<Vec<_>>()).unwrap()
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lazy_pages_do_not_visit_later_subtrees_but_reject_symlinks_when_reached() {
        let (temp, vault) = temp_vault("lazy");
        fs::write(temp.path().join("a.md"), "match [[Target]] #tag").unwrap();
        fs::create_dir(temp.path().join("z")).unwrap();
        std::os::unix::fs::symlink(temp.path().join("a.md"), temp.path().join("z/link.md"))
            .unwrap();
        assert_eq!(vault.list_note_paths(None, 1).await.unwrap(), ["a.md"]);
        assert_eq!(
            vault
                .search_notes(None, "match", 1, false, 0)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(vault.get_backlinks("Target", 1, 0).await.unwrap().len(), 1);
        assert!(matches!(
            vault.list_note_paths(None, usize::MAX).await,
            Err(ObsidianError::SymlinkNotAllowed(_))
        ));
        assert!(matches!(
            vault.search_notes(None, "match", 1, false, 1).await,
            Err(ObsidianError::SymlinkNotAllowed(_))
        ));
        assert!(matches!(
            vault.get_backlinks("Target", 1, 1).await,
            Err(ObsidianError::SymlinkNotAllowed(_))
        ));
        assert!(matches!(
            vault.list_notes(None, 1).await,
            Err(ObsidianError::SymlinkNotAllowed(_))
        ));
        assert!(matches!(
            vault.list_tags().await,
            Err(ObsidianError::SymlinkNotAllowed(_))
        ));
    }

    #[tokio::test]
    async fn traversal_observes_external_changes_without_stale_results() {
        let (temp, vault) = temp_vault("fresh");
        fs::write(temp.path().join("old.md"), "oldtext #old").unwrap();
        assert_eq!(vault.list_note_paths(None, 10).await.unwrap(), ["old.md"]);
        assert_eq!(vault.list_tags().await.unwrap()[0].tag, "old");
        fs::rename(temp.path().join("old.md"), temp.path().join("new.md")).unwrap();
        fs::write(temp.path().join("new.md"), "newtext #new").unwrap();
        assert_eq!(vault.list_note_paths(None, 10).await.unwrap(), ["new.md"]);
        assert!(
            vault
                .search_notes(None, "oldtext", 10, false, 0)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            vault
                .search_notes(None, "newtext", 10, false, 0)
                .await
                .unwrap()[0]
                .path,
            "new.md"
        );
        assert_eq!(vault.list_tags().await.unwrap()[0].tag, "new");
        fs::remove_file(temp.path().join("new.md")).unwrap();
        assert!(vault.list_note_paths(None, 10).await.unwrap().is_empty());
        assert!(vault.list_tags().await.unwrap().is_empty());
    }

    #[test]
    fn cancelled_discovery_stops_before_advancing_and_stays_finished() {
        let (temp, vault) = temp_vault("cancelled-discovery");
        let cancelled = std::sync::Arc::new(super::AtomicBool::new(true));
        let mut entries = vault.walk_markdown_files(
            &temp.path().join("missing"),
            &cancelled,
            super::TraversalOrder::Lexical,
        );
        assert!(
            matches!(entries.next(), Some(Err(ObsidianError::Io(error))) if error.kind() == std::io::ErrorKind::Interrupted)
        );
        assert!(entries.next().is_none());
        assert!(entries.next().is_none());
    }

    #[test]
    fn tag_batches_finish_with_one_blocking_thread_and_at_batch_boundaries() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            for count in [0, 1, 31, 32, 33, 63, 64, 65, 100] {
                let (temp, vault) = temp_vault("single-worker");
                for index in 0..count {
                    fs::write(temp.path().join(format!("{index}.md")), "#rust #Rust").unwrap();
                }
                let tags =
                    tokio::time::timeout(std::time::Duration::from_secs(5), vault.list_tags())
                        .await
                        .unwrap()
                        .unwrap();
                if count == 0 {
                    assert!(tags.is_empty());
                } else {
                    assert_eq!(tags.len(), 1);
                    assert_eq!(tags[0].count, count);
                }
            }
        });
    }

    #[test]
    fn vault_registry_get_or_default() {
        let temp = TempDir::new().expect("temp dir");
        fs::create_dir(temp.path().join(".obsidian")).expect("obsidian dir");

        let registry = VaultRegistry::from_specs(vec![VaultSpec {
            name: "Primary".into(),
            path: temp.path().to_path_buf(),
        }])
        .expect("registry");

        // Resolves without name when single vault
        assert_eq!(
            registry.get_or_default(None).expect("default").name(),
            "Primary"
        );
        assert_eq!(
            registry
                .get_or_default(Some("Primary"))
                .expect("named")
                .name(),
            "Primary"
        );
        assert!(registry.get_or_default(Some("NonExistent")).is_err());
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
