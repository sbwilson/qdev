use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::id::{Identifier, IdentifierKind};
use crate::schema::{validate_frontmatter, validate_frontmatter_detailed, EntityKind};

/// Advisory lock guard releasing the file lock on drop.
#[derive(Debug)]
pub struct AdvisoryLockGuard {
    file: File,
    path: PathBuf,
}

impl AdvisoryLockGuard {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for AdvisoryLockGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn is_lock_contended(err: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return true;
        }
        if let Some(code) = err.raw_os_error() {
            // EAGAIN / EWOULDBLOCK (35 on BSD/macOS, 11 on Linux)
            if code == 35 || code == 11 {
                return true;
            }
        }
    }
    #[cfg(windows)]
    {
        // ERROR_LOCK_VIOLATION is 33
        if let Some(code) = err.raw_os_error() {
            if code == 33 {
                return true;
            }
        }
    }
    false
}

/// Spells a path relative to the workspace root with forward slashes, whatever the platform
/// separator is. The result is both what the user is shown and what lands in
/// `entities.source_path`, and the sweep writes that column with `/` — a Windows write spelling
/// the same file `docs\specs\stories\E1S9.md` would give it a second identity the sweep's
/// path-keyed purge and stale flags could never match.
///
/// It walks components rather than replacing backslashes in the string: on Unix a backslash is a
/// legal character in a file name, and rewriting `a\b.md` to `a/b.md` would invent a path no file
/// carries — swapping one identity split for another.
pub(crate) fn workspace_rel_path(file_path: &Path, workspace_root: &Path) -> String {
    let rel = file_path.strip_prefix(workspace_root).unwrap_or(file_path);
    let mut out = String::new();
    for component in rel.components() {
        // A root or prefix component already carries its own separator, so it is spelled as `/`
        // and nothing is appended after it. Every other component is joined with one `/`.
        let piece = match component {
            std::path::Component::RootDir => "/".to_string(),
            other => other.as_os_str().to_string_lossy().to_string(),
        };
        if out.is_empty() || out.ends_with('/') {
            out.push_str(&piece);
        } else {
            out.push('/');
            out.push_str(&piece);
        }
    }
    out
}

/// Acquires an exclusive advisory write lock on the specified file path with a timeout.
/// Creates parent directories and the lock file if they do not exist.
/// Fails with ExitCode::Conflict (`lock_timeout`) if the timeout expires.
pub fn acquire_write_lock(
    lock_path: &Path,
    timeout: Duration,
) -> Result<AdvisoryLockGuard, QdevError> {
    if let Some(parent) = lock_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                QdevError::infrastructure_failure(
                    "io_error",
                    format!(
                        "Failed to create lock directory '{}': {}",
                        parent.display(),
                        e
                    ),
                )
            })?;
        }
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to open lock file '{}': {}", lock_path.display(), e),
            )
        })?;

    let start = Instant::now();
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => {
                return Ok(AdvisoryLockGuard {
                    file,
                    path: lock_path.to_path_buf(),
                });
            }
            Err(err) => {
                if !is_lock_contended(&err) {
                    return Err(QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to lock '{}': {}", lock_path.display(), err),
                    ));
                }
                if start.elapsed() >= timeout {
                    return Err(QdevError::conflict(
                        "lock_timeout",
                        format!(
                            "Failed to acquire advisory write lock on '{}' within {}ms",
                            lock_path.display(),
                            timeout.as_millis()
                        ),
                    )
                    .with_details(serde_json::json!({
                        "lock_file": lock_path.to_string_lossy(),
                        "timeout_ms": timeout.as_millis(),
                    })));
                }
                thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

/// Atomically writes content to `dest_path` by creating a temporary file in the target's
/// parent directory and renaming it over the destination.
pub fn write_file_atomic(dest_path: &Path, content: &str) -> Result<(), QdevError> {
    let parent = dest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    if !parent.exists() {
        fs::create_dir_all(parent).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to create directory '{}': {}", parent.display(), e),
            )
        })?;
    }

    let mut temp_file = tempfile::Builder::new()
        .prefix(".qdev_atomic_")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create temporary file in '{}': {}",
                    parent.display(),
                    e
                ),
            )
        })?;

    use std::io::Write;
    temp_file.write_all(content.as_bytes()).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to write to temporary file: {}", e),
        )
    })?;

    temp_file.flush().map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to flush temporary file: {}", e),
        )
    })?;

    temp_file.as_file().sync_all().map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to sync temporary file to disk: {}", e),
        )
    })?;

    temp_file.persist(dest_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to persist atomic temporary file to destination '{}': {}",
                dest_path.display(),
                e
            ),
        )
    })?;

    Ok(())
}

/// Author attribution metadata.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Author {
    #[serde(rename = "type")]
    pub author_type: String,
    pub id: String,
}

impl Author {
    pub fn new(author_type: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            author_type: author_type.into(),
            id: id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), QdevError> {
        if self.author_type != "human" && self.author_type != "agent" {
            return Err(QdevError::usage_error(format!(
                "Invalid author type '{}', must be 'human' or 'agent'",
                self.author_type
            )));
        }
        if self.id.trim().is_empty() {
            return Err(QdevError::usage_error("Author ID cannot be empty"));
        }
        // Attribution is audited (AD-12) and is written into YAML frontmatter. A newline or
        // control character in the id is never a real developer or agent name, and letting one
        // through means the audited field is describing something the writer did not intend.
        if self.id.chars().any(|c| c.is_control()) {
            return Err(QdevError::usage_error(format!(
                "Author ID must be a single line without control characters, got {:?}",
                self.id
            )));
        }
        Ok(())
    }
}

/// Options controlling frontmatter patching.
#[derive(Debug, Clone, Default)]
pub struct FrontmatterPatchOptions {
    pub status: Option<String>,
    pub title: Option<String>,
    pub custom_fields: Vec<(String, serde_yaml::Value)>,
    pub author: Option<Author>,
    pub if_version: Option<u64>,
}

#[derive(Debug)]
struct KeyBlock {
    key: String,
    start_line: usize,
    end_line: usize, // exclusive
}

