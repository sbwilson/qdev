//! The workspace pulse behind the default command (Story 2.12).
//!
//! `qdev` / `qdev status` compose cache-native store reads, a bounded set of **local**
//! `git` subprocess probes, this-worktree lease records, and [`select_next`] (Story 2.11,
//! reused as-is) into one payload. Everything here is read-only: no cache row, entity file,
//! lease file, or decision record is written, and no advisory lock is taken.
//!
//! A workspace with no readable cache — missing, never stamped, stamped at an older
//! version, or missing a table — is **reported** rather than repaired: [`build_pulse`]
//! carries `schema_status: "mismatch"` with null counts, no sprint blocks, and no
//! recommendation instead of erroring, and nothing here creates, rebuilds, or sweeps a
//! cache to close that gap.
//!
//! Every git probe degrades its field to `null` on failure — a git failure never fails the
//! command. The remote is never contacted: only the network-free probes listed in
//! [`build_pulse`] run, and no remote-update operation is ever issued.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::errors::QdevError;
use crate::lease::{list_leases, parse_iso8601_to_timestamp, StoryLease};
use crate::next::{select_next, NextOptions, NextSelection};
use crate::sprint::sprint_entity_id;
use crate::store::{EntityFilter, Store, CACHE_SCHEMA_VERSION};
use crate::write::Author;

// ---------------------------------------------------------------------------
// Payload types
// ---------------------------------------------------------------------------

/// The working-tree probe section of [`EnvironmentPulse`].
///
/// A non-git directory yields `None` for this whole section (a `null` marker in JSON);
/// a git directory where one individual probe fails keeps the section with that field
/// `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingTreeStatus {
    /// Always `true` when this section is present — it exists only while git reports a
    /// working tree.
    pub git_repository: bool,
    /// `false` when `dirty_files` is above zero; `null` when the status probe failed.
    pub clean: Option<bool>,
    /// Line count of `git status --porcelain`; `null` when the probe failed.
    pub dirty_files: Option<u32>,
    /// `git rev-parse --abbrev-ref HEAD`; `null` when the probe failed.
    pub branch: Option<String>,
    /// `git rev-parse --short HEAD`; `null` when the probe failed.
    pub head: Option<String>,
}

/// The integration-sync section of [`EnvironmentPulse`]: `config.git` plus the
/// `rev-list --left-right --count` answer for `<integration>...<remote>/<integration>`.
/// `None` (a `null` marker) when git itself is unavailable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationStatus {
    pub remote: String,
    /// The configured integration branch (`config.git.integration_branch`).
    pub branch: String,
    /// One of `up_to_date`, `behind`, `ahead`, `diverged`, `integration_branch_missing`,
    /// `remote_ref_missing`, `refs_missing`, or `unknown`. Missing local refs produce a
    /// state that *names them* — no remote update is ever attempted to "help".
    pub state: String,
    /// `None` unless `state` is count-based (`up_to_date`/`ahead`/`behind`/`diverged`).
    pub ahead: Option<u32>,
    /// `None` unless `state` is count-based.
    pub behind: Option<u32>,
}

/// Cache health from cache-native reads only — the `doctor` cache section's pattern:
/// schema status, entity count, the `findings` table count, and sync age. A failed read
/// reports `null` beside the `schema_status` that explains it; the command still succeeds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheHealth {
    /// `ok` when the observed cache schema version matches this binary and no table is
    /// missing; `mismatch` otherwise.
    pub schema_status: String,
    /// `None` when the `entities` count could not be read.
    pub entity_count: Option<u32>,
    /// `None` when the `findings` count could not be read.
    pub finding_count: Option<u32>,
    /// Milliseconds between the last recorded sync and `now`; `None` when the cache has
    /// never been synced or the timestamp is unparseable.
    pub synced_ms_ago: Option<u64>,
}

/// One this-worktree lease rendered in the Environment section. Multiple leases held by
/// this worktree render one entry each, sorted by story id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseInfo {
    pub story_id: String,
    pub holder: String,
    pub started_at: String,
    pub branch: String,
}

