//! Deterministic `qdev next` selection (Story 2.11).
//!
//! Pure over store/lease/identity inputs: [`select_next`] only reads — no lease is created or
//! broken, no entity or cache row is written, no decision record is logged. Candidates are the
//! stories assigned to the resolved sprint scope (`--sprint N`, or every sprint with
//! `status: active`); blocked and leased-elsewhere stories are skipped and recorded as blockers,
//! and survivors are ordered by the `docs/cli-reference.md` §5 chain:
//! `(in_active_sprint desc, sprint id asc, owner_match desc, epic phase, story seq, story id)`.
//! All outputs are independent of cache row or fixture insertion order.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::errors::QdevError;
use crate::governance::{is_user_owner, normalize_team_name};
use crate::lease::{get_lease, StoryLease};
use crate::schema::{extract_frontmatter, EntityKind};
use crate::sprint::sprint_entity_id;
use crate::store::{EntityRecord, SprintRecord, Store};
use crate::write::Author;

/// Story statuses a candidate may be selected in. `done`, `review`, `superseded`, and
/// `abandoned` stories are never picked; readiness gating is Epic 3's concern, so `draft`,
/// `ready`, and `in-progress` are all eligible.
const ELIGIBLE_STATUSES: [&str; 3] = ["draft", "ready", "in-progress"];

/// How `--owner` restricts (or orders) candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NextOwnerFilter {
    /// `--owner me`: candidates must be owned by the current identity — user id, git email,
    /// or a team they belong to (the `governance.rs` ownership helpers plus the
    /// `resolve_author` chain, reused as-is).
    Current,
    /// `--owner <name>`: candidates must carry that literal owner string (a `team:`-prefixed
    /// value or bare team name both match a `team:<name>` owner).
    Literal(String),
}

impl NextOwnerFilter {
    /// True when `owners` satisfies this filter, given the resolved identity `author`.
    /// With no filter, every owner set matches (ownership only reorders, never excludes).
    pub fn matches(
        &self,
        author: &Author,
        config: &Config,
        owners: &[String],
        workspace_root: Option<&Path>,
    ) -> bool {
        match self {
            NextOwnerFilter::Current => is_user_owner(author, config, owners, workspace_root),
            NextOwnerFilter::Literal(literal) => {
                let wanted = normalize_team_name(literal);
                owners
                    .iter()
                    .any(|o| normalize_team_name(o).eq_ignore_ascii_case(&wanted))
            }
        }
    }

    /// The filter description used in `reason`/`blockers` text.
    pub fn describe(&self, identity_id: &str) -> String {
        match self {
            NextOwnerFilter::Current => format!("me (current identity: {identity_id})"),
            NextOwnerFilter::Literal(literal) => literal.clone(),
        }
    }
}

/// Everything [`select_next`] reads from: the cache store, the resolved config, the sprint
/// scope, the owner filter, and the current identity. Nothing here is mutated.
#[derive(Clone)]
pub struct NextOptions<'a> {
    pub workspace_root: &'a Path,
    pub store: &'a dyn Store,
    pub config: &'a Config,
    /// `--sprint N`: select over that sprint's assignments whether or not it is active;
    /// `None` selects over every `status: active` sprint's assignments.
    pub sprint: Option<i64>,
    pub owner: Option<NextOwnerFilter>,
    /// Current identity, resolved through the `resolve_author` chain by the caller.
    pub author: Author,
}