/// Applies line-based frontmatter mutations to a Markdown document.
/// Preserves existing comments, blank lines, and key order outside edited fields byte-for-byte.
/// Increments the entity version integer by 1 and updates `updated_by`.
/// If `if_version` is set, verifies that the existing version matches; fails with exit 5 on mismatch.
pub fn patch_frontmatter(
    content: &str,
    options: &FrontmatterPatchOptions,
) -> Result<(String, u64), QdevError> {
    if let Some(ref author) = options.author {
        author.validate()?;
    }

    // Validate that custom fields do not attempt to mutate managed keys
    for (k, _) in &options.custom_fields {
        let k_lower = k.to_lowercase();
        if k_lower == "id"
            || k_lower == "version"
            || k_lower == "updated_by"
            || k_lower == "created_by"
        {
            return Err(QdevError::usage_error(format!(
                "Cannot modify managed frontmatter field '{}' via custom fields",
                k
            )));
        }
    }

    // Split entire content into lines preserving line endings
    let all_lines: Vec<&str> = content.split_inclusive('\n').collect();
    if all_lines.is_empty() {
        return Err(QdevError::logical_failure(
            "missing_frontmatter",
            "File content is empty",
        ));
    }

    // Locate opening frontmatter delimiter at start of document
    let mut open_idx = None;
    for (idx, &line) in all_lines.iter().enumerate() {
        let line_no_eol = line.trim_end_matches(['\r', '\n']);
        let stripped = line_no_eol.strip_prefix('\u{feff}').unwrap_or(line_no_eol);
        if stripped == "---" {
            open_idx = Some(idx);
            break;
        } else if stripped.trim().is_empty() {
            // Permitted whitespace or blank line before frontmatter opening
            continue;
        } else {
            // Content encountered before opening frontmatter delimiter
            return Err(QdevError::logical_failure(
                "missing_frontmatter",
                "Missing opening frontmatter delimiter '---' at start of document",
            ));
        }
    }

    let open_line = open_idx.ok_or_else(|| {
        QdevError::logical_failure(
            "missing_frontmatter",
            "Missing opening frontmatter delimiter '---'",
        )
    })?;

    // Locate closing frontmatter delimiter (must be unindented)
    let mut close_idx = None;
    for (idx, &line) in all_lines.iter().enumerate().skip(open_line + 1) {
        let line_no_eol = line.trim_end_matches(['\r', '\n']);
        if line_no_eol == "---" || line_no_eol == "..." {
            close_idx = Some(idx);
            break;
        }
    }

    let close_line = close_idx.ok_or_else(|| {
        QdevError::logical_failure(
            "unclosed_frontmatter",
            "Unclosed frontmatter delimiter '---'",
        )
    })?;

    // Detect line ending (CRLF or LF)
    let newline = if all_lines
        .iter()
        .any(|l| l.ends_with("\r\n") || l.contains("\r\n"))
    {
        "\r\n"
    } else {
        "\n"
    };

    let fm_lines = &all_lines[(open_line + 1)..close_line];

    // Identify top-level key blocks in frontmatter lines
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < fm_lines.len() {
        let line = fm_lines[i];
        let trimmed = line.trim_end_matches(['\r', '\n']);

        // Check if line starts a top-level key: unindented, not comment, not list item
        if !trimmed.starts_with(' ')
            && !trimmed.starts_with('\t')
            && !trimmed.starts_with('#')
            && !trimmed.starts_with('-')
        {
            if let Some((key_cand, _)) = trimmed.split_once(':') {
                let key = key_cand.trim();
                if !key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                {
                    let start = i;
                    i += 1;
                    while i < fm_lines.len() {
                        let next_line = fm_lines[i];
                        if next_line.starts_with(' ') || next_line.starts_with('\t') {
                            i += 1;
                        } else if next_line.trim().is_empty() {
                            // Look ahead past blank lines to check if indented lines follow
                            let mut lookahead = i + 1;
                            while lookahead < fm_lines.len()
                                && fm_lines[lookahead].trim().is_empty()
                            {
                                lookahead += 1;
                            }
                            if lookahead < fm_lines.len()
                                && (fm_lines[lookahead].starts_with(' ')
                                    || fm_lines[lookahead].starts_with('\t'))
                            {
                                i = lookahead + 1;
                            } else {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    blocks.push(KeyBlock {
                        key: key.to_string(),
                        start_line: start,
                        end_line: i,
                    });
                    continue;
                }
            }
        }
        i += 1;
    }

    // The version this patch fences against, read by the one function that answers that
    // question for the whole write path (see `frontmatter_version`).
    let existing_version = frontmatter_version(content);
    check_if_version(options.if_version, existing_version)?;

    let current_version = existing_version.unwrap_or(0);
    let new_version = current_version.checked_add(1).ok_or_else(|| {
        QdevError::logical_failure(
            "version_overflow",
            "Entity version overflow (u64::MAX reached)",
        )
    })?;

    // Build replacement blocks for updated keys
    let mut updates: BTreeMap<String, Vec<String>> = BTreeMap::new();

    // 1. Version
    updates.insert(
        "version".to_string(),
        vec![format!("version: {}{}", new_version, newline)],
    );

    // 2. Updated_by
    if let Some(ref author) = options.author {
        let id_yaml = serde_yaml::to_string(&author.id)
            .unwrap_or_else(|_| format!("{:?}", author.id))
            .trim()
            .to_string();
        updates.insert(
            "updated_by".to_string(),
            vec![
                format!("updated_by:{}", newline),
                format!("  type: {}{}", author.author_type, newline),
                format!("  id: {}{}", id_yaml, newline),
            ],
        );
    }

    // 3. Status
    if let Some(ref status) = options.status {
        updates.insert(
            "status".to_string(),
            vec![format!("status: {}{}", status, newline)],
        );
    }

    // 4. Title
    if let Some(ref title) = options.title {
        let title_yaml = serde_yaml::to_string(title)
            .unwrap_or_else(|_| format!("{:?}", title))
            .trim()
            .to_string();
        updates.insert(
            "title".to_string(),
            vec![format!("title: {}{}", title_yaml, newline)],
        );
    }

    // 5. Custom fields
    for (k, v) in &options.custom_fields {
        let yaml_str = serde_yaml::to_string(v).unwrap_or_default();
        let lines: Vec<String> = yaml_str
            .lines()
            .map(|l| format!("{}{}", l, newline))
            .collect();
        match v {
            serde_yaml::Value::Mapping(_) | serde_yaml::Value::Sequence(_) => {
                let mut key_lines = vec![format!("{}:{}", k, newline)];
                for l in lines {
                    key_lines.push(format!("  {}", l));
                }
                updates.insert(k.clone(), key_lines);
            }
            _ => {
                if lines.len() == 1 {
                    updates.insert(
                        k.clone(),
                        vec![format!("{}: {}{}", k, lines[0].trim_end(), newline)],
                    );
                } else {
                    let mut key_lines = vec![format!("{}:{}", k, newline)];
                    for l in lines {
                        key_lines.push(format!("  {}", l));
                    }
                    updates.insert(k.clone(), key_lines);
                }
            }
        }
    }

    // Reconstruct frontmatter lines
    let mut result_lines = Vec::new();
    let mut line_idx = 0;
    let mut applied_keys = HashSet::new();

    while line_idx < fm_lines.len() {
        if let Some(block) = blocks.iter().find(|b| b.start_line == line_idx) {
            if let Some(new_lines) = updates.get(&block.key) {
                result_lines.extend(new_lines.iter().cloned());
                applied_keys.insert(block.key.clone());
                line_idx = block.end_line;
                continue;
            }
        }
        result_lines.push(fm_lines[line_idx].to_string());
        line_idx += 1;
    }

    // Append any new fields that were not in existing blocks
    for (key, new_lines) in &updates {
        if !applied_keys.contains(key) {
            result_lines.extend(new_lines.iter().cloned());
        }
    }

    // Assemble final content
    let mut final_content = String::new();
    for &line in &all_lines[..=open_line] {
        final_content.push_str(line);
    }
    for line in result_lines {
        final_content.push_str(&line);
    }
    for &line in &all_lines[close_line..] {
        final_content.push_str(line);
    }

    Ok((final_content, new_version))
}

/// Parses a heading line into (level, title) if valid.
/// Indented lines with >= 4 spaces or starting with a tab are code blocks in Markdown, not headings.
pub(crate) fn parse_heading_line(line: &str) -> Option<(usize, &str)> {
    if line.starts_with('\t') {
        return None;
    }
    let leading_spaces = line.chars().take_while(|&c| c == ' ').count();
    if leading_spaces >= 4 {
        return None;
    }
    let trimmed = &line[leading_spaces..];
    if !trimmed.starts_with('#') {
        return None;
    }
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    if hash_count == 0 || hash_count > 6 {
        return None;
    }
    let after_hashes = &trimmed[hash_count..];
    if after_hashes.starts_with(' ') || after_hashes.starts_with('\t') {
        let title = after_hashes.trim_end_matches(['\r', '\n']).trim();
        Some((hash_count, title))
    } else {
        None
    }
}

/// Helper tracking Markdown fenced code blocks (``` or ~~~).
#[derive(Default)]
pub(crate) struct FenceTracker {
    fence: Option<(char, usize)>,
}

impl FenceTracker {
    /// Processes a line and returns whether this line is part of a fenced code block
    /// (including opening or closing fence lines).
    pub(crate) fn process_line(&mut self, line: &str) -> bool {
        if line.starts_with('\t') {
            return self.fence.is_some();
        }
        let leading_spaces = line.chars().take_while(|&c| c == ' ').count();
        if leading_spaces >= 4 {
            return self.fence.is_some();
        }
        let trimmed = &line[leading_spaces..];

        if let Some((fence_char, min_len)) = self.fence {
            let run = trimmed.chars().take_while(|&c| c == fence_char).count();
            if run >= min_len {
                let rest = &trimmed[run..];
                if rest.trim().is_empty() {
                    self.fence = None;
                    return true;
                }
            }
            true
        } else {
            if trimmed.starts_with("```") {
                let count = trimmed.chars().take_while(|&c| c == '`').count();
                self.fence = Some(('`', count));
                true
            } else if trimmed.starts_with("~~~") {
                let count = trimmed.chars().take_while(|&c| c == '~').count();
                self.fence = Some(('~', count));
                true
            } else {
                false
            }
        }
    }
}

/// Returns whether the Markdown content contains a heading with the given title (case-insensitive),
/// ignoring headings inside fenced code blocks or indented code blocks.
pub(crate) fn has_markdown_heading(content: &str, expected_title: &str) -> bool {
    let body = crate::schema::extract_frontmatter_str(content)
        .map(|(_, b)| b)
        .unwrap_or(content);
    let mut tracker = FenceTracker::default();
    for line in body.lines() {
        let inside_fence = tracker.process_line(line);
        if inside_fence {
            continue;
        }
        if let Some((_level, title)) = parse_heading_line(line) {
            if title.eq_ignore_ascii_case(expected_title) {
                return true;
            }
        }
    }
    false
}

/// Replaces exactly one matching Markdown section body in `markdown` with `new_section_body`.
/// Headings, sections, and text outside the matched section are left byte-for-byte untouched.
/// Errors with ExitCode::UsageError if 0 or >1 sections match.
pub fn replace_markdown_section(
    markdown: &str,
    heading: &str,
    new_section_body: &str,
) -> Result<String, QdevError> {
    let heading_arg = heading.trim();
    if heading_arg.is_empty() {
        return Err(QdevError::usage_error("Section heading cannot be empty"));
    }

    let (target_level_opt, target_title) = if heading_arg.starts_with('#') {
        let hash_count = heading_arg.chars().take_while(|&c| c == '#').count();
        let title = heading_arg[hash_count..].trim();
        (Some(hash_count), title)
    } else {
        (None, heading_arg)
    };

    // Separate frontmatter from body so frontmatter comments are not matched as headings
    let all_lines: Vec<&str> = markdown.split_inclusive('\n').collect();
    let mut body_start_line = 0;

    let mut open_idx = None;
    for (idx, &line) in all_lines.iter().enumerate() {
        let line_no_eol = line.trim_end_matches(['\r', '\n']);
        let stripped = line_no_eol.strip_prefix('\u{feff}').unwrap_or(line_no_eol);
        if stripped == "---" {
            open_idx = Some(idx);
            break;
        } else if !stripped.trim().is_empty() {
            break;
        }
    }

    if let Some(open) = open_idx {
        for (idx, &line) in all_lines.iter().enumerate().skip(open + 1) {
            let line_no_eol = line.trim_end_matches(['\r', '\n']);
            if line_no_eol == "---" || line_no_eol == "..." {
                body_start_line = idx + 1;
                break;
            }
        }
    }

    // Find all headings matching target in body lines (skipping code blocks)
    let mut matches = Vec::new();
    let mut fence_tracker = FenceTracker::default();
    for (idx, &line) in all_lines.iter().enumerate().skip(body_start_line) {
        let is_fence = fence_tracker.process_line(line);
        if !is_fence {
            if let Some((level, title)) = parse_heading_line(line) {
                let matched = match target_level_opt {
                    Some(req_level) => {
                        req_level == level && title.eq_ignore_ascii_case(target_title)
                    }
                    None => title.eq_ignore_ascii_case(target_title),
                };
                if matched {
                    matches.push((idx, level));
                }
            }
        }
    }

    if matches.is_empty() {
        return Err(QdevError::usage_error(format!(
            "Section '{}' not found in entity body",
            heading
        )));
    }
    if matches.len() > 1 {
        return Err(QdevError::usage_error(format!(
            "Multiple sections matching '{}' found in entity body",
            heading
        )));
    }

    let (match_line, match_level) = matches[0];

    // Find next heading with level <= match_level (or end of file), skipping code blocks
    let mut section_end_line = all_lines.len();
    let mut end_fence_tracker = FenceTracker::default();
    for (idx, &line) in all_lines.iter().enumerate().skip(match_line + 1) {
        let is_fence = end_fence_tracker.process_line(line);
        if !is_fence {
            if let Some((level, _)) = parse_heading_line(line) {
                if level <= match_level {
                    section_end_line = idx;
                    break;
                }
            }
        }
    }

    // Format new section body
    let mut formatted_body = new_section_body.to_string();
    if !formatted_body.is_empty() && !formatted_body.ends_with('\n') {
        formatted_body.push('\n');
    }

    let mut result = String::new();
    for &line in &all_lines[..=match_line] {
        result.push_str(line);
    }
    // Ensure newline exists after heading if heading had no trailing newline
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result.push_str(&formatted_body);
    for &line in &all_lines[section_end_line..] {
        result.push_str(line);
    }

    Ok(result)
}

/// In-memory representation of an entity cache row to be upserted.
pub use crate::store::EntityRecord;

/// The single civil-calendar conversion in the codebase: seconds since the Unix epoch to an
/// ISO8601 `Z` timestamp. Every `found_at`, `updated_at`, and `dirty_at` in the cache goes
/// through here, so two rows written by different code paths can never disagree about the
/// calendar. `rem_euclid` keeps pre-epoch timestamps (from a file mtime) correct.
pub fn iso8601_from_timestamp(secs: i64) -> String {
    let s = (secs.rem_euclid(60)) as u64;
    let m = ((secs / 60).rem_euclid(60)) as u64;
    let h = ((secs / 3600).rem_euclid(24)) as u64;
    let mut days = secs.div_euclid(86400);
    days += 719468;
    let era = (if days >= 0 { days } else { days - 146096 }) / 146097;
    let doe = (days - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, d, h, m, s
    )
}

/// The current time as an ISO8601 `Z` timestamp.
pub fn current_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601_from_timestamp(secs)
}

/// Computes the sha256 hex digest of a byte slice.
pub fn sha256_digest(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// One relation edge to apply to the cache's `relations` table in the same transaction as an
/// entity upsert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationRowChange {
    pub source_id: String,
    pub relation: String,
    pub target_id: String,
    /// `true` inserts the edge, `false` deletes it.
    pub add: bool,
}

/// Deletes one entity's own rows — `entities`, its `stories` detail row, and its
/// `dirty_entities` marker — and nothing else.
///
/// Not the hydration purge cascade (`purge_entity_with_children`): it deliberately leaves
/// `relations` and `constraints` alone. Every caller here is repairing a row that names an id
/// its file no longer declares, while the edges pointing at that id are being redirected by a
/// separate write; a cascade would delete those edges instead of moving them.
fn delete_entity_row_shallow(tx: &rusqlite::Transaction, id: &str) -> Result<(), QdevError> {
    // The detail row is deleted through hydration's own kind mapping rather than a second copy
    // of it: hardcoding `stories` here would leave an orphan row for every other kind — a
    // renumbered deferred-work item, decision, sprint, SOUP entry or evidence record keyed on
    // an id no file declares any more.
    let kind: Option<String> = tx
        .query_row(
            "SELECT kind FROM entities WHERE id = ?1;",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to read the kind of entity '{}': {}", id, e),
            )
        })?;
    if let Some(kind) = kind
        .as_deref()
        .and_then(|k| EntityKind::from_str_loose(k).ok())
    {
        crate::store::sqlite::delete_kind_detail_row(tx, id, kind)?;
    }

    for (sql, what) in [
        // Constraints are owned by the entity and keyed on its id, so they are orphaned by an
        // id edit exactly as the detail row is. `relations` are deliberately *not* deleted —
        // `--fix-ids` redirects the inbound edges after the renumber, and a cascade here would
        // delete the edges it is about to rewrite.
        (
            "DELETE FROM constraints WHERE owner_id = ?1;",
            "constraint rows",
        ),
        ("DELETE FROM dirty_entities WHERE id = ?1;", "dirty marker"),
        ("DELETE FROM entities WHERE id = ?1;", "entity row"),
    ] {
        tx.execute(sql, rusqlite::params![id]).map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to delete {} for entity '{}': {}", what, id, e),
            )
        })?;
    }
    Ok(())
}

