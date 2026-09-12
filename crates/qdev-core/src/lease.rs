use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::id::allocate_decision_id_in_with_rng;
use crate::schema::EntityKind;
use crate::store::{DecisionRecord, EntityRecord, SqliteStore, Store};
use crate::write::{
    acquire_workspace_write_lock, canonical_file_name, current_iso8601, directory_for_kind,
    resolve_entity_file, sha256_digest, upsert_cache_and_mark_dirty, write_file_atomic, Author,
};

/// Worktree-visible story lease per Story 2.3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryLease {
    pub story_id: String,
    pub holder: String,
    #[serde(default = "default_author_type")]
    pub author_type: String,
    #[serde(default)]
    pub worktree_path: String,
    #[serde(default = "default_branch")]
    pub branch: String,
    #[serde(default = "current_iso8601")]
    pub started_at: String,
    #[serde(default)]
    pub session_token: String,
}

fn default_author_type() -> String {
    "human".to_string()
}

fn default_branch() -> String {
    "main".to_string()
}

/// Output payload of a story lease release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleasePayload {
    pub story_id: String,
    pub released: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
}

/// Discovers the git common directory for a repository or linked worktree.
/// Resolves via `git rev-parse --git-common-dir` or parses `.git` file (`gitdir: ...` / `commondir`).
pub fn discover_git_common_dir(workspace_root: &Path) -> PathBuf {
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(workspace_root)
        .output()
    {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                let p = Path::new(&path_str);
                let resolved = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    workspace_root.join(p)
                };
                if let Ok(canonical) = resolved.canonicalize() {
                    return canonical;
                }
                return resolved;
            }
        }
    }

    let git_path = workspace_root.join(".git");
    if git_path.is_file() {
        if let Ok(content) = fs::read_to_string(&git_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(gitdir) = trimmed.strip_prefix("gitdir:") {
                    let gitdir_path = gitdir.trim();
                    let resolved = if Path::new(gitdir_path).is_absolute() {
                        PathBuf::from(gitdir_path)
                    } else {
                        workspace_root.join(gitdir_path)
                    };
                    let commondir_file = resolved.join("commondir");
                    if commondir_file.is_file() {
                        if let Ok(commondir_content) = fs::read_to_string(&commondir_file) {
                            let common_str = commondir_content.trim();
                            let common_resolved = if Path::new(common_str).is_absolute() {
                                PathBuf::from(common_str)
                            } else {
                                resolved.join(common_str)
                            };
                            if let Ok(canonical) = common_resolved.canonicalize() {
                                return canonical;
                            }
                            return common_resolved;
                        }
                    }
                    if let Ok(canonical) = resolved.canonicalize() {
                        return canonical;
                    }
                    return resolved;
                }
            }
        }
    } else if git_path.is_dir() {
        if let Ok(canonical) = git_path.canonicalize() {
            return canonical;
        }
        return git_path;
    }

    git_path
}

