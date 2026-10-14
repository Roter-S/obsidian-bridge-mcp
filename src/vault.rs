use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
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

#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct SearchMatch {
    pub path: String,
    #[schemars(with = "i32")]
    pub line: usize,
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

    pub async fn list_note_paths(
        &self,
        folder: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let mut paths = Vec::new();

        for entry in self.walk_markdown_files(&base)? {
            paths.push(self.relative_string(entry.path())?);
        }

        paths.sort();
        paths.truncate(limit);
        Ok(paths)
    }

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
        let full_content = self.read_note(path).await?;
        let lines: Vec<&str> = full_content.lines().collect();
        let total_lines = lines.len();

        let start = offset.unwrap_or(1).saturating_sub(1);
        let count = limit.unwrap_or(total_lines);

        if start >= total_lines {
            return Ok((String::new(), total_lines, 0));
        }

        let end = (start + count).min(total_lines);
        let selected = lines[start..end].join("\n");
        let read_count = end - start;

        Ok((selected, total_lines, read_count))
    }

    pub async fn get_note_outline(
        &self,
        path: &str,
    ) -> Result<(Vec<HeadingItem>, usize), ObsidianError> {
        let content = self.read_note(path).await?;
        let mut headings = Vec::new();
        let mut total_lines = 0;
        let mut in_code_block = false;

        for (idx, line) in content.lines().enumerate() {
            total_lines += 1;
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                in_code_block = !in_code_block;
                continue;
            }
            if in_code_block {
                continue;
            }

            let trimmed_start = line.trim_start();
            if trimmed_start.starts_with('#') {
                let hashes = trimmed_start.chars().take_while(|&c| c == '#').count();
                if (1..=6).contains(&hashes) {
                    let rest = &trimmed_start[hashes..];
                    if rest.starts_with(' ') || rest.is_empty() {
                        headings.push(HeadingItem {
                            level: hashes as u8,
                            text: rest.trim().to_string(),
                            line: idx + 1,
                        });
                    }
                }
            }
        }

        Ok((headings, total_lines))
    }

    pub async fn search_notes(
        &self,
        folder: Option<&str>,
        query: &str,
        limit: usize,
        path_only: bool,
    ) -> Result<Vec<SearchMatch>, ObsidianError> {
        let base = self.resolve_folder_path(folder)?;
        let query = query.trim();
        if query.is_empty() {
            return Err(ObsidianError::EmptyPath);
        }

        let needle = query.to_lowercase();
        let entries = self.walk_markdown_files(&base)?;

        if path_only {
            let mut matches = Vec::new();
            for entry in entries {
                let rel = self.relative_string(entry.path())?;
                if contains_ignore_case(&rel, &needle) {
                    matches.push(SearchMatch {
                        path: rel.clone(),
                        line: 1,
                        excerpt: rel,
                    });
                    if matches.len() >= limit {
                        break;
                    }
                }
            }
            return Ok(matches);
        }

        let semaphore = Arc::new(tokio::sync::Semaphore::new(32));
        let mut join_set = tokio::task::JoinSet::new();

        for entry in entries {
            let sem = Arc::clone(&semaphore);
            let path = entry.path().to_path_buf();
            let needle = needle.clone();

            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;
                let content = fs::read_to_string(&path).await.ok()?;
                let mut file_matches = Vec::new();

                for (index, line) in content.lines().enumerate() {
                    if contains_ignore_case(line, &needle) {
                        file_matches.push((index + 1, line.trim().to_string()));
                    }
                }
                Some((path, file_matches))
            });
        }

        let mut matches = Vec::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok(Some((path, file_matches))) = res
                && let Ok(rel) = self.relative_string(&path)
            {
                for (line, excerpt) in file_matches {
                    matches.push(SearchMatch {
                        path: rel.clone(),
                        line,
                        excerpt,
                    });
                    if matches.len() >= limit {
                        join_set.abort_all();
                        return Ok(matches);
                    }
                }
            }
        }

        matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
        matches.truncate(limit);
        Ok(matches)
    }

    pub async fn create_note(&self, path: &str, content: &str) -> Result<(), ObsidianError> {
        let path = self.resolve_note_path(path, false)?;
        if fs::try_exists(&path).await? {
            return Err(ObsidianError::NoteAlreadyExists(path_to_string(&path)));
        }

        self.atomic_write(&path, content).await?;
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
        self.atomic_write(&path, content).await?;
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
        self.atomic_write(&full_path, &patched_content).await?;
        Ok(())
    }

    pub async fn get_backlinks(
        &self,
        target_path_or_title: &str,
        limit: usize,
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

        let entries = self.walk_markdown_files(&self.root)?;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(32));
        let mut join_set = tokio::task::JoinSet::new();

        for entry in entries {
            let sem = Arc::clone(&semaphore);
            let path = entry.path().to_path_buf();
            let stem = stem.clone();
            let target_norm = target_norm.clone();

            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;
                let content = fs::read_to_string(&path).await.ok()?;
                let mut file_matches = Vec::new();

                for (index, line) in content.lines().enumerate() {
                    if line.contains("[[") && line_contains_wikilink_to(line, &stem, &target_norm) {
                        file_matches.push((index + 1, line.trim().to_string()));
                    }
                }
                Some((path, file_matches))
            });
        }

        let mut backlinks = Vec::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok(Some((path, file_matches))) = res
                && let Ok(rel) = self.relative_string(&path)
            {
                let rel_stem = Path::new(&rel)
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .unwrap_or("")
                    .to_lowercase();
                let rel_norm = rel.strip_suffix(".md").unwrap_or(&rel).to_lowercase();

                if rel_stem == stem || rel_norm == target_norm {
                    continue;
                }

                for (line, excerpt) in file_matches {
                    backlinks.push(BacklinkMatch {
                        source_path: rel.clone(),
                        line,
                        excerpt,
                    });
                    if backlinks.len() >= limit {
                        join_set.abort_all();
                        break;
                    }
                }
            }
        }

        backlinks.sort_by(|a, b| a.source_path.cmp(&b.source_path).then(a.line.cmp(&b.line)));
        backlinks.truncate(limit);
        Ok(backlinks)
    }

    pub async fn list_tags(&self) -> Result<Vec<TagInfo>, ObsidianError> {
        let entries = self.walk_markdown_files(&self.root)?;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(32));
        let mut join_set = tokio::task::JoinSet::new();

        for entry in entries {
            let sem = Arc::clone(&semaphore);
            let path = entry.path().to_path_buf();
            join_set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;
                let content = fs::read_to_string(&path).await.ok()?;
                Some(extract_tags_from_content(&content))
            });
        }

        let mut tag_counts: HashMap<String, usize> = HashMap::new();
        while let Some(res) = join_set.join_next().await {
            if let Ok(Some(tags)) = res {
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

    async fn atomic_write(&self, path: &Path, content: &str) -> Result<(), ObsidianError> {
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

        if let Err(e) = fs::rename(&temp_path, path).await {
            let _ = fs::remove_file(&temp_path).await;
            return Err(ObsidianError::Io(e));
        }

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

fn extract_tags_from_content(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut line_idx = 0;

    if let Some(&first_line) = lines.first()
        && first_line.trim() == "---"
    {
        line_idx = 1;
        let mut in_tags_list = false;
        while line_idx < lines.len() {
            let line = lines[line_idx];
            if line.trim() == "---" {
                line_idx += 1;
                break;
            }
            let trimmed = line.trim();
            if trimmed.starts_with("tags:") || trimmed.starts_with("tag:") {
                let rest = trimmed.split_once(':').map(|(_, r)| r.trim()).unwrap_or("");
                if rest.starts_with('[') && rest.ends_with(']') {
                    let inside = &rest[1..rest.len() - 1];
                    for item in inside.split(',') {
                        let clean = item.trim().trim_matches(['\'', '"', '#']);
                        if is_valid_tag_name(clean) {
                            tags.push(clean.to_lowercase());
                        }
                    }
                } else if !rest.is_empty() {
                    let clean = rest.trim_matches(['\'', '"', '#']);
                    if is_valid_tag_name(clean) {
                        tags.push(clean.to_lowercase());
                    }
                } else {
                    in_tags_list = true;
                }
            } else if in_tags_list {
                if trimmed.starts_with('-') {
                    let item = trimmed.trim_start_matches('-').trim();
                    let clean = item.trim_matches(['\'', '"', '#']);
                    if is_valid_tag_name(clean) {
                        tags.push(clean.to_lowercase());
                    }
                } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    in_tags_list = false;
                }
            }
            line_idx += 1;
        }
    }

    let mut in_code_block = false;
    for &line in &lines[line_idx..] {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '#' {
                let prev_ok = i == 0
                    || chars[i - 1].is_whitespace()
                    || chars[i - 1] == '('
                    || chars[i - 1] == '[';
                if prev_ok
                    && i + 1 < chars.len()
                    && !chars[i + 1].is_whitespace()
                    && chars[i + 1] != '#'
                {
                    let mut j = i + 1;
                    while j < chars.len()
                        && (chars[j].is_alphanumeric()
                            || chars[j] == '_'
                            || chars[j] == '-'
                            || chars[j] == '/')
                    {
                        j += 1;
                    }
                    let tag_candidate: String = chars[i + 1..j].iter().collect();
                    if is_valid_tag_name(&tag_candidate) {
                        tags.push(tag_candidate.to_lowercase());
                    }
                    i = j;
                    continue;
                }
            }
            i += 1;
        }
    }

    tags.sort();
    tags.dedup();
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
    async fn search_notes_returns_matches() {
        let (_temp, vault) = temp_vault("The Core");

        vault
            .create_note("Projects/demo.md", "line one\nPending auth\nline three")
            .await
            .expect("create note");

        let matches = vault
            .search_notes(Some("Projects"), "pending", 10, false)
            .await
            .expect("search notes");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line, 2);

        let path_matches = vault
            .search_notes(None, "demo", 10, true)
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
            .get_backlinks("Target.md", 10)
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