/// Drops the cache row for `id` when — and only when — it still claims `source_path`, together
/// with that path's `sync_state` and `findings` rows.
///
/// This is the write path's answer to a file that *moved*: `qdev validate --fix-ids` renames a
/// renumbered file, so the row naming the old id at the old path describes a file that no longer
/// exists there. Left in place it is a second row for one entity, and `qdev get <old id>` keeps
/// answering from it. The guard on `source_path` is what makes this safe in the duplicate case:
/// when the old id's row belongs to the *keeper* file (which still holds that id, on disk and
/// unchanged), the path does not match and the row is left exactly as it is.
///
/// `relations` are untouched, for the reason given on [`delete_entity_row_shallow`].
pub fn purge_entity_row_for_moved_file(
    cache_db_path: &Path,
    id: &str,
    source_path: &str,
) -> Result<bool, QdevError> {
    if !cache_db_path.exists() {
        return Ok(false);
    }
    let store = crate::store::SqliteStore::open(cache_db_path)?;
    store.with_conn_mut(|conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to begin purge transaction: {}", e),
                )
            })?;

        let claims: bool = tx
            .query_row(
                "SELECT COUNT(*) FROM entities WHERE id = ?1 AND source_path = ?2;",
                rusqlite::params![id, source_path],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to check the cache row for '{}': {}", id, e),
                )
            })?;

        if claims {
            delete_entity_row_shallow(&tx, id)?;
        }

        // The path itself is gone either way: its `sync_state` row would otherwise let a later
        // incremental sweep believe a vanished file was already accounted for, and its findings
        // would be reported against a file that no longer exists.
        for sql in [
            "DELETE FROM sync_state WHERE path = ?1;",
            "DELETE FROM findings WHERE path = ?1;",
        ] {
            tx.execute(sql, rusqlite::params![source_path])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to clear cache rows for '{}': {}", source_path, e),
                    )
                })?;
        }

        tx.commit().map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to commit purge transaction for '{}': {}", id, e),
            )
        })?;
        Ok(claims)
    })
}

/// Upserts an updated entity into the SQLite cache `entities` (and kind-specific) table,
/// records its dirty status in `dirty_entities`, and invalidates `sync_state`.
/// Configures WAL mode and `busy_timeout = 5000ms`.
///
/// When `entity.kind` differs from the kind the cache already holds for this id, the *previous*
/// kind's detail rows are deleted in the same transaction — see [`upsert_cache_with_relation`].
pub fn upsert_cache_and_mark_dirty(
    cache_db_path: &Path,
    entity: &EntityRecord,
) -> Result<(), QdevError> {
    upsert_cache_with_relation(cache_db_path, entity, None)
}

/// `upsert_cache_and_mark_dirty` plus, when `relation_change` is given, the matching
/// insert/delete on the `relations` table — in the same transaction.
///
/// Without this the `relations` table only caught up at the next process's boot sweep, so every
/// in-process reader (`qdev relate`'s own cycle pre-check, `query_entity`, `render_graph_dot`)
/// saw pre-write state, and two relation operations in one process would validate the second
/// against a graph that ignored the first.
///
/// Also the site that keeps a *kind change* repairable: when `entity.kind` differs from the kind
/// the cache holds for this id, the previous kind's detail rows are deleted in the same
/// transaction, through hydration's own kind->table mapping. Hydration cannot do it afterwards —
/// its repair compares against the cached kind, which this upsert has already replaced — so
/// without it the orphan survives every sweep and only `sync --rebuild` removes it.
pub fn upsert_cache_with_relation(
    cache_db_path: &Path,
    entity: &EntityRecord,
    relation_change: Option<&RelationRowChange>,
) -> Result<(), QdevError> {
    let store = crate::store::SqliteStore::open(cache_db_path)?;

    // Ensure schema v2 exists
    store.with_conn(|conn| {
        // Create-only: this opens whatever cache the workspace already has, so it must never
        // stamp a version onto tables it did not migrate.
        crate::store::create_schema(conn)?;
        Ok(())
    })?;

    store.with_conn_mut(|conn| {
        let tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to begin transaction: {}", e),
                )
            })?;

        let (c_type, c_id) = match entity.created_by {
            Some(ref a) => (Some(a.author_type.clone()), Some(a.id.clone())),
            None => (None, None),
        };
        let (u_type, u_id) = match entity.updated_by {
            Some(ref a) => (Some(a.author_type.clone()), Some(a.id.clone())),
            None => (None, None),
        };

        // 0. Drop any row still claiming this file under a different id, the way hydration does
        // (`sqlite.rs`, "in-place id edit"). `ON CONFLICT(id)` alone cannot see that collision,
        // so an id edit — `qdev validate --fix-ids` is the only writer that makes one — left two
        // `entities` rows pointing at one file, one of them naming an id the file no longer
        // declares.
        //
        // Deliberately narrower than hydration's cascade: `relations` rows are left alone.
        // `--fix-ids` redirects inbound edges through `qdev relate`'s own write path *after* the
        // renumber, so deleting `relations WHERE target_id = <old id>` here would delete the
        // edges it is about to redirect and lose them silently.
        let stale_path_ids: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT id FROM entities WHERE source_path = ?1 AND id <> ?2;")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare source_path conflict scan: {}", e),
                    )
                })?;
            let rows = stmt
                .query_map(
                    rusqlite::params![entity.source_path, entity.id],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to scan for rows claiming '{}': {}", entity.source_path, e),
                    )
                })?;
            let mut ids = Vec::new();
            for r in rows {
                ids.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to read conflicting row id: {}", e),
                    )
                })?);
            }
            ids
        };
        for stale_id in &stale_path_ids {
            delete_entity_row_shallow(&tx, stale_id)?;
        }

        // 0b. A write may change an entity's *kind*. The `ON CONFLICT(id)` update below
        // replaces the cached kind in place, which is the whole reason hydration's repair
        // (`clear_owned_child_rows`) cannot reach this case: it asks the `entities` table what
        // the previous kind was, and by the time any later sweep looks, the write has already
        // stored the new one, so the comparison finds equality and drops nothing. The previous
        // kind's detail row then outlives every sweep and only `sync --rebuild` removes it —
        // and it is user-visible, because `get_entity` and `list_entities` LEFT JOIN `stories`.
        //
        // The write path is the only place that still holds both halves, so it answers here,
        // in the same transaction, through the shared kind->table mapping. Note this drops only
        // the *previous* kind's rows: materializing the new kind's detail row stays hydration's
        // job on the sweep the dirty marker below already forces, the same one-pass lag every
        // other non-story field has.
        let prev_kind: Option<String> = tx
            .query_row(
                "SELECT kind FROM entities WHERE id = ?1;",
                rusqlite::params![entity.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to read the cached kind of entity '{}': {}",
                        entity.id, e
                    ),
                )
            })?;
        if let Some(prev_kind) = prev_kind
            .as_deref()
            .and_then(|k| EntityKind::from_str_loose(k).ok())
        {
            // The equal-kind case — every ordinary `update` — issues no delete at all.
            if prev_kind != entity.kind {
                crate::store::sqlite::delete_kind_detail_row(&tx, &entity.id, prev_kind)?;
            }
        }

        // 1. Upsert into entities table (preserve existing created_by via COALESCE if omitted)
        tx.execute(
            r#"
INSERT INTO entities (
    id, kind, title, status, owners, source_path, content_hash, version,
    created_by_type, created_by_id, updated_by_type, updated_by_id, updated_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
ON CONFLICT(id) DO UPDATE SET
    kind = excluded.kind,
    title = excluded.title,
    status = excluded.status,
    owners = excluded.owners,
    source_path = excluded.source_path,
    content_hash = excluded.content_hash,
    version = excluded.version,
    created_by_type = COALESCE(excluded.created_by_type, entities.created_by_type),
    created_by_id = COALESCE(excluded.created_by_id, entities.created_by_id),
    updated_by_type = excluded.updated_by_type,
    updated_by_id = excluded.updated_by_id,
    updated_at = excluded.updated_at,
    stale = 0;
"#,
            rusqlite::params![
                entity.id,
                entity.kind.as_str(),
                entity.title,
                entity.status,
                entity.owners,
                entity.source_path,
                entity.content_hash,
                entity.version,
                c_type,
                c_id,
                u_type,
                u_id,
                entity.updated_at,
            ],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert into entities table: {}", e),
            )
        })?;

        // 2. Kind-specific upsert: stories
        if entity.kind == EntityKind::Story {
            if let (Some(ref epic), Some(seq)) = (&entity.epic_id, entity.seq) {
                tx.execute(
                    r#"
INSERT INTO stories (id, epic_id, seq, appetite, safety_class, target_modules)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(id) DO UPDATE SET
    epic_id = excluded.epic_id,
    seq = excluded.seq,
    appetite = excluded.appetite,
    safety_class = excluded.safety_class,
    target_modules = excluded.target_modules;
"#,
                    rusqlite::params![
                        entity.id,
                        epic,
                        seq,
                        entity.appetite,
                        entity.safety_class,
                        entity.target_modules,
                    ],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to upsert into stories table: {}", e),
                    )
                })?;
            }
        }

        // 3. Apply the relation edge, if this write is a relate/unrelate.
        if let Some(change) = relation_change {
            if change.add {
                tx.execute(
                    r#"
INSERT INTO relations (source_id, relation, target_id)
VALUES (?1, ?2, ?3)
ON CONFLICT(source_id, relation, target_id) DO NOTHING;
"#,
                    rusqlite::params![change.source_id, change.relation, change.target_id],
                )
            } else {
                tx.execute(
                    "DELETE FROM relations WHERE source_id = ?1 AND relation = ?2 AND target_id = ?3;",
                    rusqlite::params![change.source_id, change.relation, change.target_id],
                )
            }
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to apply relation '{}' --[{}]--> '{}': {}",
                        change.source_id, change.relation, change.target_id, e
                    ),
                )
            })?;
        }

        // 4. Mark row as dirty in dirty_entities
        let dirty_at = current_iso8601();
        tx.execute(
            r#"
INSERT INTO dirty_entities (id, dirty_at)
VALUES (?1, ?2)
ON CONFLICT(id) DO UPDATE SET dirty_at = excluded.dirty_at;
"#,
            rusqlite::params![entity.id, dirty_at],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to mark dirty in dirty_entities table: {}", e),
            )
        })?;

        // 5. Invalidate sync_state for this entity path
        tx.execute(
            "DELETE FROM sync_state WHERE path = ?1;",
            rusqlite::params![entity.source_path],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to invalidate sync_state: {}", e),
            )
        })?;

        tx.commit().map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to commit transaction: {}", e),
            )
        })?;

        Ok(())
    })
}