/// A nearest-cause entry explaining why no story was chosen, or why a candidate was skipped.
/// `kind` is one of `blocked`, `leased`, `stale`, `status`, `owner`, `missing`,
/// `no_active_sprints`, or `no_candidates`; `blocking_ids` carries the unmet dependency ids
/// for `blocked` entries, and `holder`/`worktree_path` carry the lease holder for `leased`
/// entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextBlocker {
    /// Absent for workspace-wide entries like "no active sprints"; present per story.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story_id: Option<String>,
    pub kind: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// The story chosen for "what should I work on next?".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextStoryRecord {
    pub id: String,
    /// Always `"story"` — only stories are ever candidates.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: String,
    /// Always `false`: a blocked candidate is skipped, never selected.
    pub blocked: bool,
    pub owners: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appetite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_modules: Option<Vec<String>>,
    /// Story sequence number within the epic; `null` when the cache row carries none.
    pub seq: Option<u32>,
    /// The sprint whose assignment made this story a candidate.
    pub sprint: i64,
    /// That sprint's status (`"active"`, or the explicit non-active status under `--sprint`).
    pub sprint_status: String,
    pub version: u64,
    /// Present only when this worktree holds the lease — "finish what you started".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<StoryLease>,
}

/// The reason block attached to a selection (or to its absence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextReason {
    /// Stable code: `selected`, `no_active_sprints`, `no_candidates`, or `all_filtered`.
    pub code: String,
    /// One human-readable sentence; in the null case it names the nearest reason.
    pub summary: String,
    /// One line per filter applied, for full explanation in text and JSON output.
    pub notes: Vec<String>,
}

/// The full `qdev next` payload: the selected story (or `null`), the reason, and the blockers
/// explaining every filter applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NextSelection {
    pub next: Option<NextStoryRecord>,
    pub reason: NextReason,
    pub blockers: Vec<NextBlocker>,
}

/// Epic `phase` rank per the D-1 decision: numerics ascending, then strings lexicographic,
/// epics without `phase` last; tie by epic id (handled by the caller).
#[derive(Debug, Clone)]
enum PhaseRank {
    Number(f64),
    Text(String),
    Missing,
}

impl PartialEq for PhaseRank {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for PhaseRank {}

impl Ord for PhaseRank {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Number(a), Self::Number(b)) => a.total_cmp(b),
            (Self::Text(a), Self::Text(b)) => a.cmp(b),
            (Self::Missing, Self::Missing) => Ordering::Equal,
            (Self::Number(_), _) | (Self::Text(_), Self::Missing) => Ordering::Less,
            (Self::Text(_), Self::Number(_)) | (Self::Missing, _) => Ordering::Greater,
        }
    }
}

impl PartialOrd for PhaseRank {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// An eligible candidate carried through the comparator.
struct NextCandidate {
    story_id: String,
    record: EntityRecord,
    sprint_id: i64,
    sprint_status: String,
    in_active_sprint: bool,
    owner_match: bool,
    phase: PhaseRank,
    /// `None` sorts after every present seq, so unknown sequence numbers rank last.
    seq: Option<u32>,
    /// Present only when this worktree (and this identity) holds the lease.
    lease: Option<StoryLease>,
}

/// Parses a cache column storing a JSON array of strings (`owners`, `target_modules`).
/// Absent or unparseable columns yield an empty vector — same rule as `query.rs`.
fn parse_json_string_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

/// The total order for eligible candidates: active-sprint membership first, then sprint id,
/// then ownership match, then epic `phase`, then story sequence, tie-broken by story id.
/// Story id as the final key makes the order total, so selection cannot depend on cache row
/// or fixture order.
fn compare_candidates(a: &NextCandidate, b: &NextCandidate) -> Ordering {
    b.in_active_sprint
        .cmp(&a.in_active_sprint)
        .then(a.sprint_id.cmp(&b.sprint_id))
        .then(b.owner_match.cmp(&a.owner_match))
        .then(a.phase.cmp(&b.phase))
        .then_with(
            || match (a.record.epic_id.as_deref(), b.record.epic_id.as_deref()) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(x), Some(y)) => x.cmp(y),
            },
        )
        .then_with(|| a.seq.unwrap_or(u32::MAX).cmp(&b.seq.unwrap_or(u32::MAX)))
        .then(a.story_id.cmp(&b.story_id))
}

