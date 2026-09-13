//! Scratchpad domain logic for story development reasoning, decisions, tradeoffs, and transitions.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::schema::EntityKind;
use crate::store::sqlite::{file_change_stamp, SqliteStore};
use crate::store::{ScratchpadRecord, Store};
use crate::write::{
    acquire_workspace_write_lock, current_iso8601, directory_for_kind, resolve_entity_file,
    sha256_digest, write_file_atomic, Author,
};

/// Valid entry kinds for scratchpad entries.
pub const VALID_SCRATCH_KINDS: &[&str] = &["note", "decision", "tradeoff", "transition"];

/// Author attribution for a scratchpad entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchpadAuthor {
    pub r#type: String,
    pub id: String,
}

impl From<Author> for ScratchpadAuthor {
    fn from(a: Author) -> Self {
        Self {
            r#type: a.author_type,
            id: a.id,
        }
    }
}

impl From<&Author> for ScratchpadAuthor {
    fn from(a: &Author) -> Self {
        Self {
            r#type: a.author_type.clone(),
            id: a.id.clone(),
        }
    }
}

impl From<ScratchpadAuthor> for Author {
    fn from(a: ScratchpadAuthor) -> Self {
        Self {
            author_type: a.r#type,
            id: a.id,
        }
    }
}

/// A single entry in a story's scratchpad.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchpadEntry {
    pub seq: u32,
    pub at: String,
    pub author: ScratchpadAuthor,
    pub kind: String,
    pub text: String,
}

impl ScratchpadEntry {
    pub fn to_record(&self, story_id: &str) -> ScratchpadRecord {
        ScratchpadRecord {
            story_id: story_id.to_string(),
            seq: self.seq,
            at: self.at.clone(),
            author_type: Some(self.author.r#type.clone()),
            author_id: Some(self.author.id.clone()),
            kind: Some(self.kind.clone()),
            text: Some(self.text.clone()),
        }
    }

    pub fn from_record(r: &ScratchpadRecord) -> Self {
        Self {
            seq: r.seq,
            at: r.at.clone(),
            author: ScratchpadAuthor {
                r#type: r.author_type.clone().unwrap_or_else(|| "human".to_string()),
                id: r.author_id.clone().unwrap_or_default(),
            },
            kind: r.kind.clone().unwrap_or_else(|| "note".to_string()),
            text: r.text.clone().unwrap_or_default(),
        }
    }
}

/// JSON payload shape for `qdev scratch append`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchAppendPayload {
    pub story_id: String,
    pub seq: u32,
    pub at: String,
    pub author: ScratchpadAuthor,
    pub kind: String,
    pub text: String,
}

/// JSON payload shape for `qdev scratch read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScratchReadPayload {
    pub story_id: String,
    pub entries: Vec<ScratchpadEntry>,
}

/// Computes the deterministic token estimate for `text` using `(chars + 3) / 4`.
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// Appends a new scratchpad entry for `story_id` under the workspace advisory write lock,
/// atomically updating the JSONL file and synchronizing the SQLite cache.
pub fn append_scratch_entry(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    kind: Option<&str>,
    text: &str,
    author: &Author,
    store: Option<&dyn Store>,
) -> Result<ScratchpadEntry, QdevError> {
    // 1. Resolve and validate target story exists and is a story
    let (_entity_kind, canonical_story_id, _story_path) =
        resolve_entity_file(workspace_root, Some(EntityKind::Story), story_id, storage)?;

    // 2. Validate entry kind
    let resolved_kind = match kind {
        Some(k) => {
            let trimmed = k.trim();
            if !VALID_SCRATCH_KINDS.contains(&trimmed) {
                return Err(QdevError::usage_error(format!(
                    "Invalid scratchpad kind '{}', must be one of: note, decision, tradeoff, transition",
                    trimmed
                )));
            }
            trimmed.to_string()
        }
        None => "note".to_string(),
    };

    // 3. Validate entry text
    let trimmed_text = text.trim();
    if trimmed_text.is_empty() {
        return Err(QdevError::usage_error("Scratchpad text cannot be empty"));
    }

    // 4. Acquire workspace advisory write lock
    let _lock = acquire_workspace_write_lock(workspace_root, storage)?;

    // 5. Ensure scratchpad directory exists
    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    let scratch_dir = workspace_root.join(directory_for_kind(storage, EntityKind::Scratchpad));
    if !scratch_dir.exists() {
        fs::create_dir_all(&scratch_dir).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create scratchpad directory '{}': {}",
                    scratch_dir.display(),
                    e
                ),
            )
        })?;
    }

    let scratch_file_path = scratch_dir.join(format!("{}.jsonl", canonical_story_id));

    // 6. Compute sequence number by scanning existing entries
    let mut max_seq: u32 = 0;
    let mut existing_content = String::new();
    if scratch_file_path.exists() {
        existing_content = fs::read_to_string(&scratch_file_path).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to read scratchpad file '{}': {}",
                    scratch_file_path.display(),
                    e
                ),
            )
        })?;

        for (line_idx, line) in existing_content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let val = serde_json::from_str::<serde_json::Value>(trimmed).map_err(|e| {
                QdevError::logical_failure(
                    "parse_error",
                    format!(
                        "Malformed scratchpad entry in '{}' at line {}: {}",
                        scratch_file_path.display(),
                        line_idx + 1,
                        e
                    ),
                )
            })?;
            if let Some(s) = val.get("seq").and_then(|v| v.as_u64()) {
                if let Ok(seq_u32) = u32::try_from(s) {
                    max_seq = max_seq.max(seq_u32);
                }
            }
        }
    }

    let new_seq = max_seq.checked_add(1).ok_or_else(|| {
        QdevError::logical_failure(
            "overflow",
            format!(
                "Scratchpad sequence overflow in '{}'",
                scratch_file_path.display()
            ),
        )
    })?;

    // 7. Construct entry
    let timestamp = current_iso8601();
    let entry = ScratchpadEntry {
        seq: new_seq,
        at: timestamp,
        author: ScratchpadAuthor::from(author),
        kind: resolved_kind,
        text: text.to_string(),
    };

    // 8. Atomically write updated JSONL file
    let line_json = serde_json::to_string(&entry).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize scratchpad entry: {}", e),
        )
    })?;

    let mut new_content = existing_content;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    new_content.push_str(&line_json);
    new_content.push('\n');

    write_file_atomic(&scratch_file_path, &new_content)?;

    // 9. Synchronize SQLite cache and sync_state
    let rel_path = relative_path(workspace_root, &scratch_file_path);

    let (mtime, size) = file_change_stamp(&scratch_file_path);
    let content_hash = sha256_digest(new_content.as_bytes());

    let record = entry.to_record(&canonical_story_id);

    let cache_dir_rel = st.cache_dir.as_str();
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");

    if let Some(s) = store {
        s.upsert_scratchpad_entry(&record)?;
        s.upsert_sync_state(&rel_path, mtime, size, Some(&content_hash))?;
    } else if cache_db_path.is_file() {
        let s = SqliteStore::open(&cache_db_path)?;
        s.upsert_scratchpad_entry(&record)?;
        s.upsert_sync_state(&rel_path, mtime, size, Some(&content_hash))?;
    }

    Ok(entry)
}