/// The Environment section: working tree, integration state, cache health, and the
/// leases held by **this worktree** only (`lease: null` when none).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPulse {
    /// `null` in a non-git directory.
    pub working_tree: Option<WorkingTreeStatus>,
    /// `null` in a non-git directory.
    pub integration: Option<IntegrationStatus>,
    pub cache: CacheHealth,
    /// Leases held by this worktree only (canonical path comparison, the `next.rs`
    /// pattern); `None` when none.
    pub lease: Option<Vec<LeaseInfo>>,
}

/// The four fixed, mutually exclusive story buckets (decided D-2): `done` first; then
/// `in_progress` (status `in-progress`, not computed-blocked); then `blocked` (not `done`,
/// computed-blocked by the same `depends_on` live-`done` semantics as `next.rs`);
/// `backlog` is everything else assigned. The four sum to the assigned-story count
/// (live-only rows; stale rows are excluded everywhere).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryCounters {
    pub done: u32,
    pub in_progress: u32,
    pub blocked: u32,
    pub backlog: u32,
}

/// Workspace-wide deferred-work debt shown in each active sprint block (decided D-3):
/// the count of `status: open` DW and how many of those carry `safety_risk:
/// unacceptable`. Per-sprint detail stays `qdev dw list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredWorkCounts {
    pub open: u32,
    pub unacceptable: u32,
}

/// One active sprint's pulse block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SprintPulse {
    pub id: i64,
    pub title: Option<String>,
    pub release: Option<String>,
    /// Always `active` — only active sprints with a live entity row get a block.
    pub status: String,
    pub stories: StoryCounters,
    pub deferred_work: DeferredWorkCounts,
}

/// Gate evidence summary: `N/M passing` computed from the most recent run per gate.
/// Rendered only while `gate_runs` rows exist (decided D-4 — no ratchet clause until
/// Epic 3); otherwise `None` (omitted in text, `null` in JSON).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSummary {
    pub passing: u32,
    pub total: u32,
}

/// The full pulse payload: the workspace flag; the environment section; one block per
/// active sprint; the gates summary; and the `select_next` selection embedded verbatim.
/// Outside a workspace everything except `workspace` is `null` (decided D-1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PulsePayload {
    pub workspace: bool,
    pub environment: Option<EnvironmentPulse>,
    pub sprints: Option<Vec<SprintPulse>>,
    pub gates: Option<GateSummary>,
    pub next: Option<NextSelection>,
}

// ---------------------------------------------------------------------------
// Bounded, local git probes — each degrades to `None` on failure
// ---------------------------------------------------------------------------

