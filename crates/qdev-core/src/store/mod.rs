pub mod sqlite;

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::schema::EntityKind;
use crate::write::Author;

pub use sqlite::{
    create_schema, determine_entity_kind, drop_all_user_tables, ensure_cache, inspect_cache_schema,
    newer_cache_conflict, stamp_cache_version, stamp_cannot_resolve_the_edit, CacheSchemaStatus,
    SqliteStore, ALL_TABLE_NAMES, BUSY_TIMEOUT_MS, CACHE_SCHEMA_VERSION, SCHEMA_DDL,
};

/// Common entity record representing rows in the `entities` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// True when the most recent parse of this entity's source file failed (merge conflict or
    /// schema violation) and the retained row may not reflect the current file content.
    pub stale: bool,
    // Story detail fields (kept for compatibility with write layer and convenient access)
    pub epic_id: Option<String>,
    pub seq: Option<u32>,
    pub appetite: Option<String>,
    pub safety_class: Option<String>,
    pub target_modules: Option<String>,
}

/// Whether the cache holds an entity **for the purpose of deriving a finding** — the one place
/// the rule "a stale row is *absent*" is written down.
///
/// Three states rather than a boolean because two derivation sites need to tell `Stale` from
/// `Absent`: the deferred-work checks skip a stale row (its file's own parse failure is already
/// reported) but must still report against a row the cache has no entity for at all, which is a
/// broken cache rather than a known-unparseable file. Everything else wants
/// [`EntityPresence::exists_for_derivation`], which collapses both into "not there".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityPresence {
    /// A row is present and its last parse succeeded: the only state a finding may be derived
    /// from.
    Live,
    /// A row is present but retained `stale` — its file failed its last parse, so every cached
    /// field describes content the file may no longer have.
    Stale,
    /// No row at all.
    Absent,
}

impl EntityPresence {
    /// **The rule, in one function.** `stale` is `None` when the cache holds no row for the id,
    /// `Some(flag)` when it holds one. Every path that answers the derivation question — the
    /// SQL helper on [`Store`] and the row-in-hand [`EntityRecord::presence_for_derivation`] —
    /// goes through here, so the rule cannot be spelled two ways.
    pub fn from_stale_flag(stale: Option<bool>) -> Self {
        match stale {
            None => Self::Absent,
            Some(true) => Self::Stale,
            Some(false) => Self::Live,
        }
    }

    /// True only for [`EntityPresence::Live`]: a stale row does not exist as far as any derived
    /// finding is concerned.
    pub fn exists_for_derivation(self) -> bool {
        matches!(self, Self::Live)
    }
}

impl EntityRecord {
    /// [`EntityPresence`] for a row already in hand, for the checks that iterate
    /// [`Store::list_entities`] and would otherwise pay one query per row to ask the same
    /// question. Same rule, same function — see [`EntityPresence::from_stale_flag`].
    /// Never returns [`EntityPresence::Absent`] — the row is in hand. Callers that only need the
    /// decision should use [`EntityRecord::exists_for_derivation`] and avoid a dead match arm.
    pub fn presence_for_derivation(&self) -> EntityPresence {
        EntityPresence::from_stale_flag(Some(self.stale))
    }

    /// [`Self::presence_for_derivation`] as the boolean the computed checks want: may a finding
    /// be derived from this row?
    pub fn exists_for_derivation(&self) -> bool {
        self.presence_for_derivation().exists_for_derivation()
    }

    pub fn created_by_type(&self) -> Option<&str> {
        self.created_by.as_ref().map(|a| a.author_type.as_str())
    }

    pub fn created_by_id(&self) -> Option<&str> {
        self.created_by.as_ref().map(|a| a.id.as_str())
    }

    pub fn updated_by_type(&self) -> Option<&str> {
        self.updated_by.as_ref().map(|a| a.author_type.as_str())
    }

    pub fn updated_by_id(&self) -> Option<&str> {
        self.updated_by.as_ref().map(|a| a.id.as_str())
    }
}

/// Story details record representing rows in the `stories` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StoryRecord {
    pub id: String,
    pub epic_id: String,
    pub seq: u32,
    pub appetite: Option<String>,
    pub safety_class: Option<String>,
    pub target_modules: Option<String>,
}