/// Reads the epic's `phase` frontmatter for ranking. A story with no epic, an epic whose row
/// is missing or stale, an unreadable epic file, or no `phase` key all collapse to
/// [`PhaseRank::Missing`] — identical to dropping the key (D-1).
fn epic_phase(
    store: &dyn Store,
    root: &Path,
    epic_id: Option<&str>,
) -> Result<PhaseRank, QdevError> {
    let Some(epic_id) = epic_id.filter(|e| !e.is_empty()) else {
        return Ok(PhaseRank::Missing);
    };
    let Some(epic) = store.get_live_entity_for_derivation(epic_id)? else {
        return Ok(PhaseRank::Missing);
    };
    let Ok(content) = std::fs::read_to_string(root.join(&epic.source_path)) else {
        return Ok(PhaseRank::Missing);
    };
    let Ok(frontmatter) = extract_frontmatter(&content) else {
        return Ok(PhaseRank::Missing);
    };
    Ok(match frontmatter.get("phase") {
        None | Some(serde_json::Value::Null) => PhaseRank::Missing,
        Some(serde_json::Value::String(s)) => PhaseRank::Text(s.clone()),
        Some(serde_json::Value::Number(n)) => match n.as_f64() {
            Some(v) => PhaseRank::Number(v),
            None => PhaseRank::Missing,
        },
        // Booleans, sequences, and mappings are not `phase` values under D-1.
        Some(_) => PhaseRank::Missing,
    })
}

