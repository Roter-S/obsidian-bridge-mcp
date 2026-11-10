use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{BufRead, BufReader};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, Utc};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use unicode_normalization::char::{decompose_canonical, is_combining_mark};
use walkdir::WalkDir;

use crate::cli::VaultSpec;
use crate::error::ObsidianError;

#[cfg(test)]
#[path = "vault_bench.rs"]
pub(crate) mod benchmarks;

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
    pub modified: String,
    #[schemars(with = "i64")]
    pub size: u64,
}

// A max-heap keeps the worst retained candidate on top: oldest modification
// time at full precision, then greatest path. Responses only print seconds.
#[derive(Eq, PartialEq, Ord, PartialOrd)]
struct NoteCandidate {
    modified: Reverse<DateTime<Utc>>,
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
            .ok_or_else(|| ObsidianError::UnknownVault {
                name: name.to_string(),
                available: self.names(),
            })
    }

    fn names(&self) -> String {
        self.list()
            .into_iter()
            .map(|vault| vault.name)
            .collect::<Vec<_>>()
            .join(", ")
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
                    Err(ObsidianError::AmbiguousVault(self.names()))
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
        tag: Option<&str>,
    ) -> Result<Vec<NoteInfo>, ObsidianError> {
        let folder = folder.map(str::to_owned);
        let tag = tag.map(normalize_tag).transpose()?;
        self.blocking(move |vault, cancelled| {
            vault.list_notes_blocking(folder.as_deref(), limit, tag.as_deref(), cancelled)
        })
        .await
    }

    fn list_notes_blocking(
        &self,
        folder: Option<&str>,
        limit: usize,
        tag: Option<&str>,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Vec<NoteInfo>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let mut notes: BinaryHeap<NoteCandidate> = BinaryHeap::new();

        // Newest-first requires examining every note, but only retain the best
        // `limit` candidates. Do not allocate capacity from an untrusted limit.
        for entry in self.walk_markdown_files(&base, cancelled, TraversalOrder::Unordered) {
            let entry = entry?;
            if let Some(tag) = tag
                && !note_has_tag(entry.path(), tag, cancelled)?
            {
                continue;
            }
            let metadata = entry
                .metadata()
                .map_err(|error| ObsidianError::Io(std::io::Error::other(error)))?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let candidate = NoteCandidate {
                modified: Reverse(DateTime::<Utc>::from(modified)),
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
                path: note.path,
                modified: note.modified.0.to_rfc3339_opts(SecondsFormat::Secs, true),
                size: note.size,
            })
            .collect())
    }

    pub async fn list_note_paths(
        &self,
        folder: Option<&str>,
        limit: usize,
        tag: Option<&str>,
    ) -> Result<Vec<String>, ObsidianError> {
        let folder = folder.map(str::to_owned);
        let tag = tag.map(normalize_tag).transpose()?;
        self.blocking(move |vault, cancelled| {
            vault.list_note_paths_blocking(folder.as_deref(), limit, tag.as_deref(), cancelled)
        })
        .await
    }

    fn list_note_paths_blocking(
        &self,
        folder: Option<&str>,
        limit: usize,
        tag: Option<&str>,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Vec<String>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let mut paths = Vec::new();
        let mut entries = self.walk_markdown_files(&base, cancelled, TraversalOrder::Lexical);

        // Stop before advancing past a full page: later subtrees stay unvisited.
        while paths.len() < limit {
            let Some(entry) = entries.next() else { break };
            let entry = entry?;
            if let Some(tag) = tag
                && !note_has_tag(entry.path(), tag, cancelled)?
            {
                continue;
            }
            paths.push(self.relative_string(entry.path())?);
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
        let needle = fold_for_search(query);
        let mut matches = Vec::new();
        let mut skipped = 0;
        let mut scratch = String::new();

        for entry in self.walk_markdown_files(&base, cancelled, TraversalOrder::Components) {
            let entry = entry?;
            let rel = self.relative_string(entry.path())?;
            if path_only {
                if contains_folded(&rel, &needle, &mut scratch) {
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
                    if contains_folded(line, &needle, &mut scratch) {
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

    /// Appended content starts on its own line: a note that does not end in a
    /// newline gets one first, unless the content already begins with one.
    pub async fn append_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};

        let path = self.resolve_note_path(path, true)?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .append(true)
            .open(path)
            .await?;
        let mut separator = "";
        if !content.starts_with(['\n', '\r']) && file.metadata().await?.len() > 0 {
            let mut last = [0u8; 1];
            file.seek(std::io::SeekFrom::End(-1)).await?;
            file.read_exact(&mut last).await?;
            if last[0] != b'\n' {
                separator = "\n";
            }
        }
        // One write keeps the separator and content together in append mode.
        file.write_all([separator, content].concat().as_bytes())
            .await?;
        file.sync_all().await?;
        Ok(())
    }

    /// Returns the line counts before and after the replacement. A stale
    /// `expected_total_lines` rejects the write before anything changes.
    pub async fn update_note(
        &self,
        path: &str,
        content: &str,
        expected_total_lines: Option<usize>,
    ) -> Result<(usize, usize), ObsidianError> {
        let full_path = self.resolve_note_path(path, true)?;
        let previous = fs::read(&full_path)
            .await
            .map_err(|source| ObsidianError::ReadNote {
                path: full_path.clone(),
                source,
            })?;
        let previous_lines = count_lines(&previous);
        if let Some(expected) = expected_total_lines
            && expected != previous_lines
        {
            return Err(ObsidianError::LineCountMismatch {
                path: path.to_string(),
                expected,
                actual: previous_lines,
            });
        }
        self.atomic_write(&full_path, content, true).await?;
        Ok((previous_lines, count_lines(content.as_bytes())))
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

        // read_note reports lines joined by "\n". A note stored with CRLF line
        // endings must still accept a snippet copied from that output.
        let (mut old_text, mut new_text) = (old_text.to_owned(), new_text.to_owned());
        if !content.contains(&old_text) && content.contains("\r\n") && !old_text.contains('\r') {
            old_text = old_text.replace('\n', "\r\n");
            new_text = new_text.replace("\r\n", "\n").replace('\n', "\r\n");
        }

        let starts: Vec<_> = content
            .match_indices(&old_text)
            .map(|(index, _)| index)
            .collect();
        match starts.as_slice() {
            [] => Err(ObsidianError::TextNotFoundInNote {
                path: path.to_string(),
            }),
            [_] => {
                let patched_content = content.replacen(&old_text, &new_text, 1);
                self.atomic_write(&full_path, &patched_content, true).await
            }
            _ => Err(ObsidianError::AmbiguousPatchText {
                path: path.to_string(),
                occurrences: starts.len(),
                lines: starts
                    .iter()
                    .take(5)
                    .map(|&index| (content[..index].matches('\n').count() + 1).to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
            }),
        }
    }

    /// Moves a note into the vault's `.trash/` folder, as Obsidian does, and
    /// returns its new vault-relative path. Never overwrites a trashed note.
    pub async fn delete_note(&self, path: &str) -> Result<String, ObsidianError> {
        let source = self.resolve_note_path(path, true)?;
        let trash = self.checked_join(Path::new(TRASH_DIR))?;
        if source.starts_with(&trash) {
            return Err(ObsidianError::AlreadyInTrash(path.to_string()));
        }
        fs::create_dir_all(&trash).await?;
        let stem = source
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("note")
            .to_owned();
        for attempt in 1..=10_000 {
            let name = if attempt == 1 {
                format!("{stem}.md")
            } else {
                format!("{stem} {attempt}.md")
            };
            let destination = trash.join(name);
            match self.move_no_replace(&source, &destination).await {
                Ok(()) => return self.relative_string(&destination),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(ObsidianError::Io(error)),
            }
        }
        Err(ObsidianError::NoteAlreadyExists(format!(
            "{TRASH_DIR}/{stem}.md"
        )))
    }

    /// Moves or renames a note. Fails if the destination exists. Links in
    /// other notes are not rewritten.
    pub async fn move_note(&self, path: &str, new_path: &str) -> Result<(), ObsidianError> {
        let source = self.resolve_note_path(path, true)?;
        let destination = self.resolve_note_path(new_path, false)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).await?;
        }
        match self.move_no_replace(&source, &destination).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(ObsidianError::NoteAlreadyExists(new_path.to_string()))
            }
            Err(error) => Err(ObsidianError::Io(error)),
        }
    }

    // Publishing a hard link is atomic and fails if the destination exists; a
    // plain rename would silently replace it. The note keeps its timestamps.
    async fn move_no_replace(&self, source: &Path, destination: &Path) -> std::io::Result<()> {
        fs::hard_link(source, destination).await?;
        if let Err(error) = fs::remove_file(source).await {
            let _ = fs::remove_file(destination).await;
            return Err(error);
        }
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
        // A bare title can name any note with that basename; an explicit path
        // names one note, so same-named notes elsewhere are still sources.
        let target_is_path = target.ends_with(".md") || target.contains('/');
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
            if rel_norm == target_norm || (!target_is_path && rel_stem == stem) {
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
            if overwrite && let Ok(original) = fs::metadata(path).await {
                // The replacement is a new file. Carry over what Obsidian and
                // the user can observe: permissions and the creation date.
                file.set_permissions(original.permissions()).await?;
                let file = file.into_std().await;
                keep_creation_time(&file, &original);
                file.sync_all()?;
            } else {
                file.sync_all().await?;
            }
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
            .filter_entry(|entry| !is_hidden_dir(entry));
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

/// Case- and accent-insensitive search key: decompose, drop combining marks,
/// lowercase. Precomposed and decomposed spellings (macOS file names) agree.
fn fold_for_search(text: &str) -> String {
    let mut folded = String::new();
    fold_into(text, &mut folded);
    folded
}

// Dropping every combining mark makes their order irrelevant, so characters
// are decomposed one at a time instead of through a reordering NFD iterator.
fn fold_into(text: &str, folded: &mut String) {
    folded.clear();
    for c in text.chars() {
        if c.is_ascii() {
            folded.push(c.to_ascii_lowercase());
        } else {
            decompose_canonical(c, |part| {
                if !is_combining_mark(part) {
                    folded.extend(part.to_lowercase());
                }
            });
        }
    }
}

// `scratch` is reused across lines so a scan does not allocate per line.
fn contains_folded(haystack: &str, needle_folded: &str, scratch: &mut String) -> bool {
    if needle_folded.is_empty() {
        return true;
    }
    if !haystack.is_ascii() {
        fold_into(haystack, scratch);
        return scratch.contains(needle_folded);
    }
    // Folding ASCII only changes case, so compare bytes without allocating.
    let n = needle_folded.as_bytes();
    needle_folded.is_ascii()
        && haystack
            .as_bytes()
            .windows(n.len())
            .any(|w| w.eq_ignore_ascii_case(n))
}

fn count_lines(bytes: &[u8]) -> usize {
    let newlines = bytes.iter().filter(|&&byte| byte == b'\n').count();
    newlines + usize::from(bytes.last().is_some_and(|&byte| byte != b'\n'))
}

// Only macOS and Windows let a process set a file's creation time.
#[allow(unused_variables)]
fn keep_creation_time(file: &std::fs::File, original: &std::fs::Metadata) {
    #[cfg(any(target_vendor = "apple", windows))]
    {
        #[cfg(target_vendor = "apple")]
        use std::os::darwin::fs::FileTimesExt;
        #[cfg(windows)]
        use std::os::windows::fs::FileTimesExt;

        // Best effort: a filesystem without creation times must not fail a write.
        if let Ok(created) = original.created() {
            let _ = file.set_times(std::fs::FileTimes::new().set_created(created));
        }
    }
}

fn normalize_tag(tag: &str) -> Result<String, ObsidianError> {
    let tag = tag.trim().trim_start_matches('#').trim_matches('/');
    if tag.is_empty() {
        return Err(ObsidianError::EmptyTag);
    }
    Ok(tag.to_lowercase())
}

// Matches the tag itself and nested tags below it, as Obsidian does. Stops
// reading the note at the first matching tag.
fn note_has_tag(path: &Path, tag: &str, cancelled: &AtomicBool) -> Result<bool, ObsidianError> {
    let mut parser = TagParser::default();
    let mut seen = 0;
    let mut found = false;
    scan_note_lines(path, cancelled, |_, line| {
        parser.push_line(line);
        if parser.tags.len() != seen {
            seen = parser.tags.len();
            found = parser.tags.iter().any(|candidate| {
                candidate
                    .strip_prefix(tag)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            });
        }
        !found
    })?;
    Ok(found)
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
            let before = &line[..index];
            let prev_ok = before
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || c == '(' || c == '[');
            // `[text](#heading)` is a link to a heading, not a tag.
            if !prev_ok || before.ends_with("](") {
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

const TRASH_DIR: &str = ".trash";

// Obsidian hides dot-folders: `.obsidian`, `.trash` (deleted notes), `.git`.
// The folder a caller names explicitly (depth 0) is still traversed.
fn is_hidden_dir(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && entry.file_name().as_encoded_bytes().starts_with(b".")
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
            .update_note("Projects/demo.md", "# Updated", None)
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
                vault.list_note_paths(None, limit, None).await.unwrap(),
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
            .list_note_paths(Some("nested"), usize::MAX, None)
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
            let time = chrono::DateTime::<chrono::Utc>::from(metadata.modified().unwrap());
            let info = super::NoteInfo {
                path,
                modified: time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                size: metadata.len(),
            };
            expected.push((time, info));
        }
        // Ties within a printed second are still ordered by the full timestamp.
        expected.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.path.cmp(&b.1.path)));
        let expected: Vec<_> = expected.into_iter().map(|(_, info)| info).collect();
        for limit in [0, 1, 5, 50, 150, usize::MAX] {
            let actual = vault.list_notes(None, limit, None).await.unwrap();
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
        assert_eq!(
            vault.list_note_paths(None, 1, None).await.unwrap(),
            ["a.md"]
        );
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
            vault.list_note_paths(None, usize::MAX, None).await,
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
            vault.list_notes(None, 1, None).await,
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
        assert_eq!(
            vault.list_note_paths(None, 10, None).await.unwrap(),
            ["old.md"]
        );
        assert_eq!(vault.list_tags().await.unwrap()[0].tag, "old");
        fs::rename(temp.path().join("old.md"), temp.path().join("new.md")).unwrap();
        fs::write(temp.path().join("new.md"), "newtext #new").unwrap();
        assert_eq!(
            vault.list_note_paths(None, 10, None).await.unwrap(),
            ["new.md"]
        );
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
        assert!(
            vault
                .list_note_paths(None, 10, None)
                .await
                .unwrap()
                .is_empty()
        );
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

    #[tokio::test]
    async fn patch_accepts_lf_snippets_for_crlf_notes_and_locates_ambiguity() {
        let (temp, vault) = temp_vault("crlf");
        let path = temp.path().join("crlf.md");
        fs::write(&path, "uno\r\ndos\r\ntres\r\nuno").unwrap();
        // read_note reports "\n" between lines; that snippet must still patch.
        vault
            .patch_note("crlf.md", "uno\ndos", "one\ntwo")
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "one\r\ntwo\r\ntres\r\nuno"
        );
        fs::write(&path, "x\r\nsame\r\ny\r\nsame").unwrap();
        let error = vault.patch_note("crlf.md", "same", "z").await.unwrap_err();
        assert!(error.to_string().contains("lines 2, 4"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "x\r\nsame\r\ny\r\nsame");
    }

    #[tokio::test]
    async fn append_starts_on_a_new_line_without_doubling_newlines() {
        let (temp, vault) = temp_vault("append");
        let path = temp.path().join("a.md");
        for (existing, added, expected) in [
            ("tres", "cuatro", "tres\ncuatro"),
            ("tres\n", "cuatro", "tres\ncuatro"),
            ("tres", "\ncuatro", "tres\ncuatro"),
            ("", "cuatro", "cuatro"),
            ("tres", "", "tres\n"),
        ] {
            fs::write(&path, existing).unwrap();
            vault.append_note("a.md", added).await.unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn hidden_folders_are_skipped_unless_named() {
        let (temp, vault) = temp_vault("hidden");
        for path in ["live.md", ".trash/old.md", ".git/x.md", "ok/.cache/y.md"] {
            let full = temp.path().join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "needle [[Target]] #tag").unwrap();
        }
        assert_eq!(
            vault.list_note_paths(None, 10, None).await.unwrap(),
            ["live.md"]
        );
        assert_eq!(vault.list_notes(None, 10, None).await.unwrap().len(), 1);
        let found = vault
            .search_notes(None, "needle", 10, false, 0)
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(vault.get_backlinks("Target", 10, 0).await.unwrap().len(), 1);
        assert_eq!(vault.list_tags().await.unwrap()[0].count, 1);
        // Naming the trash explicitly is how a deleted note is found again.
        assert_eq!(
            vault
                .list_note_paths(Some(".trash"), 10, None)
                .await
                .unwrap(),
            [".trash/old.md"]
        );
    }

    #[tokio::test]
    async fn search_ignores_case_and_accents_in_content_and_paths() {
        let (temp, vault) = temp_vault("accents");
        // Precomposed in the body; decomposed (as macOS may store it) in the name.
        fs::write(
            temp.path().join("cancio\u{301}n.md"),
            "Mi canción vieja\nÑandú",
        )
        .unwrap();
        for query in ["cancion", "CANCIÓN", "cancio\u{301}n"] {
            for path_only in [false, true] {
                let found = vault
                    .search_notes(None, query, 10, path_only, 0)
                    .await
                    .unwrap();
                assert_eq!(found.len(), 1, "{query} path_only={path_only}");
            }
        }
        for (query, hits) in [("nandu", 1), ("vieja", 1), ("viejo", 0), ("日本", 0)] {
            let found = vault.search_notes(None, query, 10, false, 0).await.unwrap();
            assert_eq!(found.len(), hits, "{query}");
        }
    }

    #[tokio::test]
    async fn delete_trashes_without_overwriting_and_move_never_replaces() {
        let (temp, vault) = temp_vault("trash");
        for path in ["a/note.md", "b/note.md"] {
            vault.create_note(path, path).await.unwrap();
        }
        assert_eq!(
            vault.delete_note("a/note.md").await.unwrap(),
            ".trash/note.md"
        );
        assert_eq!(
            vault.delete_note("b/note.md").await.unwrap(),
            ".trash/note 2.md"
        );
        assert!(!temp.path().join("a/note.md").exists());
        assert_eq!(
            vault.read_note(".trash/note.md").await.unwrap(),
            "a/note.md"
        );
        assert_eq!(
            vault.read_note(".trash/note 2.md").await.unwrap(),
            "b/note.md"
        );
        assert!(matches!(
            vault.delete_note(".trash/note.md").await,
            Err(ObsidianError::AlreadyInTrash(_))
        ));
        assert!(matches!(
            vault.delete_note("a/note.md").await,
            Err(ObsidianError::NoteNotFound(_))
        ));

        assert!(matches!(
            vault.move_note(".trash/note.md", ".trash/note 2.md").await,
            Err(ObsidianError::NoteAlreadyExists(_))
        ));
        assert_eq!(
            vault.read_note(".trash/note.md").await.unwrap(),
            "a/note.md"
        );
        vault
            .move_note(".trash/note.md", "new/deep/renamed.md")
            .await
            .unwrap();
        assert_eq!(
            vault.read_note("new/deep/renamed.md").await.unwrap(),
            "a/note.md"
        );
        for (from, to) in [("new/deep/renamed.md", "x.txt"), ("../x.md", "y.md")] {
            assert!(vault.move_note(from, to).await.is_err());
        }
    }

    #[tokio::test]
    async fn backlinks_from_a_same_named_note_count_for_an_explicit_path() {
        let (_temp, vault) = temp_vault("same-name");
        vault.create_note("a/Target.md", "# Target").await.unwrap();
        vault
            .create_note("b/Target.md", "see [[a/Target]]")
            .await
            .unwrap();
        let sources = vault.get_backlinks("a/Target.md", 10, 0).await.unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].source_path, "b/Target.md");
        // A bare title cannot tell the two notes apart, so neither is a source.
        assert!(
            vault
                .get_backlinks("Target", 10, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn notes_can_be_listed_by_inline_frontmatter_and_nested_tags() {
        let (_temp, vault) = temp_vault("tags");
        for (path, content) in [
            ("front.md", "---\ntags: [Dev, x]\n---\nbody"),
            ("inline.md", "text #dev/backend"),
            ("link.md", "a [heading](#dev) link and #developer"),
            ("none.md", "nothing"),
        ] {
            vault.create_note(path, content).await.unwrap();
        }
        for tag in ["dev", "#Dev", " dev/ "] {
            assert_eq!(
                vault.list_note_paths(None, 10, Some(tag)).await.unwrap(),
                ["front.md", "inline.md"],
                "{tag}"
            );
        }
        let detailed = vault
            .list_notes(None, 10, Some("dev/backend"))
            .await
            .unwrap();
        assert_eq!(detailed.len(), 1);
        assert_eq!(detailed[0].path, "inline.md");
        // A page stops reading at its limit, like an unfiltered listing.
        assert_eq!(
            vault.list_note_paths(None, 1, Some("dev")).await.unwrap(),
            ["front.md"]
        );
        assert!(matches!(
            vault.list_note_paths(None, 10, Some(" # ")).await,
            Err(ObsidianError::EmptyTag)
        ));
        let tags = vault.list_tags().await.unwrap();
        assert!(tags.iter().any(|tag| tag.tag == "developer"));
        assert_eq!(tags.iter().find(|tag| tag.tag == "dev").unwrap().count, 1);
    }

    #[tokio::test]
    async fn update_rejects_a_stale_line_count_before_writing() {
        let (_temp, vault) = temp_vault("guard");
        vault.create_note("a.md", "1\n2\n3\n").await.unwrap();
        assert!(matches!(
            vault.update_note("a.md", "x", Some(2)).await,
            Err(ObsidianError::LineCountMismatch {
                expected: 2,
                actual: 3,
                ..
            })
        ));
        assert_eq!(vault.read_note("a.md").await.unwrap(), "1\n2\n3\n");
        assert_eq!(
            vault.update_note("a.md", "x\ny", Some(3)).await.unwrap(),
            (3, 2)
        );
        assert_eq!(vault.update_note("a.md", "", None).await.unwrap(), (2, 0));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn rewrites_keep_the_creation_date() {
        let (temp, vault) = temp_vault("created");
        vault.create_note("a.md", "one").await.unwrap();
        let path = temp.path().join("a.md");
        let created = fs::metadata(&path).unwrap().created().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
        vault.patch_note("a.md", "one", "two").await.unwrap();
        vault.update_note("a.md", "three", None).await.unwrap();
        let metadata = fs::metadata(&path).unwrap();
        assert_eq!(metadata.created().unwrap(), created);
        assert!(metadata.modified().unwrap() > created);
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