/// Runs one local `git` probe in `root` and returns its trimmed stdout, or `None` when
/// the command could not be spawned or exited non-zero — a failed probe degrades its
/// field, it never fails the command. Output is decoded lossily, so non-UTF8 stdout
/// renders as U+FFFD rather than degrading the field. There is no timeout: these are
/// local reads on the working copy, and the probe set — not a deadline — is what keeps
/// the pulse inside its budget. Only ever invoked with the local-only argument vectors
/// from [`build_pulse`].
fn git_probe(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// True when `lease` is held in the worktree rooted at `root` — the canonical-path
/// comparison `next.rs` applies.
fn held_by_this_worktree(lease: &StoryLease, root: &Path) -> bool {
    let lease_path = Path::new(&lease.worktree_path);
    match (lease_path.canonicalize().ok(), root.canonicalize().ok()) {
        (Some(a), Some(b)) => a == b,
        (None, None) => lease.worktree_path == root.to_string_lossy(),
        _ => false,
    }
}

/// The integration line's state and counts, read only from local refs:
/// `rev-list --left-right --count <integration>...<remote>/<integration>` yields
/// `up_to_date`/`behind`/`ahead`/`diverged`; a failed count names which refs are
/// missing (`integration_branch_missing`, `remote_ref_missing`, `refs_missing`) via local
/// `rev-parse --verify` probes — no remote update, ever.
pub fn integration_status(
    root: &Path,
    remote: &str,
    integration_branch: &str,
    git_available: bool,
) -> Option<IntegrationStatus> {
    if !git_available {
        return None;
    }
    let range = format!("{integration_branch}...{remote}/{integration_branch}");
    if let Some(counts) = git_probe(root, &["rev-list", "--left-right", "--count", &range]) {
        let mut parts = counts.split_whitespace();
        let ahead = parts.next().and_then(|v| v.parse::<u32>().ok());
        let behind = parts.next().and_then(|v| v.parse::<u32>().ok());
        if let (Some(a), Some(b)) = (ahead, behind) {
            let state = match (a, b) {
                (0, 0) => "up_to_date",
                (0, _) => "behind",
                (_, 0) => "ahead",
                _ => "diverged",
            };
            return Some(IntegrationStatus {
                remote: remote.to_string(),
                branch: integration_branch.to_string(),
                state: state.to_string(),
                ahead: Some(a),
                behind: Some(b),
            });
        }
    }
    // The count failed: name which refs are missing, from local presence only.
    let local = git_probe(
        root,
        &["rev-parse", "--verify", "--quiet", integration_branch],
    );
    let remote_ref = format!("{remote}/{integration_branch}");
    let remote_ok = git_probe(root, &["rev-parse", "--verify", "--quiet", &remote_ref]);
    let state = match (local.is_some(), remote_ok.is_some()) {
        (false, false) => "refs_missing",
        (false, true) => "integration_branch_missing",
        (true, false) => "remote_ref_missing",
        (true, true) => "unknown",
    };
    Some(IntegrationStatus {
        remote: remote.to_string(),
        branch: integration_branch.to_string(),
        state: state.to_string(),
        ahead: None,
        behind: None,
    })
}

// ---------------------------------------------------------------------------
// build_pulse — pure over store/config/git/leases
// ---------------------------------------------------------------------------

/// Everything [`build_pulse`] reads: the workspace flag, its root, the cache store
/// (`None` outside a workspace, and inside one whenever no readable cache exists — none
/// may be created or repaired), the resolved config, the current identity, and `now` for
/// the sync-age field (fixed in tests, wall-clock in the CLI).
pub struct PulseOptions<'a> {
    /// Whether a `qdev.toml` was found. `false` short-circuits to the all-null payload.
    pub workspace: bool,
    pub workspace_root: &'a Path,
    /// The cache to read, or `None` when there is nothing readable — the pulse then
    /// reports the cache as degraded instead of creating or rebuilding one.
    pub store: Option<&'a dyn Store>,
    pub config: &'a Config,
    /// Current identity, resolved through the `resolve_author` chain by the caller.
    pub author: Author,
    pub now: SystemTime,
}