/// Standard entity file resolution directory map with optional storage config.
///
/// This is the directory half of the one identity rule stated on [`resolve_entity_file`]:
/// `qdev validate` reports an entity file that lives outside every directory this map names, so
/// the convention the write path assumes is enforced rather than merely hoped for.
pub fn directory_for_kind(storage: Option<&StorageConfig>, kind: EntityKind) -> PathBuf {
    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    match kind {
        EntityKind::Prd => Path::new(&st.specs_dir).join("prd"),
        EntityKind::Requirement => Path::new(&st.specs_dir).join("requirements"),
        EntityKind::Epic => Path::new(&st.specs_dir).join("epics"),
        EntityKind::Story => Path::new(&st.specs_dir).join("stories"),
        EntityKind::Adr => Path::new(&st.specs_dir).join("adrs"),
        EntityKind::Hazard => Path::new(&st.specs_dir).join("hazards"),
        EntityKind::Sprint => Path::new(&st.state_dir).join("sprints"),
        EntityKind::Release => Path::new(&st.state_dir).join("releases"),
        EntityKind::DeferredWork => Path::new(&st.state_dir).join("dw"),
        EntityKind::Decision => Path::new(&st.state_dir).join("decisions"),
        EntityKind::Scratchpad => Path::new(&st.state_dir).join("scratch"),
        EntityKind::Soup => Path::new(&st.state_dir).join("soup"),
        EntityKind::Evidence => Path::new(&st.state_dir).join("evidence"),
    }
}

/// Does `file_name` carry `id`, i.e. is it `<id>.md`, `<id>-<slug>.md` or `<id>_<slug>.md`
/// (case-insensitively)? This is the filename half of the one identity rule stated on
/// [`resolve_entity_file`], and the single place that rule is spelled out: `find_file_in_dir`
/// resolves writes with it and `qdev validate` reports a file that fails it.
///
/// **The id and the extension are both matched case-insensitively**, so `e1s1.md`, `E1S1.MD`
/// and `E1S1-buffer.Md` all carry `E1S1`. The extension half was once literal, which made
/// resolution disagree with occupancy ([`id_carried_by_filename`] has always matched it
/// case-insensitively) and, worse, made the answer depend on the host filesystem: the deleted
/// `.is_file()` probe in `find_file_in_dir` resolved `E1S7.MD` on macOS and Windows while the
/// rule said it carried nothing, and Linux disagreed with both. Extension case was never the
/// part of a name that made it unresolvable, so it is now legal rather than a repairable defect
/// — `<id>.md` is still the only spelling any writer *creates*.
pub fn filename_carries_id(file_name: &str, id: &str) -> bool {
    let Some((stem, ext)) = file_name.rsplit_once('.') else {
        return false;
    };
    if !ext.eq_ignore_ascii_case("md") {
        return false;
    }
    let stem_lower = stem.to_ascii_lowercase();
    let id_lower = id.to_ascii_lowercase();
    stem_lower == id_lower
        || stem_lower.starts_with(&format!("{}-", id_lower))
        || stem_lower.starts_with(&format!("{}_", id_lower))
}

/// The inverse of [`filename_carries_id`]: the id `file_name` carries, in canonical spelling,
/// or `None` if the name carries no id at all. This is the filename half of the in-use id set
/// ([`crate::validate::ids_in_use`]) — a file *occupies* the id its name carries whether or not
/// its frontmatter declares it, and a file whose frontmatter will not parse still does.
///
/// Cut points are tried at each `-`/`_` boundary shortest-first, then the whole stem, because an
/// id may itself contain `-` (`AD-7`, `FR-101`, `DW-7f3a`): splitting on the first separator
/// alone reads `AD-7-context.md` as carrying no id. A candidate is accepted only if it parses as
/// an [`Identifier`] — otherwise every `README.md` would claim an id — and the canonical
/// `to_string()` is returned so a differently cased or zero-padded name (`e1s1.md`,
/// `E12S01.md`) reports the id it actually occupies.
/// `dw-7f3a` -> `DW-7f3a`: the prefix uppercased, the hex suffix left lowercase, which is the
/// only spelling `validate_hex_hash` accepts.
fn recase_hex_id(candidate: &str) -> String {
    match candidate.split_once('-') {
        Some((prefix, rest)) => format!(
            "{}-{}",
            prefix.to_ascii_uppercase(),
            rest.to_ascii_lowercase()
        ),
        None => candidate.to_ascii_uppercase(),
    }
}

pub fn id_carried_by_filename(file_name: &str) -> Option<String> {
    // The extension is matched case-insensitively, like hydration's own walk — and now like
    // [`filename_carries_id`] too, so occupancy and resolution give one answer on this axis
    // instead of two. Requiring lowercase here left a hole in the intersection of two matrix
    // rows — a `.MD` file whose frontmatter will not parse was in neither half of the union, so
    // allocation handed out its id and `create_story` then refused with `file_exists` for an id
    // the user never chose. Widening resolution to match (2026-09-11) removed the remaining
    // asymmetry rather than this one: nothing about *which ids a workspace owns* changed.
    let (stem, ext) = file_name.rsplit_once('.')?;
    if !ext.eq_ignore_ascii_case("md") {
        return None;
    }
    let mut cuts: Vec<usize> = stem
        .char_indices()
        .filter(|(_, c)| *c == '-' || *c == '_')
        .map(|(idx, _)| idx)
        .collect();
    cuts.push(stem.len());
    for cut in cuts {
        let candidate = &stem[..cut];
        if candidate.is_empty() {
            continue;
        }
        if let Ok(identifier) = candidate.parse::<Identifier>() {
            return Some(identifier.to_string());
        }
        // Both canonical cases are tried, because the grammar is mixed: planning ids are
        // uppercase (`E1S1`, `AD-7`) while the hex suffixes of `DW-`/`DEC-` ids are lowercase
        // and `validate_hex_hash` rejects uppercase. Trying only the upper form left
        // `dw-7f3a.md` carrying no id at all, while the write path resolves it for `DW-7f3a`.
        for recased in [candidate.to_ascii_uppercase(), recase_hex_id(candidate)] {
            if recased != candidate {
                if let Ok(identifier) = recased.parse::<Identifier>() {
                    return Some(identifier.to_string());
                }
            }
        }
        if let Some(identifier) = crate::id::story_id_from_lenient_filename(candidate) {
            return Some(identifier.to_string());
        }
    }
    None
}

/// The canonical file name for an entity: `<id>.md`. Reported as the expected name by
/// `qdev validate`'s off-convention check, and the rename target `--fix-ids` writes to.
pub fn canonical_file_name(id: &str) -> String {
    format!("{}.md", id)
}

/// The file name an entity's file must take when its id changes from `old_id` to `new_id`, so
/// the renamed file still carries its id and stays resolvable by [`resolve_entity_file`].
///
/// A `-slug`/`_slug` suffix is preserved (`E1S1-buffer-layout.md` -> `E1S2-buffer-layout.md`);
/// the id part is replaced with `new_id` as written, so the new name is canonically cased even
/// when the old one was not. A name that does not carry `old_id` at all cannot have a suffix
/// identified, so it becomes the canonical `<new_id>.md`.
///
/// Everything after the id is preserved, which includes the extension's case: `E1S7.MD`
/// renumbers to `E1S8.MD`. That is deliberate and consistent rather than a gap — the extension
/// is matched case-insensitively by [`filename_carries_id`], so `E1S8.MD` is a name the write
/// path resolves and `qdev validate` does not complain about, exactly like the slug it sits
/// beside.
pub fn renamed_file_name(current_name: &str, old_id: &str, new_id: &str) -> String {
    if filename_carries_id(current_name, old_id) {
        let suffix = current_name.get(old_id.len()..).unwrap_or("");
        format!("{}{}", new_id, suffix)
    } else {
        canonical_file_name(new_id)
    }
}

/// The write path's "which file holds this id?" question, for callers outside this module.
///
/// Answers with the one file in `dir` whose name carries `id` per the identity rule, `None` if
/// there is none, and a usage error naming every candidate if more than one does — which is what
/// makes it the right pre-flight check before renaming a file *to* an id.
pub fn find_file_in_dir_for_id(dir: &Path, id: &str) -> Result<Option<PathBuf>, QdevError> {
    find_file_in_dir(dir, id)
}

/// Every entry in `dir` judged by [`filename_carries_id`], and nothing else.
///
/// **The filesystem is never asked to resolve a spelling.** This used to short-circuit on
/// `dir.join("<id>.md").is_file()`, which delegates the question to the OS — and on a
/// case-insensitive filesystem the OS answers for spellings the rule does not admit. A lone
/// `e1s1.md` was then found twice, once as the probe's `E1S1.md` and once as itself, so
/// `qdev update E1S1` refused with "Multiple entity files match" naming a file that does not
/// exist; and `E1S7.MD` resolved on macOS and Windows but not on Linux. Listing the directory
/// and judging each real name gives one answer on every host, and two matches mean two files
/// because they are two directory entries.
/// If `fs::read_dir` fails on an existing directory, that failure is surfaced as an
/// `infrastructure_failure` (`io_error`), rather than being swallowed into "no file holds this
/// id".
fn find_file_in_dir(dir: &Path, id: &str) -> Result<Option<PathBuf>, QdevError> {
    match dir.try_exists() {
        Ok(false) => return Ok(None),
        Ok(true) => {}
        Err(e) => {
            return Err(QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to access directory '{}': {}", dir.display(), e),
            ));
        }
    }
    let entries = fs::read_dir(dir).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to read directory '{}': {}", dir.display(), e),
        )
    })?;
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to read directory entry in '{}': {}",
                    dir.display(),
                    e
                ),
            )
        })?;
        let path = entry.path();
        if path.is_file() {
            if let Some(name) = entry.file_name().to_str() {
                if filename_carries_id(name, id) {
                    matches.push(path);
                }
            }
        }
    }
    // `read_dir` yields entries in filesystem order, so two genuine matches must be reported in
    // a stable order for the refusal to be reproducible.
    matches.sort();

    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        _ => Err(QdevError::usage_error(format!(
            "Multiple entity files match ID '{}' in '{}': {:?}",
            id,
            dir.display(),
            matches
                .into_iter()
                .map(|p| p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string())
                .collect::<Vec<_>>()
        ))),
    }
}

