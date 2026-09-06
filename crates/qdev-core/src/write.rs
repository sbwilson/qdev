use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::id::{Identifier, IdentifierKind};
use crate::schema::{validate_frontmatter, EntityKind};

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

    // Inspect existing version
    let mut existing_version = None;
    if let Some(v_block) = blocks.iter().find(|b| b.key == "version") {
        let mut v_str = String::new();
        for &line in &fm_lines[v_block.start_line..v_block.end_line] {
            v_str.push_str(line);
        }
        if let Ok(serde_yaml::Value::Mapping(map)) =
            serde_yaml::from_str::<serde_yaml::Value>(&v_str)
        {
            if let Some(val) = map.get(serde_yaml::Value::String("version".to_string())) {
                match val {
                    serde_yaml::Value::Number(num) => existing_version = num.as_u64(),
                    serde_yaml::Value::String(s) => {
                        existing_version = s.trim().trim_matches(['"', '\'']).parse::<u64>().ok()
                    }
                    _ => {}
                }
            }
        } else if let Some((_, rest)) = v_str.split_once(':') {
            let candidate = rest
                .split(['#', '\r', '\n'])
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['"', '\'']);
            existing_version = candidate.parse::<u64>().ok();
        }
    }

    // Check optimistic concurrency --if-version
    if let Some(expected) = options.if_version {
        match existing_version {
            Some(v) if v == expected => {}
            Some(actual) => {
                return Err(QdevError::conflict(
                    "version_mismatch",
                    format!("Version mismatch: expected {}, found {}", expected, actual),
                )
                .with_details(serde_json::json!({
                    "expected_version": expected,
                    "current_version": actual,
                })));
            }
            None => {
                return Err(QdevError::conflict(
                    "version_mismatch",
                    format!(
                        "Version mismatch: expected {}, but entity has no version",
                        expected
                    ),
                )
                .with_details(serde_json::json!({
                    "expected_version": expected,
                    "current_version": serde_json::Value::Null,
                })));
            }
        }
    }

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
fn parse_heading_line(line: &str) -> Option<(usize, &str)> {
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
struct FenceTracker {
    fence: Option<(char, usize)>,
}

impl FenceTracker {
    /// Processes a line and returns whether this line is part of a fenced code block
    /// (including opening or closing fence lines).
    fn process_line(&mut self, line: &str) -> bool {
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
#[derive(Debug, Clone)]
pub struct EntityRecord {
    pub id: String,
    pub kind: EntityKind,
    pub title: Option<String>,
    pub status: Option<String>,
    pub owners: Option<String>,
    pub source_path: String,
    pub content_hash: String,
    pub version: u64,
    pub created_by: Option<Author>,
    pub updated_by: Option<Author>,
    pub updated_at: String,
    // Story detail fields
    pub epic_id: Option<String>,
    pub seq: Option<u32>,
    pub appetite: Option<String>,
    pub safety_class: Option<String>,
    pub target_modules: Option<String>,
}

fn current_iso8601() -> String {
    let now = SystemTime::now();
    let duration = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let mut days = (secs / 86400) as i64;
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

/// Computes the sha256 hex digest of a byte slice.
pub fn sha256_digest(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}

/// Upserts an updated entity into the SQLite cache `entities` (and kind-specific) table,
/// records its dirty status in `dirty_entities`, and invalidates `sync_state`.
/// Configures WAL mode and `busy_timeout = 5000ms`.
pub fn upsert_cache_and_mark_dirty(
    cache_db_path: &Path,
    entity: &EntityRecord,
) -> Result<(), QdevError> {
    if let Some(parent) = cache_db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| {
                QdevError::infrastructure_failure(
                    "io_error",
                    format!(
                        "Failed to create cache directory '{}': {}",
                        parent.display(),
                        e
                    ),
                )
            })?;
        }
    }

    let conn = rusqlite::Connection::open(cache_db_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to open cache database: {}", e),
        )
    })?;

    // WAL mode and 5000ms busy timeout per AD-4
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to set journal_mode=WAL: {}", e),
            )
        })?;

    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to set busy_timeout=5000: {}", e),
            )
        })?;

    // Ensure schema v1 exists
    crate::init::create_schema_v1(&conn)?;

    conn.execute_batch("BEGIN IMMEDIATE;").map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to begin transaction: {}", e),
        )
    })?;

    let tx_res = (|| -> Result<(), QdevError> {
        let (c_type, c_id) = match entity.created_by {
            Some(ref a) => (Some(a.author_type.clone()), Some(a.id.clone())),
            None => (None, None),
        };
        let (u_type, u_id) = match entity.updated_by {
            Some(ref a) => (Some(a.author_type.clone()), Some(a.id.clone())),
            None => (None, None),
        };

        // 1. Upsert into entities table (preserve existing created_by via COALESCE if omitted)
        conn.execute(
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
    updated_at = excluded.updated_at;
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
                conn.execute(
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

        // 3. Mark row as dirty in dirty_entities
        let dirty_at = current_iso8601();
        conn.execute(
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

        // 4. Invalidate sync_state for this entity path
        conn.execute(
            "DELETE FROM sync_state WHERE path = ?1;",
            rusqlite::params![entity.source_path],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to invalidate sync_state: {}", e),
            )
        })?;

        Ok(())
    })();

    if let Err(e) = tx_res {
        let _ = conn.execute_batch("ROLLBACK;");
        return Err(e);
    }

    conn.execute_batch("COMMIT;").map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to commit transaction: {}", e),
        )
    })?;

    Ok(())
}