/// True when `story_id` is computed-blocked: any `depends_on` target that is not
/// live-`done` (dangling, stale, or simply not done) blocks it — the same semantics
/// `next.rs` applies, so the pulse's `blocked` bucket and `select_next` cannot disagree.
fn computed_blocked(store: &dyn Store, story_id: &str) -> Result<bool, QdevError> {
    for row in store.get_relations_for_source(story_id)? {
        if row.relation != "depends_on" {
            continue;
        }
        let dep_done = store
            .get_live_entity_for_derivation(&row.target_id)?
            .and_then(|entity| entity.status)
            .is_some_and(|status| status == "done");
        if !dep_done {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Builds the workspace pulse. Pure and read-only: reads the cache store, the resolved
/// config, the bounded local git probes, and the lease files; writes nothing, takes no
/// lock, never claims or releases a lease, never runs a sweep or a cache rebuild, and
/// never mutates story or sprint state.
///
/// Outside a workspace (`options.workspace == false`) it short-circuits to the D-1
/// payload — `workspace: false` with every other field `null` — before touching any
/// probe, so no cache file can be opened, let alone created.
///
/// Inside a workspace with `options.store == None` — no cache, or one whose stamp this
/// binary cannot read — the cache is *reported*, never repaired: `sprints` is
/// `Some(vec![])`, `gates` and `next` are `None`, and the cache section is
/// `schema_status: "mismatch"` with every count `null`. Nothing is created to fill the
/// gap, and `select_next` cannot run without a store, so nothing is recommended until
/// the user runs `qdev sync`. Git probes, the workspace flag, and the lease read all
/// still work normally.
pub fn build_pulse(options: &PulseOptions<'_>) -> Result<PulsePayload, QdevError> {
    if !options.workspace {
        return Ok(PulsePayload {
            workspace: false,
            environment: None,
            sprints: None,
            gates: None,
            next: None,
        });
    }

    let root = options.workspace_root;

    // --- Environment: working tree, integration, cache health, this-worktree leases. ---
    // None of these need a store, so an uninitialized or unreadable cache still gets its
    // git and lease lines.
    let dirty_files = git_probe(root, &["status", "--porcelain"]).map(|out| {
        if out.is_empty() {
            0
        } else {
            out.lines().count() as u32
        }
    });
    let branch = git_probe(root, &["rev-parse", "--abbrev-ref", "HEAD"]);
    let head = git_probe(root, &["rev-parse", "--short", "HEAD"]);
    // The working tree exists only while the status probe succeeds; every field then
    // degrades individually to `null` rather than failing the command.
    let working_tree = dirty_files.map(|dirty| WorkingTreeStatus {
        git_repository: true,
        clean: Some(dirty == 0),
        dirty_files: Some(dirty),
        branch: branch.clone(),
        head: head.clone(),
    });
    let integration = integration_status(
        root,
        &options.config.git.remote,
        &options.config.git.integration_branch,
        dirty_files.is_some(),
    );

    // Cache health: cache-native reads only, mirroring the doctor cache section. With no
    // readable cache there is nothing to read — report `mismatch` with null counts
    // (never create or rebuild one to fill the gap).
    let cache = match options.store {
        None => CacheHealth {
            schema_status: "mismatch".to_string(),
            entity_count: None,
            finding_count: None,
            synced_ms_ago: None,
        },
        Some(store) => {
            let observed_schema_version = store.cache_schema_version()?;
            let missing_tables = store.cache_missing_tables()?;
            let schema_status =
                if observed_schema_version == CACHE_SCHEMA_VERSION && missing_tables.is_empty() {
                    "ok"
                } else {
                    "mismatch"
                };
            #[allow(clippy::disallowed_methods)]
            // The pulse reports retained rows rather than deriving state from them — same rule as the doctor cache section.
            let entity_count = store
                .list_entities(&EntityFilter::default())
                .map(|entities| entities.len() as u32)
                .ok();
            let finding_count = store
                .list_findings()
                .map(|findings| findings.len() as u32)
                .ok();
            let synced_ms_ago = store
                .get_last_synced_at()
                .ok()
                .flatten()
                .and_then(|ts| parse_iso8601_to_timestamp(&ts))
                .and_then(|synced| {
                    // `parse_iso8601_to_timestamp` yields epoch **seconds** while `now` is
                    // milliseconds — convert before subtracting or the age is nonsense.
                    let now_ms = options
                        .now
                        .duration_since(UNIX_EPOCH)
                        .ok()
                        .map(|d| d.as_millis() as i64)?;
                    Some((now_ms - synced * 1000).max(0) as u64)
                });
            CacheHealth {
                schema_status: schema_status.to_string(),
                entity_count,
                finding_count,
                synced_ms_ago,
            }
        }
    };

    // Leases held by this worktree only, sorted by story id (`list_leases` is a
    // BTreeMap-backed read, so insertion order cannot leak).
    let all_leases = list_leases(root)?;
    let own_leases: Vec<LeaseInfo> = all_leases
        .iter()
        .filter(|lease| held_by_this_worktree(lease, root))
        .map(|lease| LeaseInfo {
            story_id: lease.story_id.clone(),
            holder: lease.holder.clone(),
            started_at: lease.started_at.clone(),
            branch: lease.branch.clone(),
        })
        .collect();
    let lease = (!own_leases.is_empty()).then_some(own_leases);

    let environment = EnvironmentPulse {
        working_tree,
        integration,
        cache,
        lease,
    };

    // Everything past this point reads cache rows: with no readable cache there are no
    // sprint rows to list, no gate evidence, and no selection to make — `select_next`
    // needs the store. Report and return.
    let Some(store) = options.store else {
        return Ok(PulsePayload {
            workspace: true,
            environment: Some(environment),
            sprints: Some(Vec::new()),
            gates: None,
            next: None,
        });
    };

    // --- Sprints: active sprints with a live entity row, sorted by sprint id. ----------
    let mut active_sprints = Vec::new();
    for sprint in store.list_sprints()? {
        if sprint.status.as_deref() != Some("active") {
            continue;
        }
        // Live-only rule (`next.rs`): a sprint whose entity row is stale or missing
        // cannot be an active block.
        if store
            .get_live_entity_for_derivation(&sprint_entity_id(sprint.id))?
            .is_some()
        {
            active_sprints.push(sprint);
        }
    }
    active_sprints.sort_by_key(|sprint| sprint.id);

    // Deferred-work debt is workspace-wide (decided D-3) and rendered in every block.
    let dw_rows = store.list_deferred_work()?;
    let open_dw: Vec<&_> = dw_rows
        .iter()
        .filter(|dw| dw.status.as_deref() == Some("open"))
        .collect();
    let deferred_work = DeferredWorkCounts {
        open: open_dw.len() as u32,
        unacceptable: open_dw
            .iter()
            .filter(|dw| dw.safety_risk.as_deref() == Some("unacceptable"))
            .count() as u32,
    };

    let mut sprints = Vec::new();
    for sprint in active_sprints {
        // BTreeSet: assignment-row order cannot shift a story between buckets.
        let story_ids: BTreeSet<String> = store
            .get_sprint_assignments(sprint.id)?
            .into_iter()
            .map(|assignment| assignment.story_id)
            .collect();

        let mut counters = StoryCounters {
            done: 0,
            in_progress: 0,
            blocked: 0,
            backlog: 0,
        };
        for story_id in story_ids {
            // Live-only: stale or missing rows are excluded from every counter.
            let Some(record) = store.get_live_entity_for_derivation(&story_id)? else {
                continue;
            };
            // Only stories count; a mis-keyed assignment contributes nothing.
            if record.kind != crate::schema::EntityKind::Story {
                continue;
            }
            let status = record.status.clone().unwrap_or_default();
            if status == "done" {
                counters.done += 1;
            } else if computed_blocked(store, &story_id)? {
                counters.blocked += 1;
            } else if status == "in-progress" {
                counters.in_progress += 1;
            } else {
                counters.backlog += 1;
            }
        }

        sprints.push(SprintPulse {
            id: sprint.id,
            title: sprint.title.clone(),
            release: sprint.release_version.clone(),
            // The filter guarantees `Some("active")`.
            status: sprint
                .status
                .clone()
                .unwrap_or_else(|| "active".to_string()),
            stories: counters,
            deferred_work,
        });
    }

    // --- Gates: most recent run per gate, only while `gate_runs` rows exist (D-4). ----
    let gate_runs = store.list_gate_runs()?;
    let gates = if gate_runs.is_empty() {
        None
    } else {
        // Most recent run per gate: `(ran_at, id)` ordering, so a tie on time still
        // picks the same run. `None` times sort first (oldest).
        let mut latest: BTreeMap<&str, &crate::store::GateRunRecord> = BTreeMap::new();
        for run in &gate_runs {
            match latest.get(run.gate_id.as_str()) {
                Some(current) => {
                    let current_key = (
                        current.ran_at.as_deref().unwrap_or_default(),
                        current.id.as_str(),
                    );
                    let candidate_key =
                        (run.ran_at.as_deref().unwrap_or_default(), run.id.as_str());
                    if candidate_key > current_key {
                        latest.insert(run.gate_id.as_str(), run);
                    }
                }
                None => {
                    latest.insert(run.gate_id.as_str(), run);
                }
            }
        }
        let total = latest.len() as u32;
        let passing = latest
            .values()
            .filter(|run| run.status.as_deref() == Some("pass"))
            .count() as u32;
        Some(GateSummary { passing, total })
    };

    // --- Next: `select_next` with default scope and the resolved current identity. -----
    let next_options = NextOptions {
        workspace_root: root,
        store,
        config: options.config,
        sprint: None,
        owner: None,
        author: options.author.clone(),
    };
    let next = Some(select_next(&next_options)?);

    Ok(PulsePayload {
        workspace: true,
        environment: Some(environment),
        sprints: Some(sprints),
        gates,
        next,
    })
}