/// The refusal [`resolve_entity_file`] returns when the rule resolves no file for `id`.
///
/// "Entity file not found for 'E1S9'" is true but useless when `docs/specs/stories/notes.md`
/// plainly holds `E1S9` and `qdev get E1S9` just returned it: the id *is* in the workspace, under
/// a name the rule does not resolve. So the not-found path asks
/// [`crate::validate::off_convention_file_holding_id`] — which is where `qdev validate`'s
/// `entity_file_off_convention` warning is worded too — and appends the file and the rename that
/// would fix it. When nothing holds the id, the bare message is the whole truth and stands.
fn entity_file_not_found(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    id: &str,
) -> QdevError {
    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    match crate::validate::off_convention_file_holding_id(workspace_root, st, id) {
        Some(explanation) => QdevError::usage_error(format!(
            "Entity file not found for '{}': {}",
            id, explanation
        )),
        None => QdevError::usage_error(format!("Entity file not found for '{}'", id)),
    }
}

/// Resolves an entity file path and entity kind given workspace root, optional kind, ID, and
/// optional storage config. **The single entry point every writer uses** — no caller grows its
/// own lookup.
///
/// # The one identity rule
///
/// **A file is named for the entity it holds.** An entity with frontmatter `id: X` lives in
/// `<specs_dir|state_dir>/<kind-dir>/` (see [`directory_for_kind`]) in a file named `X.md`,
/// `X-<slug>.md` or `X_<slug>.md` (see [`filename_carries_id`]). Reads answer "which file is
/// entity X?" from frontmatter `id` and remember the path; writes answer it from the file name,
/// which under this convention is the same file — so the write path needs no cache dependency
/// and `qdev create story` keeps working outside an initialised workspace. Resolution itself
/// reads exactly one directory; only the *refusal* below walks the spec and state trees, once,
/// to name the file that holds the id — an error path, never the answer path.
///
/// The convention is enforced, not assumed: `qdev validate` reports a `warning`-severity
/// `entity_file_off_convention` finding for every hydrated entity whose file breaks either half
/// of it, and `qdev validate --fix-ids` renames as it renumbers so a renumbered entity is
/// immediately writable. A file that breaks it is still readable (hydration walks the spec and
/// state trees recursively) but may not be resolvable here; the finding names it and the name it
/// should have.
///
/// Ambiguity is never a guess: two files matching one id produce a usage error naming both.
pub fn resolve_entity_file(
    workspace_root: &Path,
    kind_opt: Option<EntityKind>,
    id_str: &str,
    storage: Option<&StorageConfig>,
) -> Result<(EntityKind, String, PathBuf), QdevError> {
    let trimmed_id = id_str.trim();
    if trimmed_id.is_empty() {
        return Err(QdevError::usage_error("Entity ID cannot be empty"));
    }
    if trimmed_id.contains('/') || trimmed_id.contains('\\') || trimmed_id.contains("..") {
        return Err(QdevError::usage_error(format!(
            "Invalid entity ID '{}': cannot contain path separators or parent directory references",
            trimmed_id
        )));
    }

    if let Some(kind) = kind_opt {
        let rel_dir = directory_for_kind(storage, kind);
        let dir = workspace_root.join(rel_dir);
        if let Some(path) = find_file_in_dir(&dir, trimmed_id)? {
            return Ok((kind, trimmed_id.to_string(), path));
        }
        return Err(entity_file_not_found(workspace_root, storage, trimmed_id));
    }

    // No kind passed: infer kind from identifier grammar
    if let Ok(identifier) = trimmed_id.parse::<Identifier>() {
        let kind = match identifier.kind() {
            IdentifierKind::Epic => EntityKind::Epic,
            IdentifierKind::Story => EntityKind::Story,
            IdentifierKind::Adr => EntityKind::Adr,
            IdentifierKind::FunctionalRequirement | IdentifierKind::NonFunctionalRequirement => {
                EntityKind::Requirement
            }
            IdentifierKind::Hazard => EntityKind::Hazard,
            IdentifierKind::Prd => EntityKind::Prd,
            IdentifierKind::DeferredWork => EntityKind::DeferredWork,
            IdentifierKind::Decision => EntityKind::Decision,
            IdentifierKind::Constraint => {
                return Err(QdevError::usage_error(format!(
                    "Constraint '{}' is embedded inside its owning entity file",
                    trimmed_id
                )));
            }
        };

        let dir = workspace_root.join(directory_for_kind(storage, kind));
        if let Some(path) = find_file_in_dir(&dir, trimmed_id)? {
            return Ok((kind, trimmed_id.to_string(), path));
        }
    }

    // Handle sprint-n, release versions, etc.
    if trimmed_id.starts_with("sprint-") {
        let default_storage = StorageConfig::default();
        let st = storage.unwrap_or(&default_storage);
        let dir = workspace_root.join(&st.state_dir).join("sprints");
        if let Some(path) = find_file_in_dir(&dir, trimmed_id)? {
            return Ok((EntityKind::Sprint, trimmed_id.to_string(), path));
        }
    }

    // Fallback: search across all 13 standard directories and check for ambiguity
    let mut fallback_matches = Vec::new();
    for &candidate_kind in EntityKind::all() {
        let dir = workspace_root.join(directory_for_kind(storage, candidate_kind));
        if let Some(path) = find_file_in_dir(&dir, trimmed_id)? {
            fallback_matches.push((candidate_kind, trimmed_id.to_string(), path));
        }
    }

    match fallback_matches.len() {
        0 => Err(entity_file_not_found(workspace_root, storage, trimmed_id)),
        1 => Ok(fallback_matches.remove(0)),
        _ => Err(QdevError::usage_error(format!(
            "Ambiguous entity ID '{}' matches multiple entity kinds: {:?}",
            trimmed_id,
            fallback_matches
                .iter()
                .map(|(k, _, p)| format!("{}: {}", k.as_str(), p.display()))
                .collect::<Vec<_>>()
        ))),
    }
}

/// The kind a writer validates against, resolved the way hydration resolves it.
///
/// [`resolve_entity_file`] answers "which file?" from the file name; it must not also be the
/// authority on "which kind?", because it reads the directory and the identifier grammar while
/// hydration prefers frontmatter `kind:`. A writer that trusted it validated against one schema
/// and the next boot sweep validated the file it had just written against another, recording an
/// error-severity `schema_violation` on a write that had exited 0.
///
/// So kind collapses onto [`crate::store::determine_entity_kind`] — hydration's own rule —
/// applied to the content this write is about to leave on disk, which is exactly what the next
/// sweep will read. `resolved_kind` is the fallback for content whose frontmatter cannot be
/// extracted at all (the caller reports that failure on its own terms).
pub fn kind_for_write(file_path: &Path, content: &str, resolved_kind: EntityKind) -> EntityKind {
    let frontmatter = match crate::schema::extract_frontmatter(content) {
        Ok(v) => v,
        Err(_) => return resolved_kind,
    };
    let id = frontmatter
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    crate::store::determine_entity_kind(file_path, id, &frontmatter)
}

/// Options for applying an entity update.
#[derive(Debug, Clone)]
pub struct EntityUpdateOptions {
    pub workspace_root: PathBuf,
    pub storage: Option<StorageConfig>,
    pub entity_kind: Option<EntityKind>,
    pub entity_id: String,
    pub status: Option<String>,
    pub title: Option<String>,
    pub custom_fields: Vec<(String, serde_yaml::Value)>,
    pub section: Option<String>,
    pub section_file: Option<PathBuf>,
    pub if_version: Option<u64>,
    pub author: Author,
}

/// Result returned from applying an entity update.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EntityUpdateResult {
    pub id: String,
    pub kind: EntityKind,
    pub path: PathBuf,
    pub rel_path: String,
    pub old_version: u64,
    pub new_version: u64,
    pub updated_frontmatter: serde_json::Value,
}

/// High-level write path engine orchestrating locking, patching, atomic writes, and cache sync.
pub fn apply_entity_update(options: &EntityUpdateOptions) -> Result<EntityUpdateResult, QdevError> {
    options.author.validate()?;

    // Verify that at least one modification is requested
    let has_frontmatter_update =
        options.status.is_some() || options.title.is_some() || !options.custom_fields.is_empty();
    let has_section_update = options.section.is_some();

    if !has_frontmatter_update && !has_section_update {
        return Err(QdevError::usage_error(
            "No fields or section specified to update",
        ));
    }

    // Verify section options consistency
    let section_content = if let Some(ref heading) = options.section {
        let file_path = options
            .section_file
            .as_ref()
            .ok_or_else(|| QdevError::usage_error("--section requires --file to be specified"))?;
        let content = fs::read_to_string(file_path).map_err(|e| {
            QdevError::usage_error(format!(
                "Failed to read section file '{}': {}",
                file_path.display(),
                e
            ))
        })?;
        Some((heading.clone(), content))
    } else {
        if options.section_file.is_some() {
            return Err(QdevError::usage_error(
                "--file requires --section to be specified",
            ));
        }
        None
    };

    // 1. Resolve entity file
    let (kind, id, file_path) = resolve_entity_file(
        &options.workspace_root,
        options.entity_kind,
        &options.entity_id,
        options.storage.as_ref(),
    )?;

    let rel_path = workspace_rel_path(&file_path, &options.workspace_root);

    // 2. Acquire advisory write lock on write.lock with 5s timeout
    let cache_dir_rel = options
        .storage
        .as_ref()
        .map(|s| s.cache_dir.as_str())
        .unwrap_or(".qdev/cache");
    let lock_path = options
        .workspace_root
        .join(cache_dir_rel)
        .join("write.lock");
    let _lock_guard = acquire_write_lock(&lock_path, Duration::from_millis(5000))?;

    // 3. Read existing file content
    let existing_content = fs::read_to_string(&file_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to read entity file '{}': {}",
                file_path.display(),
                e
            ),
        )
    })?;

    // The version this reports must be the one `patch_frontmatter` fenced and bumped against, so
    // it comes from the same reader — see `frontmatter_version`. Reading it a second way here
    // meant one call could return `old_version: 3` for a write that computed `version: 1`.
    let old_version = frontmatter_version(&existing_content).unwrap_or(0);

    // 4. Line-based frontmatter patch
    let patch_opts = FrontmatterPatchOptions {
        status: options.status.clone(),
        title: options.title.clone(),
        custom_fields: options.custom_fields.clone(),
        author: Some(options.author.clone()),
        if_version: options.if_version,
    };

    let (mut patched_content, new_version) = patch_frontmatter(&existing_content, &patch_opts)?;

    // 5. Replace markdown section if requested
    if let Some((heading, new_body)) = section_content {
        patched_content = replace_markdown_section(&patched_content, &heading, &new_body)?;
    }

    // 6. Validate updated frontmatter against JSON Schema, against the kind hydration will
    // resolve for this file once it is written (frontmatter `kind:` first) — not the kind the
    // filename lookup happened to use, so this write and the next sweep agree.
    let kind = kind_for_write(&file_path, &patched_content, kind);
    validate_frontmatter(kind, &patched_content).map_err(|errs| {
        QdevError::logical_failure(
            "schema_validation_failed",
            format!("Updated frontmatter failed schema validation: {:?}", errs),
        )
        .with_details(serde_json::json!({
            "validation_errors": errs,
        }))
    })?;

    // 7. Atomic write via tempfile rename
    write_file_atomic(&file_path, &patched_content)?;

    // 8. Extract updated frontmatter for cache and envelope
    let updated_frontmatter =
        crate::schema::extract_frontmatter(&patched_content).map_err(|e| {
            QdevError::infrastructure_failure(
                "parse_error",
                format!("Failed to parse updated frontmatter: {}", e),
            )
        })?;

    // Use canonical entity ID from parsed frontmatter if available
    let canonical_id = updated_frontmatter
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or(id);

    // 9. Upsert cache and mark dirty
    let content_hash = sha256_digest(patched_content.as_bytes());
    let title_val = updated_frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let status_val = updated_frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let owners_val = updated_frontmatter.get("owners").map(|v| v.to_string());

    let c_author = updated_frontmatter
        .get("created_by")
        .and_then(|v| serde_json::from_value::<Author>(v.clone()).ok());
    let u_author = Some(options.author.clone());

    // Story fields
    let (epic_id, seq, appetite, safety_class, target_modules) =
        story_detail_fields(kind, &canonical_id, &updated_frontmatter);

    let record = EntityRecord {
        id: canonical_id.clone(),
        kind,
        title: title_val,
        status: status_val,
        owners: owners_val,
        source_path: rel_path.clone(),
        content_hash,
        version: new_version,
        created_by: c_author,
        updated_by: u_author,
        updated_at: current_iso8601(),
        stale: false,
        epic_id,
        seq,
        appetite,
        safety_class,
        target_modules,
    };

    let cache_db_path = options
        .workspace_root
        .join(cache_dir_rel)
        .join("cache.sqlite");
    upsert_cache_and_mark_dirty(&cache_db_path, &record)?;

    Ok(EntityUpdateResult {
        id: canonical_id,
        kind,
        path: file_path,
        rel_path,
        old_version,
        new_version,
        updated_frontmatter,
    })
}