/// Standard entity file resolution directory map with optional storage config.
fn directory_for_kind(storage: Option<&StorageConfig>, kind: EntityKind) -> PathBuf {
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

/// Finds a file matching `id.md` or `id-*.md` or `id_*.md` in `dir` (case-insensitively).
/// Returns an error if multiple files match the entity ID.
fn find_file_in_dir(dir: &Path, id: &str) -> Result<Option<PathBuf>, QdevError> {
    if !dir.exists() {
        return Ok(None);
    }
    let direct = dir.join(format!("{}.md", id));
    let mut matches = Vec::new();
    if direct.is_file() {
        matches.push(direct);
    }

    if let Ok(entries) = fs::read_dir(dir) {
        let id_lower = id.to_ascii_lowercase();
        let exact_lower = format!("{}.md", id_lower);
        let prefix_dash_lower = format!("{}-", id_lower);
        let prefix_underscore_lower = format!("{}_", id_lower);
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = entry.file_name().to_str() {
                    let name_lower = name.to_ascii_lowercase();
                    if (name_lower == exact_lower
                        || name_lower.starts_with(&prefix_dash_lower)
                        || name_lower.starts_with(&prefix_underscore_lower))
                        && name.ends_with(".md")
                        && !matches.contains(&path)
                    {
                        matches.push(path);
                    }
                }
            }
        }
    }

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

/// Resolves an entity file path and entity kind given workspace root, optional kind, ID, and optional storage config.
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
        return Err(QdevError::usage_error(format!(
            "Entity file not found for '{}'",
            trimmed_id
        )));
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
        0 => Err(QdevError::usage_error(format!(
            "Entity file not found for '{}'",
            trimmed_id
        ))),
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

    let rel_path = file_path
        .strip_prefix(&options.workspace_root)
        .unwrap_or(&file_path)
        .to_string_lossy()
        .to_string();

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

    // Extract old version for record keeping
    let old_frontmatter_val = crate::schema::extract_frontmatter(&existing_content).ok();
    let old_version = old_frontmatter_val
        .as_ref()
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

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

    // 6. Validate updated frontmatter against JSON Schema
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
    let (epic_id, seq, appetite, safety_class, target_modules) = if kind == EntityKind::Story {
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
        let s_num = if let Ok(Identifier::Story { story, .. }) = canonical_id.parse::<Identifier>()
        {
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
    } else {
        (None, None, None, None, None)
    };

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
