//! Path-allowlisted chores per Story 2.10.
//!
//! A *chore* is a trivial change that belongs to no story: `qdev chore start` records a title and
//! the path globs the change is confined to, and `qdev chore commit` stages and commits **only**
//! what falls under those globs, then records the chore as a `DEC-` of type `human_ruling`.
//! `qdev chore list` shows the records, and `qdev chore close` / `qdev chore abort` settle one
//! that is never going to be committed, so a mistyped allowlist is not a dead end.
//!
//! The in-flight record lives in `<qdev>/chores/<id>.json` — gitignored, like `<qdev>/leases/`
//! (decision D-1), so nothing outside the declared allowlist ever reaches Git except the `DEC-`
//! record itself, which is the single permitted exception (decision D-5).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::decision::{log_decision, DecisionInput};
use crate::errors::QdevError;
use crate::init::qdev_dir;
use crate::validate::glob_match;
use crate::write::{acquire_workspace_write_lock, current_iso8601, write_file_atomic, Author};

/// A recorded fast-track change: the declared allowlist and, once committed, where it landed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoreRecord {
    pub id: String,
    pub title: String,
    pub paths: Vec<String>,
    pub author: Author,
    pub started_at: String,
    /// `open` until `qdev chore commit` closes it, or `close`/`abort` settle it without one.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    /// Where that ruling was written, so a retry after a failed commit reuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_path: Option<String>,
    /// When `close`/`abort` settled the chore, who did it, and the reason they gave.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_by: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A changed path that the allowlist did not cover, with whether it was sitting in the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludedPath {
    pub path: String,
    pub staged: bool,
}

/// Everything `qdev chore commit` decided, for both text and `--json` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoreCommitResult {
    pub chore_id: String,
    pub committed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub included: Vec<String>,
    pub excluded: Vec<ExcludedPath>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_path: Option<String>,
}

/// Inputs to [`start_chore`].
#[derive(Debug, Clone)]
pub struct StartChoreInput<'a> {
    pub workspace_root: &'a Path,
    pub storage: Option<&'a StorageConfig>,
    pub title: String,
    pub paths: Vec<String>,
    pub author: Author,
    pub alongside: bool,
}

/// Inputs to [`commit_chore`].
#[derive(Debug, Clone)]
pub struct CommitChoreInput<'a> {
    pub workspace_root: &'a Path,
    pub storage: Option<&'a StorageConfig>,
    pub author: Author,
    pub strict: bool,
}

/// Inputs to [`close_chore`] and [`abort_chore`], which share everything but the outcome.
#[derive(Debug, Clone)]
pub struct FinishChoreInput<'a> {
    pub workspace_root: &'a Path,
    pub storage: Option<&'a StorageConfig>,
    /// Why the chore is being settled without a commit, if the caller gave a reason.
    pub reason: Option<String>,
    /// Whoever settles a chore is usually not whoever started it.
    pub author: Author,
}

/// The directory holding chore records, relative to `workspace_root`.
///
/// Derived from the configured `cache_dir` — the same rule `qdev init` uses to create and
/// gitignore `<qdev>/chores` — so a workspace that relocated its cache keeps its records in
/// gitignored space, which is what decision D-1 promises. No config means the default layout.
pub fn chore_dir(workspace_root: &Path, storage: Option<&StorageConfig>) -> PathBuf {
    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    workspace_root.join(qdev_dir(st)).join("chores")
}

/// Persists a chore record to `<qdev>/chores/<id>.json`.
fn write_record(
    root: &Path,
    storage: Option<&StorageConfig>,
    record: &ChoreRecord,
) -> Result<(), QdevError> {
    let content = serde_json::to_string_pretty(record).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize chore record: {}", e),
        )
    })?;
    write_file_atomic(
        &chore_dir(root, storage).join(format!("{}.json", record.id)),
        &content,
    )
}