/// Derives the `stories` table detail fields (`epic_id`, `seq`, `appetite`, `safety_class`,
/// `target_modules`) from updated frontmatter, falling back to the id's own grammar for
/// `epic_id`/`seq` when the frontmatter omits them. Returns all-`None` for non-`Story` kinds.
/// Shared by `apply_entity_update` and `apply_relation_change` so both agree on story details.
/// `(epic_id, seq, appetite, safety_class, target_modules)`.
type StoryDetailFields = (
    Option<String>,
    Option<u32>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn story_detail_fields(
    kind: EntityKind,
    canonical_id: &str,
    updated_frontmatter: &serde_json::Value,
) -> StoryDetailFields {
    if kind != EntityKind::Story {
        return (None, None, None, None, None);
    }

    let epic = updated_frontmatter
        .get("epic_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| {
            if let Ok(Identifier::Story { epic, .. }) = canonical_id.parse::<Identifier>() {
                Some(format!("E{}", epic))
            } else {
                None
            }
        });
    let s_num = if let Ok(Identifier::Story { story, .. }) = canonical_id.parse::<Identifier>() {
        Some(story)
    } else {
        None
    };
    let app = updated_frontmatter
        .get("appetite")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let safe = updated_frontmatter
        .get("safety_class")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mods = updated_frontmatter
        .get("target_modules")
        .map(|v| v.to_string());
    (epic, s_num, app, safe, mods)
}

/// The body a newly created story starts with. Redesigning this template is out of scope here.
const STORY_BODY_TEMPLATE: &str = "\n## Acceptance Criteria\n";

/// Free-text frontmatter keys that are always emitted double-quoted.
///
/// Everything else goes through `serde_yaml`'s own emitter, which quotes only when the value
/// would otherwise be re-read as a different YAML type. A title is arbitrary user text, so it is
/// quoted unconditionally: that is both safer and the shape every existing reader and golden
/// test already expects.
const ALWAYS_QUOTED_KEYS: &[&str] = &["title"];

/// Options for creating a new story file (`qdev create story`).
///
/// The story id is allocated by the caller (`allocate_next_story_id_in`) and passed in, so id
/// allocation stays where the CLI can report it; everything from building the frontmatter to
/// syncing the cache happens here, under the advisory lock.
#[derive(Debug, Clone)]
pub struct StoryCreateOptions {
    pub workspace_root: PathBuf,
    pub storage: Option<StorageConfig>,
    /// Pre-allocated canonical story id, e.g. `E12S1`.
    pub story_id: String,
    pub title: Option<String>,
    pub appetite: Option<String>,
    pub safety_class: Option<String>,
    pub target_modules: Vec<String>,
    pub owners: Vec<String>,
    pub author: Author,
}

/// Result of creating a story.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoryCreateResult {
    pub id: String,
    /// Absolute path written, for callers that need to open or report the file.
    pub path: PathBuf,
    /// Workspace-relative path, which is what the CLI payload reports.
    pub rel_path: String,
}

/// Builds the story frontmatter as a `serde_yaml` mapping in the canonical field order:
/// `id`, `title`, `status`, the optional `appetite` / `safety_class` / `target_modules` /
/// `owners`, then `version`, `created_by`, `updated_by`. A `Mapping` iterates in insertion
/// order, so the order declared here is the order written to disk.
fn build_story_frontmatter(options: &StoryCreateOptions) -> serde_yaml::Mapping {
    fn author_mapping(author: &Author) -> serde_yaml::Value {
        let mut m = serde_yaml::Mapping::new();
        m.insert(
            serde_yaml::Value::String("type".to_string()),
            serde_yaml::Value::String(author.author_type.clone()),
        );
        m.insert(
            serde_yaml::Value::String("id".to_string()),
            serde_yaml::Value::String(author.id.clone()),
        );
        serde_yaml::Value::Mapping(m)
    }

    fn string_sequence(values: &[String]) -> serde_yaml::Value {
        serde_yaml::Value::Sequence(
            values
                .iter()
                .map(|v| serde_yaml::Value::String(v.clone()))
                .collect(),
        )
    }

    let mut fm = serde_yaml::Mapping::new();
    let mut put = |key: &str, value: serde_yaml::Value| {
        fm.insert(serde_yaml::Value::String(key.to_string()), value);
    };

    put(
        "id",
        serde_yaml::Value::String(options.story_id.trim().to_string()),
    );
    put(
        "title",
        serde_yaml::Value::String(options.title.clone().unwrap_or_default()),
    );
    put("status", serde_yaml::Value::String("draft".to_string()));
    if let Some(ref appetite) = options.appetite {
        put("appetite", serde_yaml::Value::String(appetite.clone()));
    }
    if let Some(ref safety_class) = options.safety_class {
        put(
            "safety_class",
            serde_yaml::Value::String(safety_class.clone()),
        );
    }
    if !options.target_modules.is_empty() {
        put("target_modules", string_sequence(&options.target_modules));
    }
    if !options.owners.is_empty() {
        put("owners", string_sequence(&options.owners));
    }
    put("version", serde_yaml::Value::Number(1.into()));
    put("created_by", author_mapping(&options.author));
    put("updated_by", author_mapping(&options.author));
    fm
}

/// Emits a single scalar the way `serde_yaml` itself would, quoting only when the plain form
/// would be re-read as some other type.
fn render_yaml_scalar(value: &serde_yaml::Value) -> Result<String, QdevError> {
    let rendered = serde_yaml::to_string(value).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialize_error",
            format!("Failed to serialize frontmatter scalar: {}", e),
        )
    })?;
    let rendered = rendered.trim_end_matches('\n').to_string();

    // `serde_yaml` renders a string containing a newline as a multi-line block scalar (`|-`
    // followed by indented lines). Every caller splices this into one `key: value` line, so a
    // block scalar would be spliced into a document that no longer parses. A JSON-quoted string
    // is always a valid single-line YAML scalar, so fall back to that instead of corrupting the
    // document. Callers that own audited fields reject control characters outright — see
    // `Author::validate`; this is the generic backstop for every other scalar.
    if rendered.contains('\n') {
        return serde_json::to_string(value).map_err(|e| {
            QdevError::infrastructure_failure(
                "serialize_error",
                format!("Failed to serialize multi-line frontmatter scalar: {}", e),
            )
        });
    }

    Ok(rendered)
}

/// Renders a frontmatter mapping plus a Markdown body into a complete document.
///
/// Sequences are emitted in flow style and free-text keys double-quoted, which is the shape the
/// rest of the tree already reads; nested mappings (`created_by` / `updated_by`) are emitted as
/// two-space-indented blocks.
fn render_frontmatter_document(
    frontmatter: &serde_yaml::Mapping,
    body: &str,
) -> Result<String, QdevError> {
    let mut out = String::from("---\n");

    for (key, value) in frontmatter {
        let key_str = key.as_str().ok_or_else(|| {
            QdevError::infrastructure_failure("serialize_error", "Frontmatter keys must be strings")
        })?;

        match value {
            serde_yaml::Value::Sequence(_) => {
                let json = serde_json::to_value(value).map_err(|e| {
                    QdevError::infrastructure_failure(
                        "serialize_error",
                        format!("Failed to serialize '{}' sequence: {}", key_str, e),
                    )
                })?;
                out.push_str(&format!("{}: {}\n", key_str, json));
            }
            serde_yaml::Value::Mapping(nested) => {
                out.push_str(&format!("{}:\n", key_str));
                for (nested_key, nested_value) in nested {
                    let nested_key_str = nested_key.as_str().ok_or_else(|| {
                        QdevError::infrastructure_failure(
                            "serialize_error",
                            "Frontmatter keys must be strings",
                        )
                    })?;
                    out.push_str(&format!(
                        "  {}: {}\n",
                        nested_key_str,
                        render_yaml_scalar(nested_value)?
                    ));
                }
            }
            serde_yaml::Value::String(s) if ALWAYS_QUOTED_KEYS.contains(&key_str) => {
                let quoted = serde_json::to_string(s).map_err(|e| {
                    QdevError::infrastructure_failure(
                        "serialize_error",
                        format!("Failed to serialize '{}': {}", key_str, e),
                    )
                })?;
                out.push_str(&format!("{}: {}\n", key_str, quoted));
            }
            _ => {
                out.push_str(&format!("{}: {}\n", key_str, render_yaml_scalar(value)?));
            }
        }
    }

    out.push_str("---\n");
    out.push_str(body);
    Ok(out)
}