/// Discovers current git branch.
pub fn discover_git_branch(workspace_root: &Path) -> String {
    if let Ok(output) = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(workspace_root)
        .output()
    {
        if output.status.success() {
            let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }

    let git_path = workspace_root.join(".git");
    let head_path = if git_path.is_file() {
        if let Ok(content) = fs::read_to_string(&git_path) {
            let gitdir = content
                .lines()
                .find_map(|l| l.strip_prefix("gitdir:").map(|g| g.trim().to_string()))
                .unwrap_or_default();
            let p = if Path::new(&gitdir).is_absolute() {
                PathBuf::from(gitdir)
            } else {
                workspace_root.join(gitdir)
            };
            p.join("HEAD")
        } else {
            git_path.join("HEAD")
        }
    } else {
        git_path.join("HEAD")
    };

    if let Ok(content) = fs::read_to_string(&head_path) {
        let trimmed = content.trim();
        if let Some(branch) = trimmed.strip_prefix("ref: refs/heads/") {
            return branch.to_string();
        } else if !trimmed.is_empty() {
            return "HEAD".to_string();
        }
    }

    "main".to_string()
}

/// Generates a session token `qs_<story_id>_<hex4>`.
pub fn generate_session_token(story_id: &str) -> String {
    let mut rng = rand::rng();
    let num: u16 = rand::Rng::random(&mut rng);
    format!("qs_{}_{:04x}", story_id, num)
}

fn days_from_civil(y: i64, m: u64, d: u64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + (doe as i64) - 719468
}

/// Parses an ISO 8601 timestamp string into epoch seconds.
pub fn parse_iso8601_to_timestamp(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: u64 = s.get(5..7)?.parse().ok()?;
    let day: u64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let min: i64 = s.get(14..16)?.parse().ok()?;
    let sec: i64 = s.get(17..19)?.parse().ok()?;

    if day == 0 || day > 31 || month == 0 || month > 12 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let mut secs = days * 86400 + hour * 3600 + min * 60 + sec;

    if let Some(tz_part) = s.get(19..) {
        if let Some(stripped) = tz_part.strip_prefix('+') {
            if stripped.len() >= 5 {
                let off_h: i64 = stripped[0..2].parse().ok()?;
                let off_m: i64 = stripped[3..5].parse().ok()?;
                secs -= off_h * 3600 + off_m * 60;
            }
        } else if let Some(stripped) = tz_part.strip_prefix('-') {
            if stripped.len() >= 5 {
                let off_h: i64 = stripped[0..2].parse().ok()?;
                let off_m: i64 = stripped[3..5].parse().ok()?;
                secs += off_h * 3600 + off_m * 60;
            }
        }
    }

    Some(secs)
}

/// Resolves an active lease for a story if one exists in shared or local storage.
pub fn get_lease(workspace_root: &Path, story_id: &str) -> Option<StoryLease> {
    let trimmed_id = story_id.trim();
    if trimmed_id.is_empty()
        || trimmed_id.contains('/')
        || trimmed_id.contains('\\')
        || trimmed_id.contains("..")
    {
        return None;
    }

    let git_common = discover_git_common_dir(workspace_root);
    let shared_file = git_common
        .join("qdev/leases")
        .join(format!("{}.json", trimmed_id));
    if shared_file.is_file() {
        if let Ok(content) = fs::read_to_string(&shared_file) {
            if let Ok(lease) = serde_json::from_str::<StoryLease>(&content) {
                return Some(lease);
            }
        }
    }

    let local_file = workspace_root
        .join(".qdev/leases")
        .join(format!("{}.json", story_id));
    if local_file.is_file() {
        if let Ok(content) = fs::read_to_string(&local_file) {
            if let Ok(lease) = serde_json::from_str::<StoryLease>(&content) {
                return Some(lease);
            }
        }
    }

    None
}

/// Lists all active leases across the shared Git directory and local worktree.
pub fn list_leases(workspace_root: &Path) -> Result<Vec<StoryLease>, QdevError> {
    let mut map: BTreeMap<String, StoryLease> = BTreeMap::new();

    let git_common = discover_git_common_dir(workspace_root);
    let shared_dir = git_common.join("qdev/leases");
    if shared_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&shared_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(lease) = serde_json::from_str::<StoryLease>(&content) {
                            map.insert(lease.story_id.clone(), lease);
                        }
                    }
                }
            }
        }
    }

    let local_dir = workspace_root.join(".qdev/leases");
    if local_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&local_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(lease) = serde_json::from_str::<StoryLease>(&content) {
                            map.entry(lease.story_id.clone()).or_insert(lease);
                        }
                    }
                }
            }
        }
    }

    Ok(map.into_values().collect())
}

/// Lists active leases present in the current workspace directory (`.qdev/leases`).
pub fn find_workspace_leases(workspace_root: &Path) -> Result<Vec<StoryLease>, QdevError> {
    let local_dir = workspace_root.join(".qdev/leases");
    let mut leases = Vec::new();
    if local_dir.is_dir() {
        let entries = fs::read_dir(&local_dir).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to read leases directory '{}': {}",
                    local_dir.display(),
                    e
                ),
            )
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(lease) = serde_json::from_str::<StoryLease>(&content) {
                        leases.push(lease);
                    }
                }
            }
        }
    }
    leases.sort_by(|a, b| a.story_id.cmp(&b.story_id));
    Ok(leases)
}