/// Constraint record representing rows in the `constraints` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConstraintRecord {
    pub id: String,
    pub owner_id: String,
    pub kind: String,
    pub text: String,
}

/// Relation record representing rows in the `relations` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RelationRecord {
    pub source_id: String,
    pub relation: String,
    pub target_id: String,
}

/// Sprint record representing rows in the `sprints` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SprintRecord {
    pub id: i64,
    pub title: Option<String>,
    pub release_version: Option<String>,
    pub status: Option<String>,
    pub owners: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

/// Sprint assignment record representing rows in the `sprint_assignments` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SprintAssignmentRecord {
    pub sprint_id: i64,
    pub story_id: String,
    pub assigned_at: String,
    pub carried_from: Option<i64>,
}

/// Decision record representing rows in the `decisions` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DecisionRecord {
    pub id: String,
    pub subject_id: String,
    pub decision_type: Option<String>,
    pub topic: Option<String>,
    pub context: Option<String>,
    pub ruling: Option<String>,
    pub author_type: Option<String>,
    pub author_id: Option<String>,
    pub created_at: Option<String>,
}

/// Deferred work record representing rows in the `deferred_work` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeferredWorkRecord {
    pub id: String,
    pub origin_story_id: Option<String>,
    pub target_module: String,
    pub status: Option<String>,
    pub safety_risk: Option<String>,
    pub rationale: Option<String>,
    pub gate: Option<String>,
    pub resolution: Option<String>,
}

/// Scratchpad entry record representing rows in the `scratchpad_entries` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScratchpadRecord {
    pub story_id: String,
    pub seq: u32,
    pub at: String,
    pub author_type: Option<String>,
    pub author_id: Option<String>,
    pub kind: Option<String>,
    pub text: Option<String>,
}

/// Gate record representing rows in the `gates` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GateRecord {
    pub id: String,
    pub command: String,
    pub kind: Option<String>,
    pub timeout_ms: Option<u64>,
    pub output_adapter: Option<String>,
    pub on_transition: Option<String>,
    pub depends_on: Option<String>,
    pub metric: Option<String>,
    pub direction: Option<String>,
}

/// Gate run evidence record representing rows in the `gate_runs` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct GateRunRecord {
    pub id: String,
    pub story_id: Option<String>,
    pub gate_id: String,
    pub commit_sha: String,
    pub status: Option<String>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub metric_value: Option<f64>,
    pub summary: Option<String>,
    pub evidence_path: String,
    pub output_hash: Option<String>,
    pub run_by_type: Option<String>,
    pub run_by_id: Option<String>,
    pub ran_at: Option<String>,
}

/// SOUP dependency record representing rows in the `soup_dependencies` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SoupRecord {
    pub id: String,
    pub name: Option<String>,
    pub version: Option<String>,
    pub license: Option<String>,
    pub cve_status: Option<String>,
    pub introduced_by_story: Option<String>,
    pub evaluated_for_release: Option<String>,
}

/// Sync state record representing rows in the `sync_state` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SyncStateRecord {
    pub path: String,
    pub mtime: i64,
    pub size: u64,
    pub content_hash: Option<String>,
}

/// Dirty entity record representing rows in the `dirty_entities` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DirtyEntityRecord {
    pub id: String,
    pub dirty_at: String,
}

/// Per-path validation finding record representing rows in the `findings` table.
/// Findings are current state: each sweep pass replaces a path's findings, and purging a
/// path removes its findings. A path may hold several findings of the same `code` — one file
/// can have two dangling relations, or take part in two disjoint dependency cycles — so rows
/// are keyed by `(path, code, message)`, not by `(path, code)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingRecord {
    pub path: String,
    pub code: String,
    pub severity: String,
    pub message: Option<String>,
    pub found_at: String,
}