/// Creates a new story file through the same write path every other mutation uses: build the
/// frontmatter, validate it against the story schema, take the advisory lock, confirm the
/// destination is free, write atomically, then upsert the cache row and mark it dirty.
///
/// The destination check happens **inside** the lock on purpose. The old hand-rolled
/// implementation got its "already exists" refusal for free from `OpenOptions::create_new`;
/// `write_file_atomic` renames over its destination and would happily clobber an existing story,
/// so the exclusivity is re-established explicitly here. Checking before taking the lock would
/// be a race.
pub fn create_story(options: &StoryCreateOptions) -> Result<StoryCreateResult, QdevError> {
    options.author.validate()?;

    let story_id = options.story_id.trim();
    match story_id.parse::<Identifier>() {
        Ok(Identifier::Story { .. }) => {}
        _ => {
            return Err(QdevError::usage_error(format!(
                "'{}' is not a story identifier (expected E<n>S<m>)",
                story_id
            )))
        }
    }

    // 1. Frontmatter as a mapping, rendered into the document that will be written verbatim.
    let frontmatter_mapping = build_story_frontmatter(options);
    let content = render_frontmatter_document(&frontmatter_mapping, STORY_BODY_TEMPLATE)?;

    // 2. Validate the exact bytes destined for disk *before* anything is written, so a schema
    //    violation leaves no file behind.
    validate_frontmatter_detailed(EntityKind::Story, &content).map_err(|errs| {
        let fields: Vec<String> = errs
            .iter()
            .map(|e| {
                if e.path.is_empty() {
                    e.message.clone()
                } else {
                    format!("{}: {}", e.path, e.message)
                }
            })
            .collect();
        QdevError::logical_failure(
            "schema_validation_failed",
            format!(
                "Generated story frontmatter failed schema validation: {}",
                fields.join("; ")
            ),
        )
        .with_details(serde_json::json!({
            "validation_errors": errs,
        }))
    })?;

    // The name is spelled by `canonical_file_name`, never by a second `format!` here: this is
    // the same rule `resolve_entity_file` resolves by, and a story created under a name only
    // this function knows how to build is a story no writer can find.
    let rel_dir = directory_for_kind(options.storage.as_ref(), EntityKind::Story);
    let file_name = canonical_file_name(story_id);
    let rel_path = format!(
        "{}/{}",
        rel_dir.to_string_lossy().replace('\\', "/"),
        file_name
    );
    let abs_path = options.workspace_root.join(&rel_dir).join(&file_name);

    // 3. Advisory write lock, same file and timeout as every other write.
    let cache_dir_rel = options
        .storage
        .as_ref()
        .map(|s| s.cache_dir.as_str())
        .unwrap_or(".qdev/cache");
    let lock_path = options
        .workspace_root
        .join(cache_dir_rel)
        .join("write.lock");
    // The lock is skipped only when the cache directory does not exist at all, which means this
    // is not an initialized workspace: there is no other qdev writer to serialize against, and
    // `acquire_write_lock` would otherwise create a stray `.qdev/cache/write.lock` tree in an
    // unrelated directory. In every real workspace `qdev init` has created the directory, so the
    // lock is always taken.
    let _lock_guard = if lock_path
        .parent()
        .map(|parent| parent.is_dir())
        .unwrap_or(false)
    {
        Some(acquire_write_lock(&lock_path, Duration::from_millis(5000))?)
    } else {
        None
    };

    // 4. Under the lock: the destination must be free. `symlink_metadata` answers for a file, a
    //    directory or a dangling symlink alike, so occupancy never depends on an error kind.
    if abs_path.symlink_metadata().is_ok() {
        return Err(QdevError::conflict(
            "file_exists",
            format!("Story path already exists: {}", abs_path.display()),
        )
        .with_details(serde_json::json!({
            "id": story_id,
            "path": rel_path,
        })));
    }

    // 5. Atomic write.
    write_file_atomic(&abs_path, &content)?;

    // 6. Cache upsert and dirty mark, so the story is queryable without waiting for a sweep.
    let frontmatter = crate::schema::extract_frontmatter(&content).map_err(|e| {
        QdevError::infrastructure_failure(
            "parse_error",
            format!("Failed to parse generated frontmatter: {}", e),
        )
    })?;

    let (epic_id, seq, appetite, safety_class, target_modules) =
        story_detail_fields(EntityKind::Story, story_id, &frontmatter);

    let record = EntityRecord {
        id: story_id.to_string(),
        kind: EntityKind::Story,
        title: frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        status: frontmatter
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        owners: frontmatter.get("owners").map(|v| v.to_string()),
        source_path: rel_path.clone(),
        content_hash: sha256_digest(content.as_bytes()),
        version: 1,
        created_by: Some(options.author.clone()),
        updated_by: Some(options.author.clone()),
        updated_at: current_iso8601(),
        stale: false,
        epic_id,
        seq,
        appetite,
        safety_class,
        target_modules,
    };

    // Only upsert into a cache that already exists. In a real workspace it always does — boot
    // runs `ensure_cache` before any command — so the "queryable without a sweep" guarantee is
    // unaffected. Outside a workspace, `upsert_cache_and_mark_dirty` would *create* a 16-table,
    // unstamped `cache.sqlite` in a directory that is not a workspace, which a later `qdev init`
    // reads as a v0 cache and migrates: it would drop and rebuild the rows this write had just
    // put there, for a file nothing had asked for. The file on disk is the source of truth; the
    // next boot hydrates it.
    let cache_db_path = options
        .workspace_root
        .join(cache_dir_rel)
        .join("cache.sqlite");
    if cache_db_path.is_file() {
        upsert_cache_and_mark_dirty(&cache_db_path, &record)?;
    }

    Ok(StoryCreateResult {
        id: story_id.to_string(),
        path: abs_path,
        rel_path,
    })
}

/// The `version:` value a document's frontmatter declares, or `None` when it declares none, or
/// declares one that is not a whole number. The single reader of that field on the write path:
/// `patch_frontmatter` fences `--if-version` against it, and `apply_relation_change` reads it
/// before it can report an idempotent no-op, so the two cannot disagree about what version a
/// file is at. Only an unindented, non-comment `version:` line counts, matching the top-level
/// key blocks `patch_frontmatter` rewrites.
pub(crate) fn frontmatter_version(content: &str) -> Option<u64> {
    let (frontmatter, _) = crate::schema::extract_frontmatter_str(content).ok()?;

    // YAML first, exactly as hydration reads it. A decimal-only scan is not enough: `0x03`,
    // `0o3` and `3_000` are all schema-valid YAML integers that `serde_yaml` — and therefore
    // `qdev get` — reports as numbers, and reading them as "no version" made `--if-version`
    // refuse the version the entity actually declares *and* an unfenced write reset the counter
    // to 1, rolling it backwards.
    if let Ok(serde_yaml::Value::Mapping(map)) =
        serde_yaml::from_str::<serde_yaml::Value>(frontmatter)
    {
        if let Some(version) = map
            .get(serde_yaml::Value::String("version".to_string()))
            .and_then(|v| v.as_u64())
        {
            return Some(version);
        }
    }

    // Fallback for frontmatter YAML cannot parse as a whole — a malformed key elsewhere in the
    // block must not hide a perfectly readable `version:` from the fence.
    for line in frontmatter.lines() {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.starts_with([' ', '\t', '#', '-']) {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once(':') else {
            continue;
        };
        if key.trim() != "version" {
            continue;
        }
        let candidate = rest
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['"', '\'']);
        return candidate.parse::<u64>().ok();
    }
    None
}

/// Compares an `--if-version` expectation (`expected`) against the version a file actually
/// declares (`existing`, from `frontmatter_version`), raising the one `version_mismatch`
/// conflict every mutating command reports. `Ok(())` when no expectation was given.
///
/// Every caller compares before it reports an outcome, no-ops included: an agent using
/// `--if-version` as a compare-and-swap fence reads exit 0 as "my expected version was current",
/// so a no-op that skipped the comparison would confirm a version it never looked at.
pub(crate) fn check_if_version(
    expected: Option<u64>,
    existing: Option<u64>,
) -> Result<(), QdevError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    match existing {
        Some(v) if v == expected => Ok(()),
        Some(actual) => Err(QdevError::conflict(
            "version_mismatch",
            format!("Version mismatch: expected {}, found {}", expected, actual),
        )
        .with_details(serde_json::json!({
            "expected_version": expected,
            "current_version": actual,
        }))),
        None => Err(QdevError::conflict(
            "version_mismatch",
            format!(
                "Version mismatch: expected {}, but entity has no version",
                expected
            ),
        )
        .with_details(serde_json::json!({
            "expected_version": expected,
            "current_version": serde_json::Value::Null,
        }))),
    }
}

/// Options for applying a relation change (`qdev relate` / `qdev unrelate`).
#[derive(Debug, Clone)]
pub struct RelationChangeOptions {
    pub workspace_root: PathBuf,
    pub storage: Option<StorageConfig>,
    pub entity_kind: Option<EntityKind>,
    /// The source entity's id (relations are declared in the source entity's frontmatter).
    pub entity_id: String,
    pub relation: String,
    pub target_id: String,
    /// `true` adds `target_id` to `relation`'s target list (`relate`); `false` removes it
    /// (`unrelate`).
    pub add: bool,
    /// Optimistic concurrency control, compared before any outcome is reported: a stale
    /// expectation is a `version_mismatch` conflict whether the requested change would have
    /// altered the file or turned out to be an idempotent no-op.
    pub if_version: Option<u64>,
    pub author: Author,
}

/// Result returned from applying a relation change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RelationChangeResult {
    pub id: String,
    pub kind: EntityKind,
    pub path: PathBuf,
    pub rel_path: String,
    pub old_version: u64,
    pub new_version: u64,
    /// `false` when the file already had the state asked for — `unrelate` of an absent entry, or
    /// `relate` of an edge already present: an idempotent no-op, nothing was written and
    /// `new_version == old_version`. Reaching this outcome still required `if_version` to match,
    /// so a `false` here never means the version went uncompared.
    pub changed: bool,
    /// The full `relations:` map after the change (or the unchanged map, for a no-op).
    pub relations: serde_json::Value,
}