/// Derives a stable, filesystem-safe record id from a chore title.
///
/// A title that normalises to nothing falls back to `chore` rather than producing an empty id,
/// and the result is capped so a long title still yields a usable file name.
pub fn derive_chore_id(title: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = true; // also suppresses a separator the title starts with
    for c in title.chars() {
        // `is_alphanumeric`, not the ASCII-only variant: a title written in another script
        // still says something, and dropping every non-ASCII letter collapses all of them to
        // one `chore-untitled` that identifies nothing.
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_was_sep = false;
        } else if matches!(c, '-' | ' ' | '_') && !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
        // Anything else (punctuation, non-ASCII) contributes nothing — and must not count as a
        // separator, or a title like "Fix typo!" would look like it held no words at all.
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        // Nothing usable survived (an empty or all-separator title): give it a name that
        // is not a mystery.
        "chore-untitled".to_string()
    } else {
        // Capped, or a long title produces a file name the filesystem will refuse.
        let capped: String = slug.chars().take(60).collect();
        format!("chore-{}", capped.trim_matches('-'))
    }
}

/// Lists every recorded chore in the workspace, by id. Records that fail to parse are skipped
/// rather than failing the whole listing — the same tolerance [`crate::lease::find_workspace_leases`]
/// shows, so one corrupt file cannot hide every other chore.
pub fn list_chore_records(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
) -> Result<Vec<ChoreRecord>, QdevError> {
    let dir = chore_dir(workspace_root, storage);
    let mut records = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(record) = serde_json::from_str::<ChoreRecord>(&content) {
                        records.push(record);
                    }
                }
            }
        }
    }
    records.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(records)
}

/// The one open chore, if any. Records that failed to parse are skipped, exactly as
/// [`crate::lease::find_workspace_leases`] skips an unreadable lease.
pub fn find_open_chore(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
) -> Result<Option<ChoreRecord>, QdevError> {
    Ok(list_chore_records(workspace_root, storage)?
        .into_iter()
        .find(|r| r.status == "open"))
}

/// Starts a chore: records its allowlist so a later `qdev chore commit` can read it back.
///
/// Refuses while this worktree holds a story lease unless `alongside` is set, and refuses to
/// open a second chore while one is already open — one fast track at a time, the same rule that
/// keeps [`crate::lease::find_active_lease`] from having to choose between two.
pub fn start_chore(input: &StartChoreInput) -> Result<ChoreRecord, QdevError> {
    let root = input.workspace_root;
    let title = input.title.trim();
    if title.is_empty() {
        return Err(QdevError::usage_error("Chore title cannot be empty"));
    }

    let mut paths: Vec<String> = Vec::new();
    for pattern in &input.paths {
        let trimmed = pattern.trim();
        if !trimmed.is_empty() && !paths.iter().any(|p| p == trimmed) {
            paths.push(trimmed.to_string());
        }
    }
    if paths.is_empty() {
        return Err(QdevError::usage_error_with_code(
            "paths_required",
            "At least one --paths glob is required to start a chore",
        ));
    }

    input.author.validate()?;

    // Held across the check-then-write below: two `chore start` runs in one worktree would
    // otherwise both see no open record and the second would overwrite the first's file.
    let _lock = acquire_workspace_write_lock(root, input.storage)?;

    if let Some(open) = find_open_chore(root, input.storage)? {
        return Err(QdevError::conflict(
            "chore_in_progress",
            format!(
                "A chore is already open ('{}', {}); commit or delete it before starting another",
                open.id, open.title
            ),
        ));
    }

    // Every lease in this worktree blocks a new chore, so all of them are named — picking one
    // out of a set would point at a story this worktree may not even be working on.
    let leases = crate::lease::find_workspace_leases(root)?;
    if !input.alongside && !leases.is_empty() {
        let named: Vec<String> = leases
            .iter()
            .map(|l| format!("{} (held by {})", l.story_id, l.holder))
            .collect();
        return Err(QdevError::policy_refusal(
            "lease_held",
            format!(
                "This worktree holds a story lease on {}; pass --alongside to start a chore \
                 alongside it",
                named.join(", ")
            ),
        ));
    }

    let slug = derive_chore_id(title);
    // An id already on disk — from an earlier chore with the same title — stays as history: take
    // the next free `-2`, `-3`, … rather than overwriting the old record.
    let dir = chore_dir(root, input.storage);
    let mut id = slug.clone();
    let mut attempt = 2;
    while dir.join(format!("{}.json", id)).exists() {
        id = format!("{}-{}", slug, attempt);
        attempt += 1;
    }
    let record = ChoreRecord {
        id: id.clone(),
        title: title.to_string(),
        paths,
        author: input.author.clone(),
        started_at: current_iso8601(),
        status: "open".to_string(),
        commit: None,
        decision_id: None,
        decision_path: None,
        closed_at: None,
        closed_by: None,
        reason: None,
    };

    write_record(root, input.storage, &record)?;
    Ok(record)
}