/// Finds the single active lease held in the current workspace.
/// Returns exit 1 `no_active_lease` if no lease is held, or exit 2 `usage_error` if multiple are held.
pub fn find_active_lease(workspace_root: &Path) -> Result<StoryLease, QdevError> {
    let leases = find_workspace_leases(workspace_root)?;
    if leases.is_empty() {
        return Err(QdevError::logical_failure(
            "no_active_lease",
            "No active story lease found in workspace",
        ));
    }
    if leases.len() > 1 {
        return Err(QdevError::usage_error(
            "Multiple active leases held in workspace; specify story ID to release",
        ));
    }
    Ok(leases.into_iter().next().unwrap())
}

/// Claims a lease for an unleased story.
pub fn claim_story(
    workspace_root: &Path,
    story_id: &str,
    author: &Author,
    storage: Option<&StorageConfig>,
    opt_store: Option<&dyn Store>,
) -> Result<StoryLease, QdevError> {
    let trimmed_id = story_id.trim();
    if trimmed_id.is_empty() {
        return Err(QdevError::usage_error("Story ID cannot be empty"));
    }
    if trimmed_id.contains('/') || trimmed_id.contains('\\') || trimmed_id.contains("..") {
        return Err(QdevError::usage_error(format!(
            "Invalid story ID '{}': cannot contain path separators or parent directory references",
            trimmed_id
        )));
    }

    // 1. Acquire workspace advisory write lock
    let _lock_guard = acquire_workspace_write_lock(workspace_root, storage)?;

    // 2. Verify story entity exists and is a story kind
    let entity_kind = if let Some(store) = opt_store {
        if let Ok(Some(record)) = store.get_entity(trimmed_id) {
            Some(record.kind)
        } else {
            resolve_entity_file(workspace_root, None, trimmed_id, storage)
                .map(|(k, _, _)| k)
                .ok()
        }
    } else {
        resolve_entity_file(workspace_root, None, trimmed_id, storage)
            .map(|(k, _, _)| k)
            .ok()
    };

    let kind = match entity_kind {
        Some(k) => k,
        None => {
            return Err(QdevError::logical_failure(
                "entity_not_found",
                format!("Story entity '{}' does not exist", trimmed_id),
            ));
        }
    };

    if kind != EntityKind::Story {
        return Err(QdevError::usage_error("Only story entities can be leased"));
    }

    // 3. Refuse duplicate claims with exit code 5 (already_leased)
    if let Some(existing) = get_lease(workspace_root, trimmed_id) {
        return Err(QdevError::conflict(
            "already_leased",
            format!(
                "Story {} is already leased by {} in {}",
                trimmed_id, existing.holder, existing.worktree_path
            ),
        )
        .with_details(serde_json::json!({
            "story_id": trimmed_id,
            "holder": existing.holder,
            "worktree_path": existing.worktree_path,
        })));
    }

    // 4. Construct lease record
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let worktree_path = canonical_root.to_string_lossy().to_string();
    let branch = discover_git_branch(workspace_root);
    let session_token = generate_session_token(trimmed_id);
    let started_at = current_iso8601();

    let lease = StoryLease {
        story_id: trimmed_id.to_string(),
        holder: author.id.clone(),
        author_type: author.author_type.clone(),
        worktree_path,
        branch,
        started_at,
        session_token,
    };

    let lease_json = serde_json::to_string_pretty(&lease).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize lease record: {}", e),
        )
    })?;

    // 5. Write local lease
    let local_file = workspace_root
        .join(".qdev/leases")
        .join(format!("{}.json", trimmed_id));
    write_file_atomic(&local_file, &lease_json)?;

    // 6. Mirror to shared Git directory
    let git_common = discover_git_common_dir(workspace_root);
    let shared_file = git_common
        .join("qdev/leases")
        .join(format!("{}.json", trimmed_id));
    if shared_file != local_file && git_common.exists() {
        write_file_atomic(&shared_file, &lease_json)?;
    }

    Ok(lease)
}