/// Patches a source entity's `relations:` frontmatter map to add or remove one
/// `relation -> target_id` edge, merging into the existing map (other relations and other
/// targets of the same relation are preserved) rather than replacing it wholesale. Reuses
/// `apply_entity_update`'s primitives: file resolution, the advisory write lock, the line-based
/// `patch_frontmatter` engine, schema validation, the atomic write, and the cache upsert.
///
/// Callers (e.g. `qdev relate`) are responsible for pre-write validation (relation name,
/// kind-pair, dangling target, would-be cycle) — this function only merges and writes; hydration
/// is the backstop that catches relations edited outside `qdev`. It deliberately accepts any
/// relation name it is given, including one architecture.md §8 does not define, because this
/// function's job is to merge and write what it was asked for — the name enum lives at the
/// command surface, as `--author-type`'s does.
///
/// (An earlier version of this comment justified that with `qdev validate --fix-ids` rewriting
/// unrecognised names found in files. That is not reachable: `relations` is
/// `additionalProperties: false` in the entity schemas, so a file carrying an unknown relation
/// name is a `schema_violation` and never hydrates, and `--fix-ids` therefore never sees such an
/// edge. The placement stands on its own; the reason did not.)
///
/// `options.if_version` is compared before any outcome is reported, the no-op included. It is
/// then passed on to `patch_frontmatter`, which compares it again — redundant here, but that
/// second check is the *only* one on the `qdev update` path, so it stays.
pub fn apply_relation_change(
    options: &RelationChangeOptions,
) -> Result<RelationChangeResult, QdevError> {
    options.author.validate()?;

    if options.relation.trim().is_empty() {
        return Err(QdevError::usage_error("Relation name cannot be empty"));
    }
    if options.target_id.trim().is_empty() {
        return Err(QdevError::usage_error("Target entity ID cannot be empty"));
    }

    // 1. Resolve entity file
    let (kind, id, file_path) = resolve_entity_file(
        &options.workspace_root,
        options.entity_kind,
        &options.entity_id,
        options.storage.as_ref(),
    )?;

    let rel_path = workspace_rel_path(&file_path, &options.workspace_root);

    // 2. Acquire advisory write lock on write.lock with 5s timeout
    let cache_dir_rel = options
        .storage
        .as_ref()
        .map(|s| s.cache_dir.as_str())
        .unwrap_or(".qdev/cache");
    let lock_path = options
        .workspace_root
        .join(cache_dir_rel)
        .join("write.lock");
    let _lock_guard = acquire_write_lock(&lock_path, Duration::from_millis(5000))?;

    // 3. Read existing file content
    let existing_content = fs::read_to_string(&file_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to read entity file '{}': {}",
                file_path.display(),
                e
            ),
        )
    })?;

    // The kind this write validates against is hydration's, not the filename lookup's (see
    // `kind_for_write`). A relate/unrelate never edits `kind:`, so resolving it once from the
    // content just read also covers the idempotent no-op return below.
    let kind = kind_for_write(&file_path, &existing_content, kind);

    // Parse the raw frontmatter YAML directly (rather than through `schema::extract_frontmatter`,
    // whose `serde_json::Value` uses an unordered/sorted `Map`) so the `relations:` map's key
    // order — and the order of other relation kinds within it — survives the round trip. A parse
    // failure is propagated rather than swallowed: unlike `apply_entity_update`, this function
    // actively reconstructs and overwrites the whole `relations:` block from the parsed value, so
    // silently defaulting to an empty map here would drop every existing relation on write.
    let (frontmatter_str, _) =
        crate::schema::extract_frontmatter_str(&existing_content).map_err(|e| {
            QdevError::logical_failure(
                "missing_frontmatter",
                format!(
                    "Failed to locate frontmatter in '{}': {}",
                    file_path.display(),
                    e
                ),
            )
        })?;
    let old_frontmatter_yaml: serde_yaml::Value =
        serde_yaml::from_str(frontmatter_str).map_err(|e| {
            QdevError::logical_failure(
                "yaml_parse_error",
                format!(
                    "Failed to parse frontmatter YAML in '{}': {}",
                    file_path.display(),
                    e
                ),
            )
        })?;

    // The version this call reports and fences against, read through the same function
    // `patch_frontmatter` uses so a no-op and a write cannot disagree about it.
    let declared_version = frontmatter_version(&existing_content);
    let old_version = declared_version.unwrap_or(0);

    // `--if-version` is compared here, above every outcome this function can return — including
    // the idempotent no-op below, which used to return `Ok(changed: false)` before
    // `patch_frontmatter` ever saw the expectation, so `relate --if-version 99` on an existing
    // edge exited 0 and confirmed a version nothing had compared.
    check_if_version(options.if_version, declared_version)?;

    // 4. Merge into the existing relations map: read it, mutate only the one relation's target
    // list, and write the whole map back — never construct a fresh map that drops other keys.
    //
    // An unexpected shape is refused rather than defaulted away. This function reconstructs and
    // overwrites the whole `relations:` block, so falling back to an empty map (or silently
    // skipping non-string entries) would rewrite the file with the existing edges deleted and
    // report success — the same silent-drop class as swallowing the YAML parse error above.
    let mut relations_obj: serde_yaml::Mapping = match old_frontmatter_yaml.get("relations") {
        None | Some(serde_yaml::Value::Null) => serde_yaml::Mapping::new(),
        Some(serde_yaml::Value::Mapping(map)) => map.clone(),
        Some(_) => {
            return Err(QdevError::logical_failure(
                "unsupported_relations_shape",
                format!(
                    "Frontmatter 'relations' in '{}' is not a relation-name to target-list map",
                    file_path.display()
                ),
            ))
        }
    };

    let relation_key = serde_yaml::Value::String(options.relation.clone());
    let mut targets: Vec<String> = match relations_obj.get(&relation_key) {
        None | Some(serde_yaml::Value::Null) => Vec::new(),
        Some(serde_yaml::Value::Sequence(items)) => {
            let mut parsed = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(target) => parsed.push(target.to_string()),
                    None => {
                        return Err(QdevError::logical_failure(
                            "unsupported_relations_shape",
                            format!(
                                "Relation '{}' in '{}' has a non-string target entry",
                                options.relation,
                                file_path.display()
                            ),
                        ))
                    }
                }
            }
            parsed
        }
        Some(_) => {
            return Err(QdevError::logical_failure(
                "unsupported_relations_shape",
                format!(
                    "Relation '{}' in '{}' is not a list of target ids",
                    options.relation,
                    file_path.display()
                ),
            ))
        }
    };

    let changed = if options.add {
        if targets.iter().any(|t| t == &options.target_id) {
            false
        } else {
            targets.push(options.target_id.clone());
            relations_obj.insert(
                relation_key.clone(),
                serde_yaml::Value::Sequence(
                    targets
                        .iter()
                        .cloned()
                        .map(serde_yaml::Value::String)
                        .collect(),
                ),
            );
            true
        }
    } else {
        let before = targets.len();
        targets.retain(|t| t != &options.target_id);
        let did_change = targets.len() != before;
        if did_change {
            if targets.is_empty() {
                relations_obj.remove(&relation_key);
            } else {
                relations_obj.insert(
                    relation_key.clone(),
                    serde_yaml::Value::Sequence(
                        targets
                            .iter()
                            .cloned()
                            .map(serde_yaml::Value::String)
                            .collect(),
                    ),
                );
            }
        }
        did_change
    };

    // Idempotent no-op (unrelate of an absent entry, or relate of an edge already present):
    // nothing to write. `if_version` was already compared above, so returning here reports an
    // outcome the caller's expectation was checked against.
    if !changed {
        let relations_out = serde_json::to_value(&relations_obj).map_err(|e| {
            QdevError::infrastructure_failure(
                "serialize_error",
                format!("Failed to serialize relations map: {}", e),
            )
        })?;
        return Ok(RelationChangeResult {
            id,
            kind,
            path: file_path,
            rel_path,
            old_version,
            new_version: old_version,
            changed: false,
            relations: relations_out,
        });
    }

    // 5. Line-based frontmatter patch: the whole `relations:` block is replaced with the merged
    // map computed above, which already carries every relation and target the file had before,
    // in its original key order.
    let relations_yaml = serde_yaml::Value::Mapping(relations_obj);

    let patch_opts = FrontmatterPatchOptions {
        status: None,
        title: None,
        custom_fields: vec![("relations".to_string(), relations_yaml)],
        author: Some(options.author.clone()),
        if_version: options.if_version,
    };

    let (patched_content, new_version) = patch_frontmatter(&existing_content, &patch_opts)?;

    // 6. Validate updated frontmatter against JSON Schema, against hydration's own kind rule
    // (see `kind_for_write`, resolved from the file content in step 3) so this write and the
    // next sweep validate against one schema.
    validate_frontmatter(kind, &patched_content).map_err(|errs| {
        QdevError::logical_failure(
            "schema_validation_failed",
            format!("Updated frontmatter failed schema validation: {:?}", errs),
        )
        .with_details(serde_json::json!({
            "validation_errors": errs,
        }))
    })?;

    // 7. Atomic write via tempfile rename
    write_file_atomic(&file_path, &patched_content)?;

    // 8. Extract updated frontmatter for cache and result
    let updated_frontmatter =
        crate::schema::extract_frontmatter(&patched_content).map_err(|e| {
            QdevError::infrastructure_failure(
                "parse_error",
                format!("Failed to parse updated frontmatter: {}", e),
            )
        })?;

    let canonical_id = updated_frontmatter
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or(id);

    // 9. Upsert cache and mark dirty (mirrors apply_entity_update step 9)
    let content_hash = sha256_digest(patched_content.as_bytes());
    let title_val = updated_frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let status_val = updated_frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let owners_val = updated_frontmatter.get("owners").map(|v| v.to_string());

    let c_author = updated_frontmatter
        .get("created_by")
        .and_then(|v| serde_json::from_value::<Author>(v.clone()).ok());
    let u_author = Some(options.author.clone());

    let (epic_id, seq, appetite, safety_class, target_modules) =
        story_detail_fields(kind, &canonical_id, &updated_frontmatter);

    let record = EntityRecord {
        id: canonical_id.clone(),
        kind,
        title: title_val,
        status: status_val,
        owners: owners_val,
        source_path: rel_path.clone(),
        content_hash,
        version: new_version,
        created_by: c_author,
        updated_by: u_author,
        updated_at: current_iso8601(),
        stale: false,
        epic_id,
        seq,
        appetite,
        safety_class,
        target_modules,
    };

    let cache_db_path = options
        .workspace_root
        .join(cache_dir_rel)
        .join("cache.sqlite");
    // The relation edge lands in the cache with the entity row, not at the next boot sweep, so
    // anything reading the graph later in this same process sees the edge this write created.
    upsert_cache_with_relation(
        &cache_db_path,
        &record,
        Some(&RelationRowChange {
            source_id: canonical_id.clone(),
            relation: options.relation.clone(),
            target_id: options.target_id.clone(),
            add: options.add,
        }),
    )?;

    let relations_out = updated_frontmatter
        .get("relations")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));

    Ok(RelationChangeResult {
        id: canonical_id,
        kind,
        path: file_path,
        rel_path,
        old_version,
        new_version,
        changed: true,
        relations: relations_out,
    })
}

#[cfg(test)]
mod path_spelling_tests {
    use super::workspace_rel_path;
    use std::path::Path;

    /// The ordinary case, and the one the CLI prints.
    #[test]
    fn test_workspace_rel_path_is_forward_slashed() {
        assert_eq!(
            workspace_rel_path(
                Path::new("/ws/docs/specs/stories/E1S9.md"),
                Path::new("/ws")
            ),
            "docs/specs/stories/E1S9.md"
        );
    }

    /// A backslash is a legal character in a Unix file name. Normalizing by string replacement
    /// would rewrite this to `docs/specs/stories/od/d.md` — a path no file carries, and one the
    /// sweep's `source_path` could never match either. Walking components leaves it alone.
    #[cfg(unix)]
    #[test]
    fn test_workspace_rel_path_keeps_a_backslash_inside_a_unix_file_name() {
        assert_eq!(
            workspace_rel_path(
                Path::new("/ws/docs/specs/stories/od\\d.md"),
                Path::new("/ws")
            ),
            "docs/specs/stories/od\\d.md"
        );
    }

    /// A path that does not sit under the root is spelled as it is, not silently rebased.
    #[test]
    fn test_workspace_rel_path_passes_through_a_foreign_path() {
        assert_eq!(
            workspace_rel_path(Path::new("/elsewhere/E1S9.md"), Path::new("/ws")),
            "/elsewhere/E1S9.md"
        );
    }
}