/// One entry of `git status --porcelain=v1 -z`, cut down to what the allowlist needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChangedPath {
    /// What the allowlist decided and the user is told about: for a rename, the destination.
    path: String,
    staged: bool,
    /// Paths to hand to `git add` — for a rename, only the destination, since the source is
    /// already staged (git reports `R` only for a staged change) and no longer exists, so
    /// staging it again would fail with `pathspec ... did not match any files`.
    stage: Vec<String>,
    /// Paths to test against the allowlist, and to name in the commit's pathspec. A rename
    /// carries both endpoints: matched against the allowlist so a file cannot escape it by
    /// changing names, and committed together so the rename lands as a rename rather than as
    /// a bare addition with a dangling deletion left in the index.
    commit: Vec<String>,
}

/// Every path git reports as changed, staged, unstaged, or untracked — the set the allowlist
/// partitions, so that "out of the allowlist" also covers work sitting in the index (D-4).
fn changed_paths(workspace_root: &Path) -> Result<Vec<ChangedPath>, QdevError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to run 'git status': {}", e),
            )
        })?;

    if !output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_status_failed",
            format!(
                "'git status --porcelain' failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    // Records are NUL-separated with no line structure to lean on, which is why the fields are
    // walked by hand: a rename or copy puts its source path in the field after the destination.
    let mut records = raw.split('\0').filter(|f| !f.is_empty());
    let mut changed: Vec<ChangedPath> = Vec::new();
    while let Some(field) = records.next() {
        // Each record is `<XY> <path>` — two status characters, a space, then the path.
        if field.len() < 3 {
            continue;
        }
        let status = &field[..2];
        let destination = field[2..]
            .strip_prefix(' ')
            .unwrap_or(&field[2..])
            .to_string();
        if destination.starts_with(".qdev/") {
            // Workspace machinery (records, leases, cache) is not project work: it never counts
            // as a change the chore might or might not cover.
            continue;
        }
        let staged = !matches!(status.chars().next(), Some(' ') | Some('?'));

        // Both endpoints of a rename or copy are offered to the allowlist, so a file cannot
        // escape it by changing names. Only the destination can be staged: git reports `R`
        // solely for a staged change, so the source is already in the index and gone from the
        // worktree, and `git add` would refuse it. It stays in the commit's pathspec, or the
        // rename would land as a bare addition with the deletion stranded in the index.
        let mut commit = vec![destination.clone()];
        if matches!(status.chars().next(), Some('R') | Some('C')) {
            if let Some(source) = records.next() {
                if source != destination {
                    commit.push(source.to_string());
                }
            }
        }
        changed.push(ChangedPath {
            path: destination.clone(),
            staged,
            stage: vec![destination],
            commit,
        });
    }
    changed.sort_by(|a, b| a.path.cmp(&b.path));
    changed.dedup_by(|a, b| a.path == b.path);
    Ok(changed)
}