/// Reads all scratchpad entries for `story_id`, sorted in `seq` ascending order.
pub fn read_scratch_entries(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
) -> Result<Vec<ScratchpadEntry>, QdevError> {
    // Validate target story exists
    let (_entity_kind, canonical_story_id, _story_path) =
        resolve_entity_file(workspace_root, Some(EntityKind::Story), story_id, storage)?;

    let scratch_dir = workspace_root.join(directory_for_kind(storage, EntityKind::Scratchpad));
    let scratch_file_path = scratch_dir.join(format!("{}.jsonl", canonical_story_id));

    if !scratch_file_path.is_file() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&scratch_file_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to read scratchpad file '{}': {}",
                scratch_file_path.display(),
                e
            ),
        )
    })?;

    let mut entries = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let entry = serde_json::from_str::<ScratchpadEntry>(trimmed).map_err(|e| {
            QdevError::logical_failure(
                "parse_error",
                format!(
                    "Malformed scratchpad entry in '{}' at line {}: {}",
                    scratch_file_path.display(),
                    line_idx + 1,
                    e
                ),
            )
        })?;
        entries.push(entry);
    }

    entries.sort_by_key(|e| e.seq);
    Ok(entries)
}

/// Summarizes scratchpad entries by including all entries of kind `decision` and `transition`
/// plus the last `last_n` entries (deduplicated and sorted by `seq` ascending), bounded by `budget`.
pub fn summarize_scratch_entries(
    entries: &[ScratchpadEntry],
    last_n: usize,
    budget: Option<usize>,
) -> Vec<ScratchpadEntry> {
    let mut map = BTreeMap::new();
    for e in entries {
        if e.kind == "decision" || e.kind == "transition" {
            map.insert(e.seq, e.clone());
        }
    }

    let start = entries.len().saturating_sub(last_n);
    for e in &entries[start..] {
        map.insert(e.seq, e.clone());
    }

    let candidates: Vec<ScratchpadEntry> = map.into_values().collect();

    if let Some(max_tokens) = budget {
        apply_token_budget(candidates, max_tokens)
    } else {
        candidates
    }
}

/// Filters any list of scratchpad entries to fit within `budget` tokens,
/// prioritizing key decisions, transitions, and recent entries while dropping lowest-priority entries.
pub fn filter_scratch_entries_by_budget(
    entries: &[ScratchpadEntry],
    budget: usize,
) -> Vec<ScratchpadEntry> {
    apply_token_budget(entries.to_vec(), budget)
}

/// Deterministically drops lowest-priority entries until total tokens <= `max_tokens`.
fn apply_token_budget(
    mut candidates: Vec<ScratchpadEntry>,
    max_tokens: usize,
) -> Vec<ScratchpadEntry> {
    let mut current_tokens: usize = candidates.iter().map(|e| estimate_tokens(&e.text)).sum();

    while current_tokens > max_tokens && !candidates.is_empty() {
        // Priority hierarchy when dropping:
        // 1. Drop non-key entries (kind != "decision" && kind != "transition") starting with lowest seq.
        // 2. If only key entries remain, drop key entries starting with lowest seq.
        let non_key_idx = candidates
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind != "decision" && e.kind != "transition")
            .min_by_key(|(_, e)| e.seq)
            .map(|(idx, _)| idx);

        let drop_idx = match non_key_idx {
            Some(idx) => idx,
            None => candidates
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.seq)
                .map(|(idx, _)| idx)
                .unwrap_or(0),
        };

        let dropped = candidates.remove(drop_idx);
        current_tokens = current_tokens.saturating_sub(estimate_tokens(&dropped.text));
    }

    candidates.sort_by_key(|e| e.seq);
    candidates
}

fn relative_path(workspace_root: &Path, file_path: &Path) -> String {
    file_path
        .strip_prefix(workspace_root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .replace('\\', "/")
}