/// Releases an existing story lease.
pub fn release_story(
    workspace_root: &Path,
    story_id: &str,
    author: &Author,
    force: bool,
    justification: Option<&str>,
    storage: Option<&StorageConfig>,
    opt_store: Option<&dyn Store>,
) -> Result<ReleasePayload, QdevError> {
    let trimmed_id = story_id.trim();
    if trimmed_id.is_empty() {
        return Err(QdevError::usage_error("Story ID cannot be empty"));
    }
    if trimmed_id.contains('/') || trimmed_id.contains('\\') || trimmed_id.contains("..") {
        return Err(QdevError::usage_error(format!(
            "Invalid story ID '{}': cannot contain path separators or parent directory references",
            trimmed_id
        )));
    }

    // Verify entity kind if entity exists
    let entity_kind = if let Some(store) = opt_store {
        if let Ok(Some(record)) = store.get_entity(trimmed_id) {
            Some(record.kind)
        } else {
            resolve_entity_file(workspace_root, None, trimmed_id, storage)
                .map(|(k, _, _)| k)
                .ok()
        }
    } else {
        resolve_entity_file(workspace_root, None, trimmed_id, storage)
            .map(|(k, _, _)| k)
            .ok()
    };
    if let Some(kind) = entity_kind {
        if kind != EntityKind::Story {
            return Err(QdevError::usage_error("Only story entities can be leased"));
        }
    }

    let existing = match get_lease(workspace_root, trimmed_id) {
        Some(l) => l,
        None => {
            return Err(QdevError::logical_failure(
                "lease_not_found",
                format!("Story {} is not leased", trimmed_id),
            ));
        }
    };

    let is_holder = existing.holder == author.id;
    let mut decision_id = None;

    if force {
        let just = justification.map(|j| j.trim()).unwrap_or("");
        if just.is_empty() {
            return Err(QdevError::policy_refusal(
                "needs_justification",
                "--force requires a non-empty --justification",
            ));
        }
        let dec_id = create_lease_override_decision(
            workspace_root,
            storage,
            trimmed_id,
            just,
            author,
            &existing,
        )?;
        decision_id = Some(dec_id);
    } else if !is_holder {
        return Err(QdevError::policy_refusal(
            "policy_refusal",
            format!(
                "Story {} is leased by {} in {}; releasing requires --force --justification",
                trimmed_id, existing.holder, existing.worktree_path
            ),
        ));
    }

    // Remove local and shared lease files
    let local_file = workspace_root
        .join(".qdev/leases")
        .join(format!("{}.json", trimmed_id));
    if local_file.exists() {
        fs::remove_file(&local_file).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to remove local lease file '{}': {}",
                    local_file.display(),
                    e
                ),
            )
        })?;
    }

    let git_common = discover_git_common_dir(workspace_root);
    let shared_file = git_common
        .join("qdev/leases")
        .join(format!("{}.json", trimmed_id));
    if shared_file.exists() {
        let _ = fs::remove_file(&shared_file);
    }

    let other_local = Path::new(&existing.worktree_path)
        .join(".qdev/leases")
        .join(format!("{}.json", trimmed_id));
    if other_local.exists() && other_local != local_file {
        let _ = fs::remove_file(&other_local);
    }

    Ok(ReleasePayload {
        story_id: trimmed_id.to_string(),
        released: true,
        decision_id,
    })
}

/// Automatically releases an active lease for a story if present (e.g. upon transition to terminal state).
pub fn auto_release_lease(workspace_root: &Path, story_id: &str) -> Result<(), QdevError> {
    let trimmed_id = story_id.trim();
    if trimmed_id.is_empty()
        || trimmed_id.contains('/')
        || trimmed_id.contains('\\')
        || trimmed_id.contains("..")
    {
        return Ok(());
    }

    if let Some(existing) = get_lease(workspace_root, trimmed_id) {
        let local_file = workspace_root
            .join(".qdev/leases")
            .join(format!("{}.json", trimmed_id));
        if local_file.exists() {
            let _ = fs::remove_file(&local_file);
        }

        let git_common = discover_git_common_dir(workspace_root);
        let shared_file = git_common
            .join("qdev/leases")
            .join(format!("{}.json", trimmed_id));
        if shared_file.exists() {
            let _ = fs::remove_file(&shared_file);
        }

        let other_local = Path::new(&existing.worktree_path)
            .join(".qdev/leases")
            .join(format!("{}.json", trimmed_id));
        if other_local.exists() && other_local != local_file {
            let _ = fs::remove_file(&other_local);
        }
    }
    Ok(())
}