/// Stages and commits only the allowlisted paths, then records the chore as a `DEC-`.
///
/// The commit is made with an explicit pathspec (`git commit -- <paths…>`), so work staged
/// before the command ran stays in the index and is reported rather than absorbed (D-4). The
/// `DEC-` record is the one path committed outside the allowlist (D-5).
pub fn commit_chore(input: &CommitChoreInput) -> Result<ChoreCommitResult, QdevError> {
    let root = input.workspace_root;
    // No write lock is taken here: `log_decision` below acquires the same lock, and holding it
    // across that call would time out. `start_chore` holds it, which is where the
    // check-then-write of a new record actually happens.
    let record = match find_open_chore(root, input.storage)? {
        Some(record) => record,
        None => {
            return Err(QdevError::usage_error_with_code(
                "no_active_chore",
                "No open chore found; start one with `qdev chore start` first",
            ));
        }
    };

    let changed = changed_paths(root)?;
    let mut included: Vec<String> = Vec::new();
    let mut excluded: Vec<ExcludedPath> = Vec::new();
    // What to `git add`, and what to name in the commit's pathspec. They differ for a rename,
    // whose source is already staged and gone from the worktree.
    let mut to_stage: Vec<String> = Vec::new();
    let mut to_commit: Vec<String> = Vec::new();
    for change in changed {
        if record
            .paths
            .iter()
            .any(|pattern| change.commit.iter().any(|p| glob_match(pattern, p)))
        {
            to_stage.extend(change.stage);
            to_commit.extend(change.commit);
            included.push(change.path);
        } else {
            excluded.push(ExcludedPath {
                path: change.path,
                staged: change.staged,
            });
        }
    }

    // The record's own `DEC-` is not a change the allowlist has to cover: on a retry after a
    // failed commit it sits in the working tree as an out-of-allowlist file, and counting it
    // there would refuse every later attempt in strict mode over a file this command wrote.
    let recorded_decision: Option<(String, String)> =
        match (&record.decision_id, &record.decision_path) {
            (Some(id), Some(path)) if root.join(path).exists() => Some((id.clone(), path.clone())),
            _ => None,
        };
    if let Some((_, path)) = &recorded_decision {
        excluded.retain(|e| e.path != *path);
    }

    // Strict first: "any changed path outside the allowlist is a refusal" covers the case
    // where *only* out-of-allowlist paths changed, which is otherwise swallowed by
    // `nothing_to_commit` and reported as a mere logical failure.
    if input.strict && !excluded.is_empty() {
        let names: Vec<String> = excluded.iter().map(|e| e.path.clone()).collect();
        return Err(QdevError::policy_refusal(
            "out_of_allowlist",
            format!(
                "Changes outside the allowlist: {}; commit nothing (strict mode)",
                names.join(", ")
            ),
        ));
    }

    if included.is_empty() {
        return Err(QdevError::logical_failure(
            "nothing_to_commit",
            format!(
                "Nothing under the declared allowlist ({}) changed",
                record.paths.join(", ")
            ),
        ));
    }

    // The ruling is written before the commit so the commit can carry it (D-5). If a previous
    // attempt already wrote it, reuse that record: a second `DEC-` for one chore would fill the
    // ledger with duplicates of the same decision.
    let ruling = format!(
        "Committed {} under allowlist: {}",
        included.join(", "),
        record.paths.join(", ")
    );
    let decision = match recorded_decision {
        Some((id, path)) => (id, path),
        None => {
            let decision_input = DecisionInput {
                subject_id: record.id.clone(),
                decision_type: "human_ruling".to_string(),
                topic: Some("chore".to_string()),
                context: Some(format!("Chore {}", record.title)),
                ruling: ruling.clone(),
                // Whoever runs `chore commit` made this call, even if someone else started the
                // chore — a commit-time `--author-*` must not be ignored.
                author: input.author.clone(),
                title: Some(format!("Chore: {}", record.title)),
                timestamp: None,
                validate_subject: false,
            };
            let payload = log_decision(root, input.storage, &decision_input)?;
            let with_decision = ChoreRecord {
                decision_id: Some(payload.id.clone()),
                decision_path: Some(payload.path.clone()),
                ..record.clone()
            };
            // Persisted before the commit is attempted, so a retry finds the ruling already
            // written instead of logging a second one.
            write_record(root, input.storage, &with_decision)?;
            (payload.id, payload.path)
        }
    };

    // The `DEC-` record is the one path committed outside the allowlist (D-5).
    if !to_commit.contains(&decision.1) {
        to_commit.push(decision.1.clone());
    }
    if !to_stage.contains(&decision.1) {
        to_stage.push(decision.1.clone());
    }

    stage_paths(root, &to_stage)?;
    let message = format!(
        "chore: {}\n\nAllowlist: {}",
        record.title,
        record.paths.join(", ")
    );
    let sha = commit_with_pathspec(root, &message, &to_commit)?;

    let closed = ChoreRecord {
        status: "committed".to_string(),
        commit: Some(sha.clone()),
        decision_id: Some(decision.0.clone()),
        decision_path: Some(decision.1.clone()),
        ..record.clone()
    };
    write_record(root, input.storage, &closed)?;

    Ok(ChoreCommitResult {
        chore_id: record.id.clone(),
        committed: true,
        commit: Some(sha),
        included,
        excluded,
        decision_id: Some(decision.0),
        decision_path: Some(decision.1),
    })
}