/// Summary of one incremental sweep pass. Field names mirror the planned `qdev sync` counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SweepSummary {
    /// Files re-parsed and upserted this pass.
    pub parsed: usize,
    /// Scanned files whose content hash was unchanged (no re-parse).
    pub unchanged: usize,
    /// Files whose content this pass read and hashed. A warm sweep that skips every file leaves
    /// this at zero: it pins the incremental optimisation, not just its correctness.
    pub hashed: usize,
    /// Files this pass read and tried to hydrate that produced no row: conflicted, schema-invalid,
    /// id-less, or unreadable. Without it the counts do not add up — such a file is neither
    /// `parsed` nor `unchanged`, so a workspace with a broken file reported a total short of the
    /// files it had scanned, and the shortfall was invisible.
    pub retained: usize,
    /// Removed files whose cache rows were purged this pass.
    pub purged: usize,
    /// Total finding rows remaining in the cache after this pass.
    pub findings: usize,
}

/// Query filter for listing entities.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityFilter {
    pub kind: Option<EntityKind>,
    pub status: Option<String>,
    /// Story's owning epic id (joined via the `stories` table).
    pub epic_id: Option<String>,
    /// Exact owner name/team to match within the JSON-array `owners` column.
    pub owner: Option<String>,
    /// Exact module id to match within the JSON-array `target_modules` column (stories only).
    pub module: Option<String>,
    /// Sprint id a story must be assigned to (joined via `sprint_assignments`).
    pub sprint: Option<i64>,
}

/// The unified Store trait exposing reads and writes for all 16 cache tables.
pub trait Store: Send + Sync {
    // Entities
    fn upsert_entity(&self, record: &EntityRecord) -> Result<(), QdevError>;
    /// Returns the row as stored, **stale rows included**. Reads must keep working: `qdev get`
    /// and `qdev list` return a stale entity with its `stale` flag set, which is what retention
    /// exists for. A site that derives a *finding* from an entity's existence must ask
    /// [`Store::entity_exists_for_derivation`] instead — a bare `get_entity(..).is_some()` at
    /// such a site is the defect this pair of methods exists to prevent.
    fn get_entity(&self, id: &str) -> Result<Option<EntityRecord>, QdevError>;
    /// Every row, stale ones included — the same read `get_entity` is, in list form.
    ///
    /// A derivation site iterating these must ask each row
    /// [`EntityRecord::exists_for_derivation`]; a bare iteration treats pre-edit content as
    /// current, which is the defect [`Store::entity_presence_for_derivation`] exists to prevent.
    fn list_entities(&self, filter: &EntityFilter) -> Result<Vec<EntityRecord>, QdevError>;
    fn delete_entity(&self, id: &str) -> Result<bool, QdevError>;

    /// Returns the entity only when its cached row is live, for derived state that needs fields
    /// from one coherent row snapshot. Unlike a presence probe followed by `get_entity`, this
    /// cannot observe the row before and after a concurrent sweep independently. In particular,
    /// a retained stale `done` dependency is absent here and must block its dependent story.
    ///
    /// The stale-inclusive raw read is deliberate inside this helper: it is the one place that
    /// turns its row snapshot into the derivation answer. Callers deriving state must use this
    /// method rather than reading `get_entity` directly.
    #[allow(clippy::disallowed_methods)]
    fn get_live_entity_for_derivation(&self, id: &str) -> Result<Option<EntityRecord>, QdevError> {
        Ok(self.get_entity(id)?.filter(|entity| !entity.stale))
    }

    /// **The one answer to "does this entity exist, for the purpose of deriving a finding?"**
    /// A stale row is [`EntityPresence::Stale`], never `Live`, so it is absent to every derived
    /// finding — which is what makes an incremental sweep (which retains stale rows) and a full
    /// rebuild (which has none) report the same list. Most callers want the boolean
    /// [`Store::entity_exists_for_derivation`]; take the three-state answer only when `Stale`
    /// and `Absent` must be told apart.
    ///
    /// **Provided, not required.** The rule is written down once, here, in terms of
    /// [`Store::get_entity`] — so an implementor cannot spell it differently, which is the
    /// mistake this whole helper exists to make impossible. An implementation may override it
    /// for a cheaper read (`SqliteStore` fetches the flag alone rather than the whole row), but
    /// it must return exactly what this default would.
    #[allow(clippy::disallowed_methods)]
    fn entity_presence_for_derivation(&self, id: &str) -> Result<EntityPresence, QdevError> {
        Ok(EntityPresence::from_stale_flag(
            self.get_entity(id)?.map(|entity| entity.stale),
        ))
    }