/// Logs a committed `DEC-` record with `decision_type: lease_override` when a lease is broken with `--force --justification`.
pub fn create_lease_override_decision(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    justification: &str,
    author: &Author,
    prior_lease: &StoryLease,
) -> Result<String, QdevError> {
    let _lock_guard = acquire_workspace_write_lock(workspace_root, storage)?;

    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    let cache_dir_rel = st.cache_dir.as_str();
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");
    let opt_store = if cache_db_path.is_file() {
        Some(SqliteStore::open(&cache_db_path)?)
    } else {
        None
    };

    let mut rng = rand::rng();
    let dec_ident = allocate_decision_id_in_with_rng(
        workspace_root,
        st,
        &mut rng,
        opt_store.as_ref().map(|s| s as &dyn Store),
    )?;
    let dec_id = dec_ident.to_string();

    let title = format!("Lease override on story {}", story_id);
    let timestamp = current_iso8601();
    let context_desc = format!(
        "Story lease held by {} in worktree {} overridden",
        prior_lease.holder, prior_lease.worktree_path
    );

    let frontmatter_json = serde_json::json!({
        "id": dec_id,
        "title": title,
        "status": "active",
        "version": 1,
        "created_by": {
            "type": author.author_type,
            "id": author.id,
        },
        "updated_by": {
            "type": author.author_type,
            "id": author.id,
        },
        "subject_id": story_id,
        "decision_type": "lease_override",
        "context": context_desc,
        "ruling": justification,
        "created_at": timestamp,
    });

    crate::schema::validate_value_detailed(EntityKind::Decision, &frontmatter_json).map_err(
        |errs| {
            QdevError::logical_failure(
                "schema_violation",
                format!(
                    "Decision frontmatter schema validation failed: {}",
                    errs.iter()
                        .map(|e| e.to_string())
                        .collect::<Vec<_>>()
                        .join("; ")
                ),
            )
        },
    )?;

    let frontmatter_yaml = serde_yaml::to_string(&frontmatter_json).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize decision frontmatter: {}", e),
        )
    })?;

    let dec_content = format!(
        "---\n{}---\n\n# {}\n\nContext: {}\n\n{}\n",
        frontmatter_yaml, title, context_desc, justification
    );

    let rel_dir = directory_for_kind(storage, EntityKind::Decision);
    let file_name = canonical_file_name(&dec_id);
    let dec_file_path = workspace_root.join(&rel_dir).join(&file_name);

    write_file_atomic(&dec_file_path, &dec_content)?;

    if let Some(ref store) = opt_store {
        let rel_source_path = crate::write::workspace_rel_path(&dec_file_path, workspace_root);
        let content_hash = sha256_digest(dec_content.as_bytes());

        let decision_entity_record = EntityRecord {
            id: dec_id.clone(),
            kind: EntityKind::Decision,
            title: Some(title),
            status: Some("active".to_string()),
            owners: None,
            source_path: rel_source_path,
            content_hash,
            version: 1,
            created_by: Some(author.clone()),
            updated_by: Some(author.clone()),
            updated_at: timestamp.to_string(),
            stale: false,
            epic_id: None,
            seq: None,
            appetite: None,
            safety_class: None,
            target_modules: None,
        };
        upsert_cache_and_mark_dirty(&cache_db_path, &decision_entity_record)?;

        let decision_record = DecisionRecord {
            id: dec_id.clone(),
            subject_id: story_id.to_string(),
            decision_type: Some("lease_override".to_string()),
            topic: None,
            context: Some(context_desc),
            ruling: Some(justification.to_string()),
            author_type: Some(author.author_type.clone()),
            author_id: Some(author.id.clone()),
            created_at: Some(timestamp.to_string()),
        };
        store.upsert_decision(&decision_record)?;
    }

    Ok(dec_id)
}