/// Settles the open chore without a commit, under `status` (`closed` or `abandoned`).
///
/// This is the way out of a chore whose allowlist was wrong, or whose work turned out not to
/// be worth doing: without it the record stays `open` forever, `nothing_to_commit` blocks the
/// commit, and every later `chore start` is refused with `chore_in_progress`. Only the first
/// open record is settled per call — more than one exists only if someone hand-created a
/// second, and running the command again takes the next one.
fn finish_chore(input: &FinishChoreInput, status: &str) -> Result<ChoreRecord, QdevError> {
    let root = input.workspace_root;
    input.author.validate()?;

    // The same lock `start_chore` takes: checking for an open record and settling it must not
    // race a second `chore close`, or both settle the same chore and the second write
    // overwrites the first with a different reason.
    let _lock = acquire_workspace_write_lock(root, input.storage)?;

    let record = match find_open_chore(root, input.storage)? {
        Some(record) => record,
        None => {
            return Err(QdevError::usage_error_with_code(
                "no_active_chore",
                "No open chore found; start one with `qdev chore start` first",
            ));
        }
    };

    // The record stays on disk as history, exactly as a re-used title keeps its earlier record:
    // `qdev chore list` is how anyone sees what this workspace decided to skip.
    let settled = ChoreRecord {
        status: status.to_string(),
        closed_at: Some(current_iso8601()),
        closed_by: Some(input.author.clone()),
        reason: input
            .reason
            .as_ref()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty()),
        ..record.clone()
    };
    write_record(root, input.storage, &settled)?;
    Ok(settled)
}

/// Marks the open chore `closed`: its work was finished some other way, or deliberately
/// stopped. No `DEC-` is written — only `qdev chore commit` records a ruling (D-5).
///
/// The story's rationale is that a chore belongs to no story, so a settled one is a local
/// bookkeeping fact, not a decision anyone else has to be told about.
pub fn close_chore(input: &FinishChoreInput) -> Result<ChoreRecord, QdevError> {
    finish_chore(input, "closed")
}

/// Marks the open chore `abandoned`: the work is not happening. Same contract as
/// [`close_chore`], a different outcome for whoever reads `qdev chore list` later.
pub fn abort_chore(input: &FinishChoreInput) -> Result<ChoreRecord, QdevError> {
    finish_chore(input, "abandoned")
}

/// Stages exactly the given paths. Nothing else is touched, so work that was already in the
/// index stays there and is reported to the user instead of being committed (D-4).
fn stage_paths(workspace_root: &Path, paths: &[String]) -> Result<(), QdevError> {
    let mut command = Command::new("git");
    command.args(["add", "--"]);
    for path in paths {
        command.arg(path);
    }
    let output = command.current_dir(workspace_root).output().map_err(|e| {
        QdevError::infrastructure_failure(
            "git_unavailable",
            format!("Failed to run 'git add': {}", e),
        )
    })?;
    if !output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_add_failed",
            format!(
                "'git add' failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(())
}

/// Commits exactly the given paths with `git commit -- <paths…>`, so nothing staged outside the
/// allowlist can ride along.
fn commit_with_pathspec(
    workspace_root: &Path,
    message: &str,
    paths: &[String],
) -> Result<String, QdevError> {
    let mut command = Command::new("git");
    command.args(["commit", "-m", message, "--"]);
    for path in paths {
        command.arg(path);
    }
    let output = command.current_dir(workspace_root).output().map_err(|e| {
        QdevError::infrastructure_failure(
            "git_unavailable",
            format!("Failed to run 'git commit': {}", e),
        )
    })?;
    if !output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_commit_failed",
            format!(
                "'git commit' failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }

    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to run 'git rev-parse': {}", e),
            )
        })?;
    if !head.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_rev_parse_failed",
            format!(
                "'git rev-parse HEAD' failed: {}",
                String::from_utf8_lossy(&head.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&head.stdout).trim().to_string())
}