    /// [`Store::entity_presence_for_derivation`] as a boolean, for the derivation sites that do
    /// not care *why* an entity is not there. Provided, not required: an implementor cannot get
    /// the rule wrong here.
    fn entity_exists_for_derivation(&self, id: &str) -> Result<bool, QdevError> {
        Ok(self
            .entity_presence_for_derivation(id)?
            .exists_for_derivation())
    }

    // Story details
    fn upsert_story_details(&self, story: &StoryRecord) -> Result<(), QdevError>;
    fn get_story_details(&self, id: &str) -> Result<Option<StoryRecord>, QdevError>;
    fn list_story_details(&self) -> Result<Vec<StoryRecord>, QdevError>;
    fn delete_story_details(&self, id: &str) -> Result<bool, QdevError>;

    // Constraints
    fn upsert_constraint(&self, constraint: &ConstraintRecord) -> Result<(), QdevError>;
    fn get_constraint(&self, id: &str) -> Result<Option<ConstraintRecord>, QdevError>;
    fn get_constraints_for_owner(&self, owner_id: &str)
        -> Result<Vec<ConstraintRecord>, QdevError>;
    fn list_constraints(&self) -> Result<Vec<ConstraintRecord>, QdevError>;
    fn delete_constraint(&self, id: &str) -> Result<bool, QdevError>;