/// Deterministically select the next eligible story. Pure: reads only the store, the lease
/// files, and entity frontmatter; never claims, releases, or fabricates a lease, never
/// transitions a story, never writes to `docs/state/`, the cache, or the lease files.
///
/// An explicit `--sprint` whose record is absent from the cache is a `sprint_not_found`
/// usage error (exit 2); everything else is reported through the returned
/// [`NextSelection`] (`next: null` plus `blockers` when nothing is eligible).
pub fn select_next(options: &NextOptions) -> Result<NextSelection, QdevError> {
    let root = options.workspace_root;

    // --- Sprint scope: explicit sprint's assignments, or every live `active` sprint's. ------
    // `resolve_sprint_selection` is deliberately not reused for the default case: multiple
    // active sprints are legal here, and sprint id breaks that tie first.
    let scope: Vec<(i64, SprintRecord)> = match options.sprint {
        Some(n) => match options.store.get_sprint(n)? {
            Some(s) => vec![(n, s)],
            None => {
                return Err(QdevError::usage_error_with_code(
                    "sprint_not_found",
                    format!(
                        "Sprint {n} not found — no sprint record for it in the cache; open or assign it first"
                    ),
                ))
            }
        },
        None => {
            let mut active: Vec<(i64, SprintRecord)> = Vec::new();
            for s in options.store.list_sprints()? {
                if s.status.as_deref() != Some("active") {
                    continue;
                }
                // Live-only: a sprint whose entity row is stale or missing cannot be an
                // active scope (do not offer stale entities).
                if options
                    .store
                    .get_live_entity_for_derivation(&sprint_entity_id(s.id))?
                    .is_some()
                {
                    active.push((s.id, s));
                }
            }
            active.sort_by_key(|(id, _)| *id);
            active
        }
    };

    let scope_note = match options.sprint {
        Some(n) => {
            let status = scope[0]
                .1
                .status
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            if status == "active" {
                format!("explicit scope: sprint {n} (status: active)")
            } else {
                format!(
                    "explicit scope: sprint {n} (status: {status} — selected over its assignments anyway)"
                )
            }
        }
        None if scope.is_empty() => {
            "no active sprints — candidates only come from sprints with status: active".to_string()
        }
        None => {
            let ids: Vec<String> = scope.iter().map(|(id, _)| id.to_string()).collect();
            format!(
                "scope: stories assigned to {} active sprint(s): [{}]",
                scope.len(),
                ids.join(", ")
            )
        }
    };

    // --- Candidate scope: stories in the scope's assignments; stories in no sprint
    //     assignment are never candidates. First sprint (ascending id) wins per story. -------
    let mut candidate_scope: BTreeMap<String, (i64, String)> = BTreeMap::new();
    for (sprint_id, sprint) in &scope {
        let status = sprint
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        for assignment in options.store.get_sprint_assignments(*sprint_id)? {
            candidate_scope
                .entry(assignment.story_id)
                .or_insert((*sprint_id, status.clone()));
        }
    }

    // --- Per-candidate analysis: blocked/leased/stale/status/owner filters, then ranking. ---
    let mut candidates: Vec<NextCandidate> = Vec::new();
    let mut blockers: Vec<NextBlocker> = Vec::new();

    for (story_id, (sprint_id, sprint_status)) in &candidate_scope {
        // Raw read: a retained stale row must be *reported* as a blocker (`stale`), and a
        // missing row as `missing`. Neither is ever a candidate. This is the documented
        // reporting exception to the derive-a-finding rule.
        #[allow(clippy::disallowed_methods)]
        // Reporting: the stale/absent distinction is the point.
        let record = match options.store.get_entity(story_id)? {
            None => {
                blockers.push(NextBlocker {
                    story_id: Some(story_id.clone()),
                    kind: "missing".to_string(),
                    detail: format!(
                        "{story_id} is assigned to sprint {sprint_id} but has no cache row — run `qdev sync` and retry"
                    ),
                    blocking_ids: None,
                    holder: None,
                    worktree_path: None,
                });
                continue;
            }
            Some(record) => record,
        };

        if record.stale {
            blockers.push(NextBlocker {
                story_id: Some(story_id.clone()),
                kind: "stale".to_string(),
                detail: format!(
                    "{story_id}'s cache row is stale (its file failed the last parse) — run `qdev sync` and retry"
                ),
                blocking_ids: None,
                holder: None,
                worktree_path: None,
            });
            continue;
        }

        // Only stories are candidates; a mis-keyed assignment is skipped without a blocker.
        if record.kind != EntityKind::Story {
            continue;
        }

        let status = record.status.clone().unwrap_or_default();
        if !ELIGIBLE_STATUSES.contains(&status.as_str()) {
            blockers.push(NextBlocker {
                story_id: Some(story_id.clone()),
                kind: "status".to_string(),
                detail: match &record.status {
                    Some(s) => format!(
                        "{story_id} has status '{s}' — only draft, ready, or in-progress stories are eligible"
                    ),
                    None => format!(
                        "{story_id} has no status in the cache — only draft, ready, or in-progress stories are eligible"
                    ),
                },
                blocking_ids: None,
                holder: None,
                worktree_path: None,
            });
            continue;
        }

        // Blocked: same semantics as `query.rs` `compute_blocked` / `transition.rs`
        // `validate_dependencies` — any `depends_on` target that is not live-`done`
        // (dangling, stale, or simply not done) blocks the story.
        let relation_rows = options.store.get_relations_for_source(story_id)?;
        let mut blocking_ids: BTreeSet<String> = BTreeSet::new();
        for row in &relation_rows {
            if row.relation != "depends_on" {
                continue;
            }
            let dep_status = options
                .store
                .get_live_entity_for_derivation(&row.target_id)?
                .and_then(|entity| entity.status);
            if dep_status.as_deref() != Some("done") {
                blocking_ids.insert(row.target_id.clone());
            }
        }
        if !blocking_ids.is_empty() {
            let ids: Vec<String> = blocking_ids.iter().cloned().collect();
            blockers.push(NextBlocker {
                story_id: Some(story_id.clone()),
                kind: "blocked".to_string(),
                detail: format!("waiting on {}", ids.join(", ")),
                blocking_ids: Some(ids),
                holder: None,
                worktree_path: None,
            });
            continue;
        }

        // Leased: a story leased in another worktree (or by another holder) is skipped; a
        // story leased by this worktree, by this identity, stays eligible — finish what you
        // started. A lease on disk is an active lease (no TTL).
        let mut own_lease: Option<StoryLease> = None;
        if let Some(lease) = get_lease(root, story_id) {
            let here = {
                let lease_path = Path::new(&lease.worktree_path);
                let canon_lease = lease_path.canonicalize().ok();
                let canon_root = root.canonicalize().ok();
                match (canon_lease, canon_root) {
                    (Some(a), Some(b)) => a == b,
                    (None, None) => lease.worktree_path == root.to_string_lossy(),
                    _ => false,
                }
            };
            let by_me = lease.holder.trim() == options.author.id.trim();
            if !(here && by_me) {
                blockers.push(NextBlocker {
                    story_id: Some(story_id.clone()),
                    kind: "leased".to_string(),
                    detail: format!("held by {} in {}", lease.holder, lease.worktree_path),
                    blocking_ids: None,
                    holder: Some(lease.holder.clone()),
                    worktree_path: Some(lease.worktree_path.clone()),
                });
                continue;
            }
            own_lease = Some(lease);
        }

        // Ownership: with `--owner`, only matching stories survive; without it every owner
        // set is eligible but current-user-or-team matches sort ahead (see the comparator).
        let owners = parse_json_string_array(record.owners.as_deref());
        let owner_match = match &options.owner {
            Some(filter) => filter.matches(&options.author, options.config, &owners, Some(root)),
            None => is_user_owner(&options.author, options.config, &owners, Some(root)),
        };
        if let Some(filter) = &options.owner {
            if !owner_match {
                let owners_text = if owners.is_empty() {
                    "(no declared owners)".to_string()
                } else {
                    owners.join(", ")
                };
                blockers.push(NextBlocker {
                    story_id: Some(story_id.clone()),
                    kind: "owner".to_string(),
                    detail: format!(
                        "owned by {owners_text} — does not match owner filter '{}'",
                        filter.describe(&options.author.id)
                    ),
                    blocking_ids: None,
                    holder: None,
                    worktree_path: None,
                });
                continue;
            }
        }

        let phase = epic_phase(options.store, root, record.epic_id.as_deref())?;

        candidates.push(NextCandidate {
            story_id: story_id.clone(),
            record: record.clone(),
            sprint_id: *sprint_id,
            sprint_status: sprint_status.clone(),
            in_active_sprint: sprint_status == "active",
            owner_match,
            phase,
            seq: record.seq,
            lease: own_lease,
        });
    }

    // `--sprint` with a known but assignment-less sprint, or active sprints that assign
    // nothing: nothing to rank at all.
    if candidate_scope.is_empty() {
        let (code, summary, detail) = match options.sprint {
            Some(n) => (
                "no_candidates",
                format!("Sprint {n} has no assigned stories — nothing to select from it"),
                format!("sprint {n} has no assigned stories — assign stories with `qdev sprint assign` first"),
            ),
            None if scope.is_empty() => (
                "no_active_sprints",
                "No active sprints — candidates only come from sprints with status: active".to_string(),
                "no sprint has status: active — pass --sprint <n> or open a sprint with `qdev sprint open`".to_string(),
            ),
            None => {
                let ids: Vec<String> = scope.iter().map(|(id, _)| id.to_string()).collect();
                (
                    "no_candidates",
                    format!(
                        "No stories are assigned to the active sprints [{}] — assign stories first",
                        ids.join(", ")
                    ),
                    format!(
                        "no stories are assigned to the active sprints [{}] — assign stories with `qdev sprint assign` first",
                        ids.join(", ")
                    ),
                )
            }
        };
        return Ok(NextSelection {
            next: None,
            reason: NextReason {
                code: code.to_string(),
                summary: summary.clone(),
                // `detail` not `summary`: the notes must add the actionable cause,
                // not repeat the sentence the summary already gives.
                notes: vec![scope_note.clone(), detail.clone()],
            },
            blockers: vec![NextBlocker {
                story_id: None,
                kind: code.to_string(),
                detail,
                blocking_ids: None,
                holder: None,
                worktree_path: None,
            }],
        });
    }

    if candidates.is_empty() {
        // Every candidate was filtered (blocked / leased / stale / status / owner / missing).
        blockers.sort_by(|a, b| {
            (&a.story_id, &a.kind, &a.detail).cmp(&(&b.story_id, &b.kind, &b.detail))
        });
        return Ok(NextSelection {
            next: None,
            reason: NextReason {
                code: "all_filtered".to_string(),
                summary: format!(
                    "No eligible story: all {} candidate(s) are blocked, leased, stale, or otherwise not selectable — see blockers",
                    blockers.len()
                ),
                notes: vec![
                    scope_note.clone(),
                    format!(
                        "owner filter: {}",
                        options
                            .owner
                            .as_ref()
                            .map(|f| f.describe(&options.author.id))
                            .unwrap_or_else(|| "none — any ownership is eligible".to_string())
                    ),
                ],
            },
            blockers,
        });
    }

    // --- Selection: total-order sort; the best candidate is always the same answer. ----------
    candidates.sort_by(compare_candidates);
    let best = &candidates[0];

    let title_text = best
        .record
        .title
        .clone()
        .unwrap_or_else(|| best.story_id.clone());
    let summary = match &best.lease {
        Some(lease) => format!(
            "{} \"{}\" is {} — leased by you here ({}); finish what you started",
            best.story_id,
            title_text,
            best.record.status.clone().unwrap_or_default(),
            lease.holder
        ),
        None => format!(
            "{} \"{}\" is {} and unblocked.",
            best.story_id,
            title_text,
            best.record.status.clone().unwrap_or_default()
        ),
    };
    let mut notes = vec![scope_note.clone()];
    let best_owners = parse_json_string_array(best.record.owners.as_deref());
    notes.push(match &options.owner {
        Some(filter) => format!(
            "owner filter: {} — only stories it matches are candidates",
            filter.describe(&options.author.id)
        ),
        // An unowned story counts as a match (`is_user_owner` on empty owners —
        // claimable), so say "claimable", never "owned by the current identity".
        None if best.owner_match && best_owners.is_empty() => {
            "owner: no declared owners — unowned stories are claimable, ranked with current-user/team matches".to_string()
        }
        None if best.owner_match => {
            format!(
                "owner: owned by the current identity ({}) — ranked ahead of other-owned stories",
                options.author.id
            )
        }
        None => {
            format!(
                "owner: {} — ranked below current-user/team matches",
                best_owners.join(", ")
            )
        }
    });
    if candidates.len() > 1 {
        notes.push(format!(
            "{} ranked first among {} eligible candidates",
            best.story_id,
            candidates.len()
        ));
    } else {
        notes.push(format!("{} is the only eligible candidate", best.story_id));
    }
    if best.lease.is_some() {
        notes.push(format!(
            "{} is leased by you here — finish what you started",
            best.story_id
        ));
    }

    blockers
        .sort_by(|a, b| (&a.story_id, &a.kind, &a.detail).cmp(&(&b.story_id, &b.kind, &b.detail)));

    let record = &best.record;
    Ok(NextSelection {
        next: Some(NextStoryRecord {
            id: record.id.clone(),
            kind: record.kind.as_str().to_string(),
            epic_id: record.epic_id.clone(),
            title: record.title.clone(),
            status: record.status.clone().unwrap_or_else(|| "ready".to_string()),
            blocked: false,
            owners: parse_json_string_array(record.owners.as_deref()),
            appetite: record.appetite.clone(),
            safety_class: record.safety_class.clone(),
            target_modules: if record.kind == EntityKind::Story {
                Some(parse_json_string_array(record.target_modules.as_deref()))
            } else {
                None
            },
            seq: record.seq,
            sprint: best.sprint_id,
            sprint_status: best.sprint_status.clone(),
            version: record.version,
            lease: best.lease.clone(),
        }),
        reason: NextReason {
            code: "selected".to_string(),
            summary,
            notes,
        },
        blockers,
    })
}