    // Relations
    fn upsert_relation(&self, relation: &RelationRecord) -> Result<(), QdevError>;
    fn get_relations_for_source(&self, source_id: &str) -> Result<Vec<RelationRecord>, QdevError>;
    fn get_relations_for_target(&self, target_id: &str) -> Result<Vec<RelationRecord>, QdevError>;
    fn list_relations(&self) -> Result<Vec<RelationRecord>, QdevError>;
    fn delete_relation(
        &self,
        source_id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, QdevError>;

    // Sprints
    fn upsert_sprint(&self, sprint: &SprintRecord) -> Result<(), QdevError>;
    fn get_sprint(&self, id: i64) -> Result<Option<SprintRecord>, QdevError>;
    fn list_sprints(&self) -> Result<Vec<SprintRecord>, QdevError>;
    fn delete_sprint(&self, id: i64) -> Result<bool, QdevError>;

    // Sprint assignments
    fn upsert_sprint_assignment(
        &self,
        assignment: &SprintAssignmentRecord,
    ) -> Result<(), QdevError>;
    fn get_sprint_assignments(
        &self,
        sprint_id: i64,
    ) -> Result<Vec<SprintAssignmentRecord>, QdevError>;
    fn get_assignments_for_story(
        &self,
        story_id: &str,
    ) -> Result<Vec<SprintAssignmentRecord>, QdevError>;
    fn list_sprint_assignments(&self) -> Result<Vec<SprintAssignmentRecord>, QdevError>;
    fn delete_sprint_assignment(&self, sprint_id: i64, story_id: &str) -> Result<bool, QdevError>;

    // Decisions
    fn upsert_decision(&self, decision: &DecisionRecord) -> Result<(), QdevError>;
    fn get_decision(&self, id: &str) -> Result<Option<DecisionRecord>, QdevError>;
    fn get_decisions_for_subject(&self, subject_id: &str)
        -> Result<Vec<DecisionRecord>, QdevError>;
    fn list_decisions(&self) -> Result<Vec<DecisionRecord>, QdevError>;
    fn delete_decision(&self, id: &str) -> Result<bool, QdevError>;

    // Deferred work
    fn upsert_deferred_work(&self, dw: &DeferredWorkRecord) -> Result<(), QdevError>;
    fn get_deferred_work(&self, id: &str) -> Result<Option<DeferredWorkRecord>, QdevError>;
    fn list_deferred_work(&self) -> Result<Vec<DeferredWorkRecord>, QdevError>;
    fn delete_deferred_work(&self, id: &str) -> Result<bool, QdevError>;

    // Scratchpad entries
    fn upsert_scratchpad_entry(&self, entry: &ScratchpadRecord) -> Result<(), QdevError>;
    fn get_scratchpad_entries(&self, story_id: &str) -> Result<Vec<ScratchpadRecord>, QdevError>;
    fn list_scratchpad_entries(&self) -> Result<Vec<ScratchpadRecord>, QdevError>;
    fn delete_scratchpad_entry(&self, story_id: &str, seq: u32) -> Result<bool, QdevError>;

    // Gates
    fn upsert_gate(&self, gate: &GateRecord) -> Result<(), QdevError>;
    fn get_gate(&self, id: &str) -> Result<Option<GateRecord>, QdevError>;
    fn list_gates(&self) -> Result<Vec<GateRecord>, QdevError>;
    fn delete_gate(&self, id: &str) -> Result<bool, QdevError>;

    // Gate runs
    fn upsert_gate_run(&self, run: &GateRunRecord) -> Result<(), QdevError>;
    fn get_gate_run(&self, id: &str) -> Result<Option<GateRunRecord>, QdevError>;
    fn list_gate_runs(&self) -> Result<Vec<GateRunRecord>, QdevError>;
    fn delete_gate_run(&self, id: &str) -> Result<bool, QdevError>;

    // SOUP dependencies
    fn upsert_soup(&self, soup: &SoupRecord) -> Result<(), QdevError>;
    fn get_soup(&self, id: &str) -> Result<Option<SoupRecord>, QdevError>;
    fn list_soup(&self) -> Result<Vec<SoupRecord>, QdevError>;
    fn delete_soup(&self, id: &str) -> Result<bool, QdevError>;

    // Sync state
    fn upsert_sync_state(
        &self,
        path: &str,
        mtime: i64,
        size: u64,
        hash: Option<&str>,
    ) -> Result<(), QdevError>;
    fn get_sync_state(&self, path: &str) -> Result<Option<SyncStateRecord>, QdevError>;
    fn list_sync_state(&self) -> Result<Vec<SyncStateRecord>, QdevError>;
    fn delete_sync_state(&self, path: &str) -> Result<bool, QdevError>;

    // Dirty entities
    fn mark_entity_dirty(&self, id: &str, dirty_at: &str) -> Result<(), QdevError>;
    fn get_dirty_entities(&self) -> Result<Vec<DirtyEntityRecord>, QdevError>;
    fn clear_dirty_entity(&self, id: &str) -> Result<bool, QdevError>;
    fn clear_all_dirty_entities(&self) -> Result<usize, QdevError>;

    // Findings
    fn upsert_finding(&self, finding: &FindingRecord) -> Result<(), QdevError>;
    /// One finding for `(path, code)`. Since v3 a path can hold several findings of the same
    /// code (distinguished by message), so this returns the lowest-message one; use
    /// `get_findings_for_path` to see them all.
    fn get_finding(&self, path: &str, code: &str) -> Result<Option<FindingRecord>, QdevError>;
    fn get_findings_for_path(&self, path: &str) -> Result<Vec<FindingRecord>, QdevError>;
    fn list_findings(&self) -> Result<Vec<FindingRecord>, QdevError>;
    fn delete_findings_for_path(&self, path: &str) -> Result<usize, QdevError>;

    /// Returns the ISO8601 timestamp of the most recent successful sweep or rebuild, or `None`
    /// if the cache has never been synced (a freshly created schema with an empty `sync_meta`).
    fn get_last_synced_at(&self) -> Result<Option<String>, QdevError>;

    // Schema introspection

    /// The open cache database's own `PRAGMA user_version`, read from the connection rather
    /// than reported back from the binary's compiled-in `CACHE_SCHEMA_VERSION`. Diagnostics
    /// need the observed value to detect a stale or half-migrated cache at all.
    fn cache_schema_version(&self) -> Result<u32, QdevError>;

    /// Names from `ALL_TABLE_NAMES` that are absent from the open cache database, sorted. Empty
    /// for a healthy cache; non-empty means a half-migrated one that a rebuild would heal.
    fn cache_missing_tables(&self) -> Result<Vec<String>, QdevError>;

    // Incremental sweep
    fn sweep_workspace(
        &self,
        workspace_root: &Path,
        storage: &StorageConfig,
    ) -> Result<SweepSummary, QdevError>;
}
