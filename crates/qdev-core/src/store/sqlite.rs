use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::id::{Identifier, IdentifierKind};
use crate::schema::{extract_frontmatter, validate_value_detailed, EntityKind, ValidationError};
use crate::store::{
    ConstraintRecord, DecisionRecord, DeferredWorkRecord, DirtyEntityRecord, EntityFilter,
    EntityPresence, EntityRecord, FindingRecord, GateRecord, GateRunRecord, RelationRecord,
    ScratchpadRecord, SoupRecord, SprintAssignmentRecord, SprintRecord, Store, StoryRecord,
    SweepSummary, SyncStateRecord,
};
use crate::write::Author;

// v3 widened the `findings` primary key from (path, code) to (path, code, message_key): the
// narrower key silently collapsed a path's several dangling relations, or its membership in
// two disjoint dependency cycles, down to whichever row was written last. A v2 cache is
// therefore rebuilt on first open rather than migrated in place.
//
// `PRAGMA user_version` is the *only* version stamp this cache carries. `PRAGMA schema_version`
// is SQLite's internal schema cookie — auto-incremented on every DDL statement and used by
// SQLite to invalidate other connections' prepared statements — so it is neither application
// owned nor safe to write. Cache validity is `user_version` plus the table-presence and column
// checks in `inspect_cache_schema`. A second version dimension, if ever wanted, belongs in
// `sync_meta` as an ordinary row.
pub const CACHE_SCHEMA_VERSION: u32 = 3;
pub const BUSY_TIMEOUT_MS: u64 = 5000;

pub const ALL_TABLE_NAMES: &[&str] = &[
    "entities",
    "stories",
    "constraints",
    "relations",
    "sprints",
    "sprint_assignments",
    "decisions",
    "deferred_work",
    "scratchpad_entries",
    "gates",
    "gate_runs",
    "soup_dependencies",
    "sync_state",
    "dirty_entities",
    "findings",
    "sync_meta",
];

pub const SCHEMA_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS entities (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT,
    status TEXT,
    owners TEXT,
    source_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    version INTEGER,
    created_by_type TEXT,
    created_by_id TEXT,
    updated_by_type TEXT,
    updated_by_id TEXT,
    updated_at TEXT,
    stale INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS stories (
    id TEXT PRIMARY KEY REFERENCES entities(id),
    epic_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    appetite TEXT CHECK(appetite IN ('tiny','small','medium','deep')),
    safety_class TEXT CHECK(safety_class IN ('ClassA','ClassB','ClassC')),
    target_modules TEXT
);

CREATE TABLE IF NOT EXISTS constraints (
    id TEXT PRIMARY KEY,
    owner_id TEXT NOT NULL,
    kind TEXT CHECK(kind IN ('no_go','rabbit_hole','appetite')),
    text TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS relations (
    source_id TEXT NOT NULL,
    relation TEXT NOT NULL,
    target_id TEXT NOT NULL,
    PRIMARY KEY (source_id, relation, target_id)
);

CREATE TABLE IF NOT EXISTS sprints (
    id INTEGER PRIMARY KEY,
    title TEXT,
    release_version TEXT,
    status TEXT CHECK(status IN ('planning','active','completed','paused','abandoned')),
    owners TEXT,
    started_at TEXT,
    completed_at TEXT
);

CREATE TABLE IF NOT EXISTS sprint_assignments (
    sprint_id INTEGER REFERENCES sprints(id),
    story_id TEXT NOT NULL,
    assigned_at TEXT NOT NULL,
    carried_from INTEGER,
    PRIMARY KEY (sprint_id, story_id)
);

CREATE TABLE IF NOT EXISTS decisions (
    id TEXT PRIMARY KEY,
    subject_id TEXT NOT NULL,
    decision_type TEXT CHECK(decision_type IN
      ('human_ruling','agent_assumption','cross_team_override','pivot','review_rejection','lease_override')),
    topic TEXT,
    context TEXT,
    ruling TEXT,
    author_type TEXT,
    author_id TEXT,
    created_at TEXT
);

CREATE TABLE IF NOT EXISTS deferred_work (
    id TEXT PRIMARY KEY,
    origin_story_id TEXT,
    target_module TEXT NOT NULL,
    status TEXT CHECK(status IN ('open','done','wont_fix')),
    safety_risk TEXT CHECK(safety_risk IN ('negligible','acceptable_with_mitigation','unacceptable')),
    rationale TEXT,
    gate TEXT,
    resolution TEXT
);

CREATE TABLE IF NOT EXISTS scratchpad_entries (
    story_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    at TEXT NOT NULL,
    author_type TEXT,
    author_id TEXT,
    kind TEXT,
    text TEXT,
    PRIMARY KEY (story_id, seq)
);

CREATE TABLE IF NOT EXISTS gates (
    id TEXT PRIMARY KEY,
    command TEXT NOT NULL,
    kind TEXT CHECK(kind IN ('check','ratchet')),
    timeout_ms INTEGER,
    output_adapter TEXT,
    on_transition TEXT,
    depends_on TEXT,
    metric TEXT,
    direction TEXT
);

CREATE TABLE IF NOT EXISTS gate_runs (
    id TEXT PRIMARY KEY,
    story_id TEXT,
    gate_id TEXT NOT NULL,
    commit_sha TEXT NOT NULL,
    status TEXT CHECK(status IN ('pass','fail','infra')),
    exit_code INTEGER,
    duration_ms INTEGER,
    metric_value REAL,
    summary TEXT,
    evidence_path TEXT NOT NULL,
    output_hash TEXT,
    run_by_type TEXT,
    run_by_id TEXT,
    ran_at TEXT
);

CREATE TABLE IF NOT EXISTS soup_dependencies (
    id TEXT PRIMARY KEY,
    name TEXT,
    version TEXT,
    license TEXT,
    cve_status TEXT,
    introduced_by_story TEXT,
    evaluated_for_release TEXT
);

CREATE TABLE IF NOT EXISTS sync_state (
    path TEXT PRIMARY KEY,
    mtime INTEGER,
    size INTEGER,
    content_hash TEXT
);

CREATE TABLE IF NOT EXISTS dirty_entities (
    id TEXT PRIMARY KEY,
    dirty_at TEXT
);

CREATE TABLE IF NOT EXISTS findings (
    path TEXT NOT NULL,
    code TEXT NOT NULL,
    severity TEXT NOT NULL,
    message TEXT,
    message_key TEXT NOT NULL,
    found_at TEXT NOT NULL,
    PRIMARY KEY (path, code, message_key)
);

CREATE TABLE IF NOT EXISTS sync_meta (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    last_synced_at TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSchemaStatus {
    Valid,
    /// The cache is at an older version, or is structurally incomplete (a missing table or a
    /// missing column). Either way it is rebuilt losslessly from the Markdown files.
    Mismatch,
    /// The cache was stamped by a newer binary than this one. Rebuilding would silently discard
    /// whatever that binary recorded, so boot refuses instead; `qdev sync --rebuild` is the
    /// documented recovery path.
    NewerThanSupported {
        found: u32,
        supported: u32,
    },
}

/// SQLite-backed cache implementation conforming to AD-2, AD-3, and AD-4.
pub struct SqliteStore {
    conn: Mutex<rusqlite::Connection>,
    path: Option<PathBuf>,
}

impl SqliteStore {
    /// Opens a SQLite cache database connection at `path`, configuring WAL mode and busy_timeout=5000.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, QdevError> {
        let path_buf = path.as_ref().to_path_buf();
        if let Some(parent) = path_buf.parent().filter(|p| !p.as_os_str().is_empty()) {
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

        let conn = rusqlite::Connection::open(&path_buf).map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!(
                    "Failed to open SQLite database at '{}': {}",
                    path_buf.display(),
                    e
                ),
            )
        })?;

        // Set busy_timeout first so journal_mode update waits if locked
        conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to set busy_timeout={}: {}", BUSY_TIMEOUT_MS, e),
                )
            })?;

        let current_mode: String = conn
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .unwrap_or_default();
        if !current_mode.eq_ignore_ascii_case("wal") {
            conn.pragma_update(None, "journal_mode", "WAL")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to set journal_mode=WAL: {}", e),
                    )
                })?;
        }

        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to enable foreign_keys: {}", e),
                )
            })?;

        Ok(Self {
            conn: Mutex::new(conn),
            path: Some(path_buf),
        })
    }

    /// Opens an in-memory SQLite cache database (useful for fast unit tests).
    pub fn open_in_memory() -> Result<Self, QdevError> {
        let conn = rusqlite::Connection::open_in_memory().map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to open in-memory SQLite database: {}", e),
            )
        })?;

        conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to set busy_timeout={}: {}", BUSY_TIMEOUT_MS, e),
                )
            })?;

        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to enable foreign_keys: {}", e),
                )
            })?;

        // A brand-new in-memory database, so stamping is safe.
        create_schema(&conn)?;
        stamp_cache_version(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
            path: None,
        })
    }

    /// Wraps an existing connection into `SqliteStore`.
    pub fn from_connection(conn: rusqlite::Connection) -> Self {
        Self {
            conn: Mutex::new(conn),
            path: None,
        }
    }

    /// Returns the database path if file-backed.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn with_conn<F, R>(&self, f: F) -> Result<R, QdevError>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<R, QdevError>,
    {
        let conn = self.conn.lock().map_err(|e| {
            QdevError::infrastructure_failure(
                "mutex_poisoned",
                format!("SQLite connection mutex was poisoned: {}", e),
            )
        })?;
        f(&conn)
    }

    pub fn with_conn_mut<F, R>(&self, f: F) -> Result<R, QdevError>
    where
        F: FnOnce(&mut rusqlite::Connection) -> Result<R, QdevError>,
    {
        let mut conn = self.conn.lock().map_err(|e| {
            QdevError::infrastructure_failure(
                "mutex_poisoned",
                format!("SQLite connection mutex was poisoned: {}", e),
            )
        })?;
        f(&mut conn)
    }

    /// Drops every table and trigger, recreates the schema, repopulates the cache rows from the
    /// workspace's Markdown entity files, and stamps `PRAGMA user_version` to
    /// `CACHE_SCHEMA_VERSION`.
    ///
    /// Deliberately does **not** take the advisory write lock: `ensure_cache` already holds it
    /// when it calls this on a mismatched cache, so acquiring it here would deadlock. Any
    /// caller reaching this without that guard — `qdev sync --rebuild` is the only one — must
    /// take the lock itself, or the drop-and-repopulate can interleave with a concurrent write.
    pub fn reset_and_rebuild(
        &self,
        workspace_root: &Path,
        storage: &StorageConfig,
    ) -> Result<SweepSummary, QdevError> {
        self.with_conn_mut(|conn| {
            drop_all_user_tables(conn)?;
            create_schema(conn)?;
            Ok(())
        })?;

        let summary = self.rebuild_from_workspace(workspace_root, storage)?;

        // The tables were just dropped and recreated, so stamping the current version is safe.
        // Routed through `stamp_cache_version` so there is exactly one place that writes it.
        self.with_conn(stamp_cache_version)?;

        Ok(summary)
    }

    /// Scans the workspace specification and state directories and rebuilds all cache tables.
    /// Shares the per-file parse/upsert/finding logic with `sweep_workspace` so a full rebuild
    /// and a sweep of the same tree converge on identical state.
    pub fn rebuild_from_workspace(
        &self,
        workspace_root: &Path,
        storage: &StorageConfig,
    ) -> Result<SweepSummary, QdevError> {
        // Collect all file paths to parse in deterministic sorted order
        let mut entity_files = Vec::new();

        let specs_dir = workspace_root.join(&storage.specs_dir);
        let state_dir = workspace_root.join(&storage.state_dir);

        collect_markdown_files(&specs_dir, &mut entity_files);
        collect_markdown_files(&state_dir, &mut entity_files);

        // Sort by path for deterministic rebuild order
        entity_files.sort();
        // Deduped because `specs_dir` and `state_dir` may overlap (nothing rejects a layout
        // where one contains the other), and a file collected twice would be hydrated twice and
        // double-counted. The duplicate-id scan dedups the same way, so all three walks agree.
        entity_files.dedup();

        // Also collect scratchpad (.jsonl) and evidence (.json) files
        let scratch_dir = state_dir.join("scratch");
        let mut scratch_files = Vec::new();
        collect_files_with_ext(&scratch_dir, "jsonl", &mut scratch_files);
        scratch_files.sort();

        let evidence_dir = state_dir.join("evidence");
        let mut evidence_files = Vec::new();
        collect_files_with_ext(&evidence_dir, "json", &mut evidence_files);
        evidence_files.sort();

        let config_path = workspace_root.join("qdev.toml");

        // Rebuild within a single transaction
        self.with_conn_mut(|conn| {
            let tx = conn.transaction().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to begin rebuild transaction: {}", e),
                )
            })?;

            // Clear all data
            tx.execute_batch(
                r#"
DELETE FROM gate_runs;
DELETE FROM gates;
DELETE FROM scratchpad_entries;
DELETE FROM deferred_work;
DELETE FROM decisions;
DELETE FROM sprint_assignments;
DELETE FROM sprints;
DELETE FROM relations;
DELETE FROM constraints;
DELETE FROM stories;
DELETE FROM soup_dependencies;
DELETE FROM dirty_entities;
DELETE FROM sync_state;
DELETE FROM entities;
DELETE FROM findings;
DELETE FROM sync_meta;
"#,
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to clear tables for rebuild: {}", e),
                )
            })?;

            // `parsed` counts only confirmed successful reads + hydrations this pass, matching
            // the meaning `sweep_workspace` gives `SweepSummary.parsed`.
            let mut parsed = 0usize;
            // Files whose content this pass read and hashed. A rebuild reads everything it
            // finds; the sweep counts the same thing, which is how a test can pin that a warm
            // sweep re-reads nothing.
            let mut hashed = 0usize;

            // 1. Process gates from qdev.toml if present (and record its sync_state row)
            if config_path.exists() {
                match fs::read_to_string(&config_path) {
                    Ok(content) => {
                        hashed += 1;
                        let (mtime, size) = file_change_stamp(&config_path);
                        let content_hash = sha256_digest(content.as_bytes());
                        upsert_sync_state_row(&tx, "qdev.toml", mtime, size, &content_hash)?;
                        refresh_gates(&tx, &content)?;
                        parsed += 1;
                    }
                    Err(e) => record_read_error(&tx, "qdev.toml", &e)?,
                }
            }

            // 2. Process all Markdown entity files.
            //
            // An unreadable file records a `read_error` finding, exactly as the sweep does:
            // swallowing it here made `qdev validate` exit 0 on the command that triggered the
            // rebuild (rebuild truncates `findings` first and `ensure_cache` takes one branch or
            // the other), while the very next command reported the same file.
            for file_path in &entity_files {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(e) => {
                        record_read_error(&tx, &relative_path(workspace_root, file_path), &e)?;
                        continue;
                    }
                };
                hashed += 1;
                let outcome = hydrate_markdown_file(&tx, workspace_root, file_path, &content)?;
                if matches!(outcome, HydrateOutcome::Parsed { .. }) {
                    parsed += 1;
                }
            }

            // 2b. Whole-graph relation validation (kind pairs, dangling targets, depends_on
            // cycles) now that every entity file has been parsed into `entities`/`relations`.
            validate_relations_graph(&tx)?;

            // 3. Process scratchpad (.jsonl) files
            for file_path in &scratch_files {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(e) => {
                        record_read_error(&tx, &relative_path(workspace_root, file_path), &e)?;
                        continue;
                    }
                };
                hashed += 1;
                hydrate_scratch_file(&tx, workspace_root, file_path, &content)?;
                parsed += 1;
            }

            // 4. Process evidence JSON files
            for file_path in &evidence_files {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(e) => {
                        record_read_error(&tx, &relative_path(workspace_root, file_path), &e)?;
                        continue;
                    }
                };
                hashed += 1;
                hydrate_evidence_file(&tx, workspace_root, file_path, &content)?;
                parsed += 1;
            }

            // Stamp freshness for this rebuild pass, inside the same transaction as the rest of
            // the rebuilt rows so `sync_meta` never observably lags the data it describes.
            tx.execute(
                "INSERT INTO sync_meta (id, last_synced_at) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET last_synced_at = excluded.last_synced_at;",
                rusqlite::params![crate::write::current_iso8601()],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to stamp sync_meta during rebuild: {}", e),
                )
            })?;

            // Total finding rows remaining after this rebuild pass.
            let findings: usize = tx
                .query_row("SELECT COUNT(*) FROM findings;", [], |row| row.get(0))
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to count findings for rebuild summary: {}", e),
                    )
                })?;

            tx.commit().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to commit rebuild transaction: {}", e),
                )
            })?;

            Ok(SweepSummary {
                parsed,
                unchanged: 0,
                // A rebuild reads and hashes every file it finds, by definition.
                hashed,
                purged: 0,
                findings,
            })
        })
    }
}

impl Store for SqliteStore {
    fn upsert_entity(&self, record: &EntityRecord) -> Result<(), QdevError> {
        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!(
                            "Failed to begin upsert transaction for '{}': {}",
                            record.id, e
                        ),
                    )
                })?;

            let (c_type, c_id) = match record.created_by {
                Some(ref a) => (Some(a.author_type.clone()), Some(a.id.clone())),
                None => (None, None),
            };
            let (u_type, u_id) = match record.updated_by {
                Some(ref a) => (Some(a.author_type.clone()), Some(a.id.clone())),
                None => (None, None),
            };

            tx.execute(
                r#"
INSERT INTO entities (
    id, kind, title, status, owners, source_path, content_hash, version,
    created_by_type, created_by_id, updated_by_type, updated_by_id, updated_at, stale
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
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
    stale = excluded.stale;
"#,
                rusqlite::params![
                    record.id,
                    record.kind.as_str(),
                    record.title,
                    record.status,
                    record.owners,
                    record.source_path,
                    record.content_hash,
                    record.version,
                    c_type,
                    c_id,
                    u_type,
                    u_id,
                    record.updated_at,
                    record.stale as i64,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert entity '{}': {}", record.id, e),
                )
            })?;

            if record.kind == EntityKind::Story
                && (record.epic_id.is_some()
                    || record.seq.is_some()
                    || record.appetite.is_some()
                    || record.safety_class.is_some()
                    || record.target_modules.is_some())
            {
                let epic_id = record.epic_id.as_deref().unwrap_or("");
                let seq = record.seq.unwrap_or(0);
                tx.execute(
                    r#"
INSERT INTO stories (id, epic_id, seq, appetite, safety_class, target_modules)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(id) DO UPDATE SET
    epic_id = CASE WHEN excluded.epic_id != '' THEN excluded.epic_id ELSE stories.epic_id END,
    seq = CASE WHEN excluded.seq != 0 THEN excluded.seq ELSE stories.seq END,
    appetite = COALESCE(excluded.appetite, stories.appetite),
    safety_class = COALESCE(excluded.safety_class, stories.safety_class),
    target_modules = COALESCE(excluded.target_modules, stories.target_modules);
"#,
                    rusqlite::params![
                        record.id,
                        epic_id,
                        seq,
                        record.appetite,
                        record.safety_class,
                        record.target_modules,
                    ],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to upsert story details for '{}': {}", record.id, e),
                    )
                })?;
            }

            tx.commit().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to commit upsert transaction for '{}': {}",
                        record.id, e
                    ),
                )
            })?;

            Ok(())
        })
    }

    fn get_entity(&self, id: &str) -> Result<Option<EntityRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
    e.id, e.kind, e.title, e.status, e.owners, e.source_path, e.content_hash, e.version,
    e.created_by_type, e.created_by_id, e.updated_by_type, e.updated_by_id, e.updated_at,
    s.epic_id, s.seq, s.appetite, s.safety_class, s.target_modules, e.stale
FROM entities e
LEFT JOIN stories s ON e.id = s.id
WHERE e.id = ?1;
"#,
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_entity query: {}", e),
                    )
                })?;

            let mut rows = stmt.query(rusqlite::params![id]).map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to query entity '{}': {}", id, e),
                )
            })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to fetch entity row for '{}': {}", id, e),
                )
            })? {
                let kind_str: String = row.get(1).map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed reading kind: {}", e),
                    )
                })?;
                let kind = EntityKind::from_str_loose(&kind_str)?;

                let c_type: Option<String> = row.get(8).unwrap_or(None);
                let c_id: Option<String> = row.get(9).unwrap_or(None);
                let created_by = match (c_type, c_id) {
                    (Some(t), Some(i)) => Some(Author::new(t, i)),
                    _ => None,
                };

                let u_type: Option<String> = row.get(10).unwrap_or(None);
                let u_id: Option<String> = row.get(11).unwrap_or(None);
                let updated_by = match (u_type, u_id) {
                    (Some(t), Some(i)) => Some(Author::new(t, i)),
                    _ => None,
                };

                Ok(Some(EntityRecord {
                    id: row.get(0).unwrap_or_default(),
                    kind,
                    title: row.get(2).unwrap_or(None),
                    status: row.get(3).unwrap_or(None),
                    owners: row.get(4).unwrap_or(None),
                    source_path: row.get(5).unwrap_or_default(),
                    content_hash: row.get(6).unwrap_or_default(),
                    version: row.get(7).unwrap_or(1),
                    created_by,
                    updated_by,
                    updated_at: row.get(12).unwrap_or_default(),
                    epic_id: row.get(13).unwrap_or(None),
                    seq: row.get(14).unwrap_or(None),
                    appetite: row.get(15).unwrap_or(None),
                    safety_class: row.get(16).unwrap_or(None),
                    target_modules: row.get(17).unwrap_or(None),
                    stale: row.get(18).map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed reading stale flag for '{}': {}", id, e),
                        )
                    })?,
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn entity_presence_for_derivation(&self, id: &str) -> Result<EntityPresence, QdevError> {
        self.with_conn(|conn| {
            // Only the flag is read — the rule itself lives in `EntityPresence::from_stale_flag`,
            // never in this SQL. A `WHERE stale = 0` here would be a second spelling of it, and
            // would also collapse `Stale` into `Absent`, which the deferred-work checks must
            // tell apart.
            let stale: Option<bool> = conn
                .query_row(
                    "SELECT stale FROM entities WHERE id = ?1;",
                    rusqlite::params![id],
                    |row| row.get::<_, i64>(0).map(|flag| flag != 0),
                )
                .optional()
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query entity presence for '{}': {}", id, e),
                    )
                })?;
            Ok(EntityPresence::from_stale_flag(stale))
        })
    }

    fn list_entities(&self, filter: &EntityFilter) -> Result<Vec<EntityRecord>, QdevError> {
        self.with_conn(|conn| {
            // `owner`/`module` are matched by exact array-element equality in Rust below, not in
            // SQL: `owners`/`target_modules` are JSON-array text columns, and a SQL `LIKE`
            // pattern can't express exact membership without either being vulnerable to
            // unescaped wildcard characters (`%`, `_`) in the filter value or SQLite's default
            // ASCII-case-insensitive `LIKE` matching — both would violate the "exact match"
            // contract these filters document.
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
    e.id, e.kind, e.title, e.status, e.owners, e.source_path, e.content_hash, e.version,
    e.created_by_type, e.created_by_id, e.updated_by_type, e.updated_by_id, e.updated_at,
    s.epic_id, s.seq, s.appetite, s.safety_class, s.target_modules, e.stale
FROM entities e
LEFT JOIN stories s ON e.id = s.id
WHERE (?1 IS NULL OR e.kind = ?1)
  AND (?2 IS NULL OR e.status = ?2)
  AND (?3 IS NULL OR s.epic_id = ?3)
  AND (?4 IS NULL OR e.id IN (SELECT story_id FROM sprint_assignments WHERE sprint_id = ?4))
ORDER BY e.id ASC;
"#,
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare list_entities query: {}", e),
                    )
                })?;

            let kind_filter = filter.kind.map(|k| k.as_str().to_string());
            let status_filter = filter.status.clone();
            let epic_filter = filter.epic_id.clone();
            let sprint_filter = filter.sprint;

            let rows = stmt
                .query_map(
                    rusqlite::params![kind_filter, status_filter, epic_filter, sprint_filter,],
                    |row| {
                        let kind_str: String = row.get(1)?;
                        let kind =
                            EntityKind::from_str_loose(&kind_str).unwrap_or(EntityKind::Story);
                        let c_type: Option<String> = row.get(8).unwrap_or(None);
                        let c_id: Option<String> = row.get(9).unwrap_or(None);
                        let created_by = match (c_type, c_id) {
                            (Some(t), Some(i)) => Some(Author::new(t, i)),
                            _ => None,
                        };
                        let u_type: Option<String> = row.get(10).unwrap_or(None);
                        let u_id: Option<String> = row.get(11).unwrap_or(None);
                        let updated_by = match (u_type, u_id) {
                            (Some(t), Some(i)) => Some(Author::new(t, i)),
                            _ => None,
                        };

                        Ok(EntityRecord {
                            id: row.get(0)?,
                            kind,
                            title: row.get(2)?,
                            status: row.get(3)?,
                            owners: row.get(4)?,
                            source_path: row.get(5)?,
                            content_hash: row.get(6)?,
                            version: row.get(7)?,
                            created_by,
                            updated_by,
                            updated_at: row.get(12)?,
                            epic_id: row.get(13)?,
                            seq: row.get(14)?,
                            appetite: row.get(15)?,
                            safety_class: row.get(16)?,
                            target_modules: row.get(17)?,
                            stale: row.get(18)?,
                        })
                    },
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to list entities: {}", e),
                    )
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed reading entity row: {}", e),
                    )
                })?);
            }

            if let Some(ref owner) = filter.owner {
                results.retain(|e| json_string_array_contains(e.owners.as_deref(), owner));
            }
            if let Some(ref module) = filter.module {
                results.retain(|e| json_string_array_contains(e.target_modules.as_deref(), module));
            }

            Ok(results)
        })
    }

    fn delete_entity(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to begin delete transaction: {}", e),
                    )
                })?;
            tx.execute("DELETE FROM stories WHERE id = ?1;", rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!(
                            "Failed to delete story child row for entity '{}': {}",
                            id, e
                        ),
                    )
                })?;
            tx.execute(
                "DELETE FROM constraints WHERE owner_id = ?1;",
                rusqlite::params![id],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to delete constraints for entity '{}': {}", id, e),
                )
            })?;
            tx.execute(
                // Only the edges this entity declared — the same rule `purge_entity_with_children`
                // follows. Edges *into* it belong to other files, and deleting them would destroy
                // content those files still assert.
                "DELETE FROM relations WHERE source_id = ?1;",
                rusqlite::params![id],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to delete relations for entity '{}': {}", id, e),
                )
            })?;
            let count = tx
                .execute("DELETE FROM entities WHERE id = ?1;", rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete entity '{}': {}", id, e),
                    )
                })?;
            tx.commit().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to commit delete transaction for entity '{}': {}",
                        id, e
                    ),
                )
            })?;
            Ok(count > 0)
        })
    }

    fn upsert_story_details(&self, story: &StoryRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
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
                    story.id,
                    story.epic_id,
                    story.seq,
                    story.appetite,
                    story.safety_class,
                    story.target_modules,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert story details for '{}': {}", story.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_story_details(&self, id: &str) -> Result<Option<StoryRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, epic_id, seq, appetite, safety_class, target_modules FROM stories WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_story_details: {}", e))
                })?;

            let mut rows = stmt
                .query(rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query stories table: {}", e))
                })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure("sqlite_error", format!("Failed fetching story row: {}", e))
            })? {
                Ok(Some(StoryRecord {
                    id: row.get(0).unwrap_or_default(),
                    epic_id: row.get(1).unwrap_or_default(),
                    seq: row.get(2).unwrap_or_default(),
                    appetite: row.get(3).unwrap_or(None),
                    safety_class: row.get(4).unwrap_or(None),
                    target_modules: row.get(5).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_story_details(&self) -> Result<Vec<StoryRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, epic_id, seq, appetite, safety_class, target_modules FROM stories ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_story_details: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(StoryRecord {
                        id: row.get(0)?,
                        epic_id: row.get(1)?,
                        seq: row.get(2)?,
                        appetite: row.get(3)?,
                        safety_class: row.get(4)?,
                        target_modules: row.get(5)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query stories: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading story row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_story_details(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute("DELETE FROM stories WHERE id = ?1;", rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete story details '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_constraint(&self, constraint: &ConstraintRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO constraints (id, owner_id, kind, text)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(id) DO UPDATE SET
    owner_id = excluded.owner_id,
    kind = excluded.kind,
    text = excluded.text;
"#,
                rusqlite::params![
                    constraint.id,
                    constraint.owner_id,
                    constraint.kind,
                    constraint.text,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert constraint '{}': {}", constraint.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_constraint(&self, id: &str) -> Result<Option<ConstraintRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, owner_id, kind, text FROM constraints WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_constraint: {}", e),
                    )
                })?;

            let mut rows = stmt.query(rusqlite::params![id]).map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to query constraint: {}", e),
                )
            })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed reading constraint row: {}", e),
                )
            })? {
                Ok(Some(ConstraintRecord {
                    id: row.get(0).unwrap_or_default(),
                    owner_id: row.get(1).unwrap_or_default(),
                    kind: row.get(2).unwrap_or_default(),
                    text: row.get(3).unwrap_or_default(),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_constraints_for_owner(
        &self,
        owner_id: &str,
    ) -> Result<Vec<ConstraintRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, owner_id, kind, text FROM constraints WHERE owner_id = ?1 ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_constraints_for_owner: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![owner_id], |row| {
                    Ok(ConstraintRecord {
                        id: row.get(0)?,
                        owner_id: row.get(1)?,
                        kind: row.get(2)?,
                        text: row.get(3)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query constraints for owner: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading constraint row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn list_constraints(&self) -> Result<Vec<ConstraintRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, owner_id, kind, text FROM constraints ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare list_constraints: {}", e),
                    )
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(ConstraintRecord {
                        id: row.get(0)?,
                        owner_id: row.get(1)?,
                        kind: row.get(2)?,
                        text: row.get(3)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query constraints: {}", e),
                    )
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed reading constraint row: {}", e),
                    )
                })?);
            }
            Ok(results)
        })
    }

    fn delete_constraint(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM constraints WHERE id = ?1;",
                    rusqlite::params![id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete constraint '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_relation(&self, relation: &RelationRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO relations (source_id, relation, target_id)
VALUES (?1, ?2, ?3)
ON CONFLICT(source_id, relation, target_id) DO NOTHING;
"#,
                rusqlite::params![relation.source_id, relation.relation, relation.target_id],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to upsert relation '{}' --[{}]--> '{}': {}",
                        relation.source_id, relation.relation, relation.target_id, e
                    ),
                )
            })?;
            Ok(())
        })
    }

    fn get_relations_for_source(&self, source_id: &str) -> Result<Vec<RelationRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT source_id, relation, target_id FROM relations WHERE source_id = ?1 ORDER BY relation, target_id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_relations_for_source: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![source_id], |row| {
                    Ok(RelationRecord {
                        source_id: row.get(0)?,
                        relation: row.get(1)?,
                        target_id: row.get(2)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query relations for source: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading relation row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn get_relations_for_target(&self, target_id: &str) -> Result<Vec<RelationRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT source_id, relation, target_id FROM relations WHERE target_id = ?1 ORDER BY source_id, relation ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_relations_for_target: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![target_id], |row| {
                    Ok(RelationRecord {
                        source_id: row.get(0)?,
                        relation: row.get(1)?,
                        target_id: row.get(2)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query relations for target: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading relation row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn list_relations(&self) -> Result<Vec<RelationRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT source_id, relation, target_id FROM relations ORDER BY source_id, relation, target_id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_relations: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(RelationRecord {
                        source_id: row.get(0)?,
                        relation: row.get(1)?,
                        target_id: row.get(2)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query relations: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading relation row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_relation(
        &self,
        source_id: &str,
        relation: &str,
        target_id: &str,
    ) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM relations WHERE source_id = ?1 AND relation = ?2 AND target_id = ?3;",
                    rusqlite::params![source_id, relation, target_id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to delete relation: {}", e))
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_sprint(&self, sprint: &SprintRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO sprints (id, title, release_version, status, owners, started_at, completed_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(id) DO UPDATE SET
    title = excluded.title,
    release_version = excluded.release_version,
    status = excluded.status,
    owners = excluded.owners,
    started_at = excluded.started_at,
    completed_at = excluded.completed_at;
"#,
                rusqlite::params![
                    sprint.id,
                    sprint.title,
                    sprint.release_version,
                    sprint.status,
                    sprint.owners,
                    sprint.started_at,
                    sprint.completed_at,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert sprint '{}': {}", sprint.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_sprint(&self, id: i64) -> Result<Option<SprintRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, title, release_version, status, owners, started_at, completed_at FROM sprints WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_sprint: {}", e))
                })?;

            let mut rows = stmt
                .query(rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query sprint: {}", e))
                })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure("sqlite_error", format!("Failed reading sprint row: {}", e))
            })? {
                Ok(Some(SprintRecord {
                    id: row.get(0).unwrap_or_default(),
                    title: row.get(1).unwrap_or(None),
                    release_version: row.get(2).unwrap_or(None),
                    status: row.get(3).unwrap_or(None),
                    owners: row.get(4).unwrap_or(None),
                    started_at: row.get(5).unwrap_or(None),
                    completed_at: row.get(6).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_sprints(&self) -> Result<Vec<SprintRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, title, release_version, status, owners, started_at, completed_at FROM sprints ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_sprints: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(SprintRecord {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        release_version: row.get(2)?,
                        status: row.get(3)?,
                        owners: row.get(4)?,
                        started_at: row.get(5)?,
                        completed_at: row.get(6)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query sprints: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading sprint row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_sprint(&self, id: i64) -> Result<bool, QdevError> {
        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to begin delete sprint transaction: {}", e),
                    )
                })?;
            tx.execute(
                "DELETE FROM sprint_assignments WHERE sprint_id = ?1;",
                rusqlite::params![id],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to delete sprint assignments for sprint {}: {}",
                        id, e
                    ),
                )
            })?;
            let count = tx
                .execute("DELETE FROM sprints WHERE id = ?1;", rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete sprint {}: {}", id, e),
                    )
                })?;
            tx.commit().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to commit delete sprint transaction for {}: {}",
                        id, e
                    ),
                )
            })?;
            Ok(count > 0)
        })
    }

    fn upsert_sprint_assignment(
        &self,
        assignment: &SprintAssignmentRecord,
    ) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO sprint_assignments (sprint_id, story_id, assigned_at, carried_from)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(sprint_id, story_id) DO UPDATE SET
    assigned_at = excluded.assigned_at,
    carried_from = excluded.carried_from;
"#,
                rusqlite::params![
                    assignment.sprint_id,
                    assignment.story_id,
                    assignment.assigned_at,
                    assignment.carried_from,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to upsert sprint assignment for sprint {} story '{}': {}",
                        assignment.sprint_id, assignment.story_id, e
                    ),
                )
            })?;
            Ok(())
        })
    }

    fn get_sprint_assignments(
        &self,
        sprint_id: i64,
    ) -> Result<Vec<SprintAssignmentRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT sprint_id, story_id, assigned_at, carried_from FROM sprint_assignments WHERE sprint_id = ?1 ORDER BY story_id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_sprint_assignments: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![sprint_id], |row| {
                    Ok(SprintAssignmentRecord {
                        sprint_id: row.get(0)?,
                        story_id: row.get(1)?,
                        assigned_at: row.get(2)?,
                        carried_from: row.get(3)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query sprint assignments: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading sprint assignment row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn get_assignments_for_story(
        &self,
        story_id: &str,
    ) -> Result<Vec<SprintAssignmentRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT sprint_id, story_id, assigned_at, carried_from FROM sprint_assignments WHERE story_id = ?1 ORDER BY sprint_id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_assignments_for_story: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![story_id], |row| {
                    Ok(SprintAssignmentRecord {
                        sprint_id: row.get(0)?,
                        story_id: row.get(1)?,
                        assigned_at: row.get(2)?,
                        carried_from: row.get(3)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query assignments for story: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading sprint assignment row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn list_sprint_assignments(&self) -> Result<Vec<SprintAssignmentRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT sprint_id, story_id, assigned_at, carried_from FROM sprint_assignments ORDER BY sprint_id, story_id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_sprint_assignments: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(SprintAssignmentRecord {
                        sprint_id: row.get(0)?,
                        story_id: row.get(1)?,
                        assigned_at: row.get(2)?,
                        carried_from: row.get(3)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query sprint assignments: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading sprint assignment row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_sprint_assignment(&self, sprint_id: i64, story_id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM sprint_assignments WHERE sprint_id = ?1 AND story_id = ?2;",
                    rusqlite::params![sprint_id, story_id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete sprint assignment: {}", e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_decision(&self, decision: &DecisionRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO decisions (id, subject_id, decision_type, topic, context, ruling, author_type, author_id, created_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(id) DO UPDATE SET
    subject_id = excluded.subject_id,
    decision_type = excluded.decision_type,
    topic = excluded.topic,
    context = excluded.context,
    ruling = excluded.ruling,
    author_type = excluded.author_type,
    author_id = excluded.author_id,
    created_at = excluded.created_at;
"#,
                rusqlite::params![
                    decision.id,
                    decision.subject_id,
                    decision.decision_type,
                    decision.topic,
                    decision.context,
                    decision.ruling,
                    decision.author_type,
                    decision.author_id,
                    decision.created_at,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert decision '{}': {}", decision.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_decision(&self, id: &str) -> Result<Option<DecisionRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, subject_id, decision_type, topic, context, ruling, author_type, author_id, created_at FROM decisions WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_decision: {}", e))
                })?;

            let mut rows = stmt
                .query(rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query decision: {}", e))
                })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure("sqlite_error", format!("Failed reading decision row: {}", e))
            })? {
                Ok(Some(DecisionRecord {
                    id: row.get(0).unwrap_or_default(),
                    subject_id: row.get(1).unwrap_or_default(),
                    decision_type: row.get(2).unwrap_or(None),
                    topic: row.get(3).unwrap_or(None),
                    context: row.get(4).unwrap_or(None),
                    ruling: row.get(5).unwrap_or(None),
                    author_type: row.get(6).unwrap_or(None),
                    author_id: row.get(7).unwrap_or(None),
                    created_at: row.get(8).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_decisions_for_subject(
        &self,
        subject_id: &str,
    ) -> Result<Vec<DecisionRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, subject_id, decision_type, topic, context, ruling, author_type, author_id, created_at FROM decisions WHERE subject_id = ?1 ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_decisions_for_subject: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![subject_id], |row| {
                    Ok(DecisionRecord {
                        id: row.get(0)?,
                        subject_id: row.get(1)?,
                        decision_type: row.get(2)?,
                        topic: row.get(3)?,
                        context: row.get(4)?,
                        ruling: row.get(5)?,
                        author_type: row.get(6)?,
                        author_id: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query decisions for subject: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading decision row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn list_decisions(&self) -> Result<Vec<DecisionRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, subject_id, decision_type, topic, context, ruling, author_type, author_id, created_at FROM decisions ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_decisions: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(DecisionRecord {
                        id: row.get(0)?,
                        subject_id: row.get(1)?,
                        decision_type: row.get(2)?,
                        topic: row.get(3)?,
                        context: row.get(4)?,
                        ruling: row.get(5)?,
                        author_type: row.get(6)?,
                        author_id: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query decisions: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading decision row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_decision(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM decisions WHERE id = ?1;",
                    rusqlite::params![id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete decision '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_deferred_work(&self, dw: &DeferredWorkRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO deferred_work (id, origin_story_id, target_module, status, safety_risk, rationale, gate, resolution)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT(id) DO UPDATE SET
    origin_story_id = excluded.origin_story_id,
    target_module = excluded.target_module,
    status = excluded.status,
    safety_risk = excluded.safety_risk,
    rationale = excluded.rationale,
    gate = excluded.gate,
    resolution = excluded.resolution;
"#,
                rusqlite::params![
                    dw.id,
                    dw.origin_story_id,
                    dw.target_module,
                    dw.status,
                    dw.safety_risk,
                    dw.rationale,
                    dw.gate,
                    dw.resolution,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert deferred work '{}': {}", dw.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_deferred_work(&self, id: &str) -> Result<Option<DeferredWorkRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, origin_story_id, target_module, status, safety_risk, rationale, gate, resolution FROM deferred_work WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_deferred_work: {}", e))
                })?;

            let mut rows = stmt
                .query(rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query deferred work: {}", e))
                })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure("sqlite_error", format!("Failed reading deferred work row: {}", e))
            })? {
                Ok(Some(DeferredWorkRecord {
                    id: row.get(0).unwrap_or_default(),
                    origin_story_id: row.get(1).unwrap_or(None),
                    target_module: row.get(2).unwrap_or_default(),
                    status: row.get(3).unwrap_or(None),
                    safety_risk: row.get(4).unwrap_or(None),
                    rationale: row.get(5).unwrap_or(None),
                    gate: row.get(6).unwrap_or(None),
                    resolution: row.get(7).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_deferred_work(&self) -> Result<Vec<DeferredWorkRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, origin_story_id, target_module, status, safety_risk, rationale, gate, resolution FROM deferred_work ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_deferred_work: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(DeferredWorkRecord {
                        id: row.get(0)?,
                        origin_story_id: row.get(1)?,
                        target_module: row.get(2)?,
                        status: row.get(3)?,
                        safety_risk: row.get(4)?,
                        rationale: row.get(5)?,
                        gate: row.get(6)?,
                        resolution: row.get(7)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query deferred work: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading deferred work row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_deferred_work(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM deferred_work WHERE id = ?1;",
                    rusqlite::params![id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete deferred work '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_scratchpad_entry(&self, entry: &ScratchpadRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO scratchpad_entries (story_id, seq, at, author_type, author_id, kind, text)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(story_id, seq) DO UPDATE SET
    at = excluded.at,
    author_type = excluded.author_type,
    author_id = excluded.author_id,
    kind = excluded.kind,
    text = excluded.text;
"#,
                rusqlite::params![
                    entry.story_id,
                    entry.seq,
                    entry.at,
                    entry.author_type,
                    entry.author_id,
                    entry.kind,
                    entry.text,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to upsert scratchpad entry for '{}:{}': {}",
                        entry.story_id, entry.seq, e
                    ),
                )
            })?;
            Ok(())
        })
    }

    fn get_scratchpad_entries(&self, story_id: &str) -> Result<Vec<ScratchpadRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT story_id, seq, at, author_type, author_id, kind, text FROM scratchpad_entries WHERE story_id = ?1 ORDER BY seq ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_scratchpad_entries: {}", e))
                })?;

            let rows = stmt
                .query_map(rusqlite::params![story_id], |row| {
                    Ok(ScratchpadRecord {
                        story_id: row.get(0)?,
                        seq: row.get(1)?,
                        at: row.get(2)?,
                        author_type: row.get(3)?,
                        author_id: row.get(4)?,
                        kind: row.get(5)?,
                        text: row.get(6)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query scratchpad entries: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading scratchpad row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn list_scratchpad_entries(&self) -> Result<Vec<ScratchpadRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT story_id, seq, at, author_type, author_id, kind, text FROM scratchpad_entries ORDER BY story_id, seq ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_scratchpad_entries: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(ScratchpadRecord {
                        story_id: row.get(0)?,
                        seq: row.get(1)?,
                        at: row.get(2)?,
                        author_type: row.get(3)?,
                        author_id: row.get(4)?,
                        kind: row.get(5)?,
                        text: row.get(6)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query scratchpad entries: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading scratchpad row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_scratchpad_entry(&self, story_id: &str, seq: u32) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM scratchpad_entries WHERE story_id = ?1 AND seq = ?2;",
                    rusqlite::params![story_id, seq],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete scratchpad entry: {}", e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_gate(&self, gate: &GateRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO gates (id, command, kind, timeout_ms, output_adapter, on_transition, depends_on, metric, direction)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(id) DO UPDATE SET
    command = excluded.command,
    kind = excluded.kind,
    timeout_ms = excluded.timeout_ms,
    output_adapter = excluded.output_adapter,
    on_transition = excluded.on_transition,
    depends_on = excluded.depends_on,
    metric = excluded.metric,
    direction = excluded.direction;
"#,
                rusqlite::params![
                    gate.id,
                    gate.command,
                    gate.kind,
                    gate.timeout_ms,
                    gate.output_adapter,
                    gate.on_transition,
                    gate.depends_on,
                    gate.metric,
                    gate.direction,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert gate '{}': {}", gate.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_gate(&self, id: &str) -> Result<Option<GateRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, command, kind, timeout_ms, output_adapter, on_transition, depends_on, metric, direction FROM gates WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_gate: {}", e))
                })?;

            let mut rows = stmt
                .query(rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query gate: {}", e))
                })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure("sqlite_error", format!("Failed reading gate row: {}", e))
            })? {
                Ok(Some(GateRecord {
                    id: row.get(0).unwrap_or_default(),
                    command: row.get(1).unwrap_or_default(),
                    kind: row.get(2).unwrap_or(None),
                    timeout_ms: row.get(3).unwrap_or(None),
                    output_adapter: row.get(4).unwrap_or(None),
                    on_transition: row.get(5).unwrap_or(None),
                    depends_on: row.get(6).unwrap_or(None),
                    metric: row.get(7).unwrap_or(None),
                    direction: row.get(8).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_gates(&self) -> Result<Vec<GateRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, command, kind, timeout_ms, output_adapter, on_transition, depends_on, metric, direction FROM gates ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_gates: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(GateRecord {
                        id: row.get(0)?,
                        command: row.get(1)?,
                        kind: row.get(2)?,
                        timeout_ms: row.get(3)?,
                        output_adapter: row.get(4)?,
                        on_transition: row.get(5)?,
                        depends_on: row.get(6)?,
                        metric: row.get(7)?,
                        direction: row.get(8)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query gates: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading gate row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_gate(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute("DELETE FROM gates WHERE id = ?1;", rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete gate '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_gate_run(&self, run: &GateRunRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO gate_runs (
    id, story_id, gate_id, commit_sha, status, exit_code, duration_ms,
    metric_value, summary, evidence_path, output_hash, run_by_type, run_by_id, ran_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
ON CONFLICT(id) DO UPDATE SET
    story_id = excluded.story_id,
    gate_id = excluded.gate_id,
    commit_sha = excluded.commit_sha,
    status = excluded.status,
    exit_code = excluded.exit_code,
    duration_ms = excluded.duration_ms,
    metric_value = excluded.metric_value,
    summary = excluded.summary,
    evidence_path = excluded.evidence_path,
    output_hash = excluded.output_hash,
    run_by_type = excluded.run_by_type,
    run_by_id = excluded.run_by_id,
    ran_at = excluded.ran_at;
"#,
                rusqlite::params![
                    run.id,
                    run.story_id,
                    run.gate_id,
                    run.commit_sha,
                    run.status,
                    run.exit_code,
                    run.duration_ms,
                    run.metric_value,
                    run.summary,
                    run.evidence_path,
                    run.output_hash,
                    run.run_by_type,
                    run.run_by_id,
                    run.ran_at,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert gate run '{}': {}", run.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_gate_run(&self, id: &str) -> Result<Option<GateRunRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT id, story_id, gate_id, commit_sha, status, exit_code, duration_ms,
       metric_value, summary, evidence_path, output_hash, run_by_type, run_by_id, ran_at
FROM gate_runs WHERE id = ?1;
"#,
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_gate_run: {}", e),
                    )
                })?;

            let mut rows = stmt.query(rusqlite::params![id]).map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to query gate run: {}", e),
                )
            })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed reading gate run row: {}", e),
                )
            })? {
                Ok(Some(GateRunRecord {
                    id: row.get(0).unwrap_or_default(),
                    story_id: row.get(1).unwrap_or(None),
                    gate_id: row.get(2).unwrap_or_default(),
                    commit_sha: row.get(3).unwrap_or_default(),
                    status: row.get(4).unwrap_or(None),
                    exit_code: row.get(5).unwrap_or(None),
                    duration_ms: row.get(6).unwrap_or(None),
                    metric_value: row.get(7).unwrap_or(None),
                    summary: row.get(8).unwrap_or(None),
                    evidence_path: row.get(9).unwrap_or_default(),
                    output_hash: row.get(10).unwrap_or(None),
                    run_by_type: row.get(11).unwrap_or(None),
                    run_by_id: row.get(12).unwrap_or(None),
                    ran_at: row.get(13).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_gate_runs(&self) -> Result<Vec<GateRunRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT id, story_id, gate_id, commit_sha, status, exit_code, duration_ms,
       metric_value, summary, evidence_path, output_hash, run_by_type, run_by_id, ran_at
FROM gate_runs ORDER BY id ASC;
"#,
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare list_gate_runs: {}", e),
                    )
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(GateRunRecord {
                        id: row.get(0)?,
                        story_id: row.get(1)?,
                        gate_id: row.get(2)?,
                        commit_sha: row.get(3)?,
                        status: row.get(4)?,
                        exit_code: row.get(5)?,
                        duration_ms: row.get(6)?,
                        metric_value: row.get(7)?,
                        summary: row.get(8)?,
                        evidence_path: row.get(9)?,
                        output_hash: row.get(10)?,
                        run_by_type: row.get(11)?,
                        run_by_id: row.get(12)?,
                        ran_at: row.get(13)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query gate runs: {}", e),
                    )
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed reading gate run row: {}", e),
                    )
                })?);
            }
            Ok(results)
        })
    }

    fn delete_gate_run(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM gate_runs WHERE id = ?1;",
                    rusqlite::params![id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete gate run '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_soup(&self, soup: &SoupRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO soup_dependencies (id, name, version, license, cve_status, introduced_by_story, evaluated_for_release)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(id) DO UPDATE SET
    name = excluded.name,
    version = excluded.version,
    license = excluded.license,
    cve_status = excluded.cve_status,
    introduced_by_story = excluded.introduced_by_story,
    evaluated_for_release = excluded.evaluated_for_release;
"#,
                rusqlite::params![
                    soup.id,
                    soup.name,
                    soup.version,
                    soup.license,
                    soup.cve_status,
                    soup.introduced_by_story,
                    soup.evaluated_for_release,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert SOUP dependency '{}': {}", soup.id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_soup(&self, id: &str) -> Result<Option<SoupRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, name, version, license, cve_status, introduced_by_story, evaluated_for_release FROM soup_dependencies WHERE id = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare get_soup: {}", e))
                })?;

            let mut rows = stmt
                .query(rusqlite::params![id])
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query soup_dependencies: {}", e))
                })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure("sqlite_error", format!("Failed reading soup row: {}", e))
            })? {
                Ok(Some(SoupRecord {
                    id: row.get(0).unwrap_or_default(),
                    name: row.get(1).unwrap_or(None),
                    version: row.get(2).unwrap_or(None),
                    license: row.get(3).unwrap_or(None),
                    cve_status: row.get(4).unwrap_or(None),
                    introduced_by_story: row.get(5).unwrap_or(None),
                    evaluated_for_release: row.get(6).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_soup(&self) -> Result<Vec<SoupRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, name, version, license, cve_status, introduced_by_story, evaluated_for_release FROM soup_dependencies ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to prepare list_soup: {}", e))
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(SoupRecord {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        version: row.get(2)?,
                        license: row.get(3)?,
                        cve_status: row.get(4)?,
                        introduced_by_story: row.get(5)?,
                        evaluated_for_release: row.get(6)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed to query soup_dependencies: {}", e))
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure("sqlite_error", format!("Failed reading soup row: {}", e))
                })?);
            }
            Ok(results)
        })
    }

    fn delete_soup(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM soup_dependencies WHERE id = ?1;",
                    rusqlite::params![id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete SOUP dependency '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn upsert_sync_state(
        &self,
        path: &str,
        mtime: i64,
        size: u64,
        hash: Option<&str>,
    ) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO sync_state (path, mtime, size, content_hash)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(path) DO UPDATE SET
    mtime = excluded.mtime,
    size = excluded.size,
    content_hash = excluded.content_hash;
"#,
                rusqlite::params![path, mtime, size, hash],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert sync_state for '{}': {}", path, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_sync_state(&self, path: &str) -> Result<Option<SyncStateRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT path, mtime, size, content_hash FROM sync_state WHERE path = ?1;")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_sync_state: {}", e),
                    )
                })?;

            let mut rows = stmt.query(rusqlite::params![path]).map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to query sync_state: {}", e),
                )
            })?;

            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed reading sync_state row: {}", e),
                )
            })? {
                Ok(Some(SyncStateRecord {
                    path: row.get(0).unwrap_or_default(),
                    mtime: row.get(1).unwrap_or_default(),
                    size: row.get(2).unwrap_or_default(),
                    content_hash: row.get(3).unwrap_or(None),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_sync_state(&self) -> Result<Vec<SyncStateRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, mtime, size, content_hash FROM sync_state ORDER BY path ASC;",
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare list_sync_state: {}", e),
                    )
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(SyncStateRecord {
                        path: row.get(0)?,
                        mtime: row.get(1)?,
                        size: row.get(2)?,
                        content_hash: row.get(3)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query sync_state: {}", e),
                    )
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed reading sync_state row: {}", e),
                    )
                })?);
            }
            Ok(results)
        })
    }

    fn delete_sync_state(&self, path: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM sync_state WHERE path = ?1;",
                    rusqlite::params![path],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete sync_state for '{}': {}", path, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn mark_entity_dirty(&self, id: &str, dirty_at: &str) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO dirty_entities (id, dirty_at)
VALUES (?1, ?2)
ON CONFLICT(id) DO UPDATE SET dirty_at = excluded.dirty_at;
"#,
                rusqlite::params![id, dirty_at],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to mark entity '{}' dirty: {}", id, e),
                )
            })?;
            Ok(())
        })
    }

    fn get_dirty_entities(&self) -> Result<Vec<DirtyEntityRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, dirty_at FROM dirty_entities ORDER BY id ASC;")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_dirty_entities: {}", e),
                    )
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(DirtyEntityRecord {
                        id: row.get(0)?,
                        dirty_at: row.get(1)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query dirty_entities: {}", e),
                    )
                })?;

            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed reading dirty_entities row: {}", e),
                    )
                })?);
            }
            Ok(results)
        })
    }

    fn clear_dirty_entity(&self, id: &str) -> Result<bool, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM dirty_entities WHERE id = ?1;",
                    rusqlite::params![id],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to clear dirty entity '{}': {}", id, e),
                    )
                })?;
            Ok(count > 0)
        })
    }

    fn clear_all_dirty_entities(&self) -> Result<usize, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute("DELETE FROM dirty_entities;", [])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to clear all dirty entities: {}", e),
                    )
                })?;
            Ok(count)
        })
    }

    fn upsert_finding(&self, finding: &FindingRecord) -> Result<(), QdevError> {
        self.with_conn(|conn| {
            conn.execute(
                r#"
INSERT INTO findings (path, code, severity, message, message_key, found_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(path, code, message_key) DO UPDATE SET
    severity = excluded.severity,
    message = excluded.message,
    found_at = excluded.found_at;
"#,
                rusqlite::params![
                    finding.path,
                    finding.code,
                    finding.severity,
                    finding.message,
                    // `message_key` cannot distinguish a NULL message from an empty one, so
                    // `message` stays in the update list: without it the first of the two
                    // written would win permanently and could never be corrected.
                    finding.message.as_deref().unwrap_or(""),
                    finding.found_at,
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to upsert finding '{}/{}': {}",
                        finding.path, finding.code, e
                    ),
                )
            })?;
            Ok(())
        })
    }

    fn get_finding(&self, path: &str, code: &str) -> Result<Option<FindingRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, code, severity, message, found_at FROM findings WHERE path = ?1 AND code = ?2 ORDER BY message_key ASC LIMIT 1;",
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_finding query: {}", e),
                    )
                })?;
            let mut rows = stmt
                .query(rusqlite::params![path, code])
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query finding '{}/{}': {}", path, code, e),
                    )
                })?;
            if let Some(row) = rows.next().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to read finding row: {}", e),
                )
            })? {
                Ok(Some(FindingRecord {
                    path: row.get(0).unwrap_or_default(),
                    code: row.get(1).unwrap_or_default(),
                    severity: row.get(2).unwrap_or_default(),
                    message: row.get(3).unwrap_or(None),
                    found_at: row.get(4).unwrap_or_default(),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn get_findings_for_path(&self, path: &str) -> Result<Vec<FindingRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, code, severity, message, found_at FROM findings WHERE path = ?1 ORDER BY code ASC, message_key ASC;",
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare get_findings_for_path query: {}", e),
                    )
                })?;
            let rows = stmt
                .query_map(rusqlite::params![path], |row| {
                    Ok(FindingRecord {
                        path: row.get(0)?,
                        code: row.get(1)?,
                        severity: row.get(2)?,
                        message: row.get(3)?,
                        found_at: row.get(4)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query findings for path '{}': {}", path, e),
                    )
                })?;
            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to read finding row for path '{}': {}", path, e),
                    )
                })?);
            }
            Ok(results)
        })
    }

    fn list_findings(&self) -> Result<Vec<FindingRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, code, severity, message, found_at FROM findings ORDER BY path ASC, code ASC, message_key ASC;",
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare list_findings query: {}", e),
                    )
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(FindingRecord {
                        path: row.get(0)?,
                        code: row.get(1)?,
                        severity: row.get(2)?,
                        message: row.get(3)?,
                        found_at: row.get(4)?,
                    })
                })
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to query findings: {}", e),
                    )
                })?;
            let mut results = Vec::new();
            for r in rows {
                results.push(r.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to read finding row: {}", e),
                    )
                })?);
            }
            Ok(results)
        })
    }

    fn delete_findings_for_path(&self, path: &str) -> Result<usize, QdevError> {
        self.with_conn(|conn| {
            let count = conn
                .execute(
                    "DELETE FROM findings WHERE path = ?1;",
                    rusqlite::params![path],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to delete findings for path '{}': {}", path, e),
                    )
                })?;
            Ok(count)
        })
    }

    fn get_last_synced_at(&self) -> Result<Option<String>, QdevError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT last_synced_at FROM sync_meta WHERE id = 1;",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to read last_synced_at from sync_meta: {}", e),
                )
            })
        })
    }

    fn cache_schema_version(&self) -> Result<u32, QdevError> {
        self.with_conn(|conn| {
            conn.query_row("PRAGMA user_version;", [], |row| row.get(0))
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to read user_version pragma: {}", e),
                    )
                })
        })
    }

    fn cache_missing_tables(&self) -> Result<Vec<String>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type = 'table';")
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare sqlite_master query: {}", e),
                    )
                })?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to list cache tables: {}", e),
                    )
                })?;
            let mut present = HashSet::new();
            for row in rows {
                present.insert(row.map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to read cache table name: {}", e),
                    )
                })?);
            }
            let mut missing: Vec<String> = ALL_TABLE_NAMES
                .iter()
                .filter(|t| !present.contains(**t))
                .map(|t| (*t).to_string())
                .collect();
            missing.sort();
            Ok(missing)
        })
    }

    fn sweep_workspace(
        &self,
        workspace_root: &Path,
        storage: &StorageConfig,
    ) -> Result<SweepSummary, QdevError> {
        let cache_dir = workspace_root.join(&storage.cache_dir);
        let lock_path = cache_dir.join("write.lock");
        let _lock_guard = crate::write::acquire_write_lock(
            &lock_path,
            std::time::Duration::from_millis(BUSY_TIMEOUT_MS),
        )?;

        let specs_dir = workspace_root.join(&storage.specs_dir);
        let state_dir = workspace_root.join(&storage.state_dir);
        let scratch_dir = state_dir.join("scratch");
        let evidence_dir = state_dir.join("evidence");
        let config_path = workspace_root.join("qdev.toml");

        // Collect the current on-disk file sets (the same 4+1 sets a full rebuild scans)
        let mut entity_files = Vec::new();
        collect_markdown_files(&specs_dir, &mut entity_files);
        collect_markdown_files(&state_dir, &mut entity_files);
        entity_files.sort();
        // Deduped for the same reason as the rebuild's walk: an overlapping
        // `specs_dir`/`state_dir` layout would otherwise sweep a file twice.
        entity_files.dedup();

        let mut scratch_files = Vec::new();
        collect_files_with_ext(&scratch_dir, "jsonl", &mut scratch_files);
        scratch_files.sort();

        let mut evidence_files = Vec::new();
        collect_files_with_ext(&evidence_dir, "json", &mut evidence_files);
        evidence_files.sort();

        let mut disk: Vec<(String, PathBuf, SweepFileRole)> = Vec::new();
        for f in &entity_files {
            disk.push((
                relative_path(workspace_root, f),
                f.clone(),
                SweepFileRole::Markdown,
            ));
        }
        for f in &scratch_files {
            disk.push((
                relative_path(workspace_root, f),
                f.clone(),
                SweepFileRole::Scratch,
            ));
        }
        for f in &evidence_files {
            disk.push((
                relative_path(workspace_root, f),
                f.clone(),
                SweepFileRole::Evidence,
            ));
        }
        if config_path.exists() {
            disk.push((
                "qdev.toml".to_string(),
                config_path.clone(),
                SweepFileRole::Config,
            ));
        }
        let disk_paths: HashSet<String> = disk.iter().map(|d| d.0.clone()).collect();

        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to begin sweep transaction: {}", e),
                    )
                })?;

            // Load current sync_state: path -> (mtime, size, content_hash)
            let mut sync_map: HashMap<String, (i64, i64, Option<String>)> = HashMap::new();
            {
                let mut stmt = tx
                    .prepare("SELECT path, mtime, size, content_hash FROM sync_state;")
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to prepare sync_state sweep query: {}", e),
                        )
                    })?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, Option<String>>(3)?,
                        ))
                    })
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to query sync_state for sweep: {}", e),
                        )
                    })?;
                for r in rows {
                    let (p, m, s, h) = r.map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to read sync_state row for sweep: {}", e),
                        )
                    })?;
                    sync_map.insert(p, (m, s, h));
                }
            }

            // Load dirty entity ids and map them to their source paths
            let mut dirty_ids: HashSet<String> = HashSet::new();
            {
                let mut stmt = tx.prepare("SELECT id FROM dirty_entities;").map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to prepare dirty sweep query: {}", e),
                    )
                })?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to query dirty_entities for sweep: {}", e),
                        )
                    })?;
                for r in rows {
                    dirty_ids.insert(r.map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to read dirty row for sweep: {}", e),
                        )
                    })?);
                }
            }
            // Load the whole `id -> source_path` mapping once. Three things are derived from
            // it, each of which used to cost its own scan or a per-path query: which paths hold
            // a dirty entity, which paths the cache knows about (the purge-driving set), and
            // which paths own an `entities` row at all (the accounted-for check below).
            let mut dirty_paths: HashSet<String> = HashSet::new();
            let mut path_of_id: HashMap<String, String> = HashMap::new();
            let mut ids_by_path: HashMap<String, HashSet<String>> = HashMap::new();
            {
                let mut stmt = tx
                    .prepare("SELECT id, source_path FROM entities;")
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to prepare entity path sweep query: {}", e),
                        )
                    })?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to query entities for sweep: {}", e),
                        )
                    })?;
                for r in rows {
                    let (id, sp) = r.map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to read entity row for sweep: {}", e),
                        )
                    })?;
                    if dirty_ids.contains(&id) {
                        dirty_paths.insert(sp.clone());
                    }
                    path_of_id.insert(id.clone(), sp.clone());
                    ids_by_path.entry(sp).or_default().insert(id);
                }
            }

            // Every path that currently carries at least one finding. Together with
            // `ids_by_path` this answers, without touching the database again, whether a file
            // the change gate would skip is accounted for: a row claiming it, or a finding
            // saying why there is none.
            let mut finding_paths: HashSet<String> = HashSet::new();
            {
                let mut stmt = tx
                    .prepare(
                        // Only the codes that explain why a file has no row count as an
                        // explanation. `dangling_relation`, `invalid_relation_kind` and
                        // `dependency_cycle` are recorded against paths that parsed
                        // *successfully*, so accepting any finding at all would let a file whose
                        // row was taken by a duplicate be treated as accounted for and never
                        // re-parsed — the invariant unenforced for exactly the file it protects.
                        "SELECT DISTINCT path FROM findings \
                         WHERE code IN ('schema_violation', 'merge_conflict', 'read_error');",
                    )
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to prepare findings path scan: {}", e),
                        )
                    })?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to scan finding paths for sweep: {}", e),
                        )
                    })?;
                for r in rows {
                    finding_paths.insert(r.map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to read finding path for sweep: {}", e),
                        )
                    })?);
                }
            }

            let mut parsed = 0usize;
            let mut unchanged = 0usize;
            let mut hashed = 0usize;
            let mut purged = 0usize;
            // Paths whose hydration succeeded this pass. Only these consume a dirty row.
            let mut parsed_paths: HashSet<String> = HashSet::new();

            // 1. Purge removed files: any path the cache knows about that is no longer on disk.
            //
            // The known set is the union of `sync_state` and every `entities.source_path`, not
            // `sync_state` alone. The write path deletes a file's `sync_state` row to invalidate
            // it, so an entity that was written and then deleted has no `sync_state` row at all —
            // driving purge from that table by itself left it in the cache permanently, still
            // queryable, with no file behind it.
            let mut known_paths: HashSet<String> = sync_map.keys().cloned().collect();
            known_paths.extend(ids_by_path.keys().cloned());

            let mut removed: Vec<String> = known_paths
                .iter()
                .filter(|p| !disk_paths.contains(*p))
                .cloned()
                .collect();
            removed.sort();
            for path in &removed {
                purge_removed_path(&tx, path)?;
                purged += 1;
                // Keep the in-memory view of the cache in step with what was just deleted, so
                // the accounted-for check below sees a file whose row a cascade took away.
                for id in ids_by_path.remove(path).unwrap_or_default() {
                    path_of_id.remove(&id);
                }
                finding_paths.remove(path);
            }

            // 2. Sweep each on-disk file
            for (rel, abs, role) in &disk {
                let (stamp, size) = file_change_stamp(abs);
                let size_i64 = size as i64;
                let is_dirty = dirty_paths.contains(rel);
                let stored = sync_map.get(rel);

                // A file is a re-hash candidate only if its mtime or size differs, its
                // sync_state row is missing, or its entity is dirty.
                let meta_changed = match stored {
                    Some((m, s, _)) => *m != stamp || *s != size_i64,
                    None => true,
                };

                // ...or if the cache has lost the row this file owns. The change gate above is
                // an optimisation resting on the assumption that only a change to a file can
                // invalidate the rows that file owns; a cascading purge, or another file taking
                // over its id, breaks that assumption, and an unchanged file would then be
                // trusted forever while `entities` has nothing for it. The invariant enforced
                // here is that every known, readable entity file either has an `entities` row
                // claiming it or a finding explaining why it does not.
                //
                // Both halves are answered from the maps loaded once above and kept in step
                // with every purge and hydration this pass makes — no extra query and no extra
                // I/O per file, so the boot budget keeps its shape. Kept in step rather than
                // snapshotted because the two must agree: when two files declare one id, each
                // takes the row from the other, so both re-parse and the file that ends up
                // owning it is the last in the sorted order the loop walks — the same one a
                // full rebuild leaves owning it.
                let unaccounted = matches!(role, SweepFileRole::Markdown)
                    && !meta_changed
                    && !is_dirty
                    && !ids_by_path.contains_key(rel)
                    && !finding_paths.contains(rel);

                // ...or if the mtime comparison cannot resolve the window the edit could be in.
                // Stored and current mtime are equal *and* carry no sub-second detail, so all
                // they establish is that the file was last written somewhere inside that one
                // second: an edit made later in the same second, to the same length, is
                // indistinguishable from no edit at all. That means a coarse-granularity
                // filesystem (HFS+, some network mounts), not a full-precision one, where an
                // edit moves the nanoseconds and `meta_changed` already sees it.
                //
                // Such a file is read and hashed rather than trusted, and the hash then decides
                // whether it re-parses — so the price is a read, not a parse, and it is paid only
                // by files whose mtime lands exactly on a second boundary rather than by the
                // whole workspace on every boot (AD-6's 30 ms budget rules that out).
                let unresolved_second = match stored {
                    Some((m, s, _)) => stamp_cannot_resolve_the_edit(*m, stamp, *s, size_i64),
                    None => false,
                };

                if !meta_changed && !is_dirty && !unaccounted && !unresolved_second {
                    // Content and permissions both known unchanged: the stamp compared above is
                    // the later of mtime and ctime, so a `chmod` that makes the file unreadable
                    // moves it and the file takes the candidate path below, where the failed read
                    // is recorded as a `read_error` — the same answer a rebuild gives. See
                    // `file_mtime_size` for why this is not done by opening every skipped file.
                    unchanged += 1;
                    continue;
                }

                // Candidate: read and hash. A read failure is recorded as a finding and the
                // previous rows are retained stale - it is never silently counted as swept.
                let content = match fs::read_to_string(abs) {
                    Ok(c) => c,
                    Err(e) => {
                        record_read_error(&tx, rel, &e)?;
                        finding_paths.insert(rel.clone());
                        unchanged += 1;
                        continue;
                    }
                };
                hashed += 1;
                let hash = sha256_digest(content.as_bytes());
                let hash_changed = match stored {
                    Some((_, _, h)) => h.as_deref() != Some(hash.as_str()),
                    None => true,
                };

                if hash_changed || is_dirty || unaccounted {
                    match role {
                        SweepFileRole::Markdown => {
                            // `parsed` counts only successful upserts; conflicted / schema-violating
                            // files are retained stale and not counted here.
                            let outcome =
                                hydrate_markdown_file(&tx, workspace_root, abs, &content)?;
                            match &outcome {
                                HydrateOutcome::Parsed { id } => {
                                    parsed += 1;
                                    parsed_paths.insert(rel.clone());
                                    // Mirror the row moves this hydration just made. Taking an
                                    // id from another file leaves *that* file without a row, and
                                    // it must then be re-parsed rather than trusted — which is
                                    // how two files declaring one id settle on the same winner a
                                    // full rebuild would pick, the last one in sorted order.
                                    let claimed = ids_by_path.entry(rel.clone()).or_default();
                                    let displaced: Vec<String> =
                                        claimed.iter().filter(|old| *old != id).cloned().collect();
                                    claimed.insert(id.clone());
                                    for old in displaced {
                                        claimed.remove(&old);
                                        path_of_id.remove(&old);
                                    }
                                    if let Some(previous) =
                                        path_of_id.insert(id.clone(), rel.clone())
                                    {
                                        if previous != *rel {
                                            if let Some(ids) = ids_by_path.get_mut(&previous) {
                                                ids.remove(id);
                                                if ids.is_empty() {
                                                    ids_by_path.remove(&previous);
                                                }
                                            }
                                        }
                                    }
                                    finding_paths.remove(rel);
                                }
                                _ => {
                                    // Conflicted, schema-invalid or id-less: the file carries a
                                    // finding that explains why it has no fresh row.
                                    finding_paths.insert(rel.clone());
                                }
                            }
                        }
                        SweepFileRole::Scratch => {
                            hydrate_scratch_file(&tx, workspace_root, abs, &content)?;
                            parsed += 1;
                            parsed_paths.insert(rel.clone());
                        }
                        SweepFileRole::Evidence => {
                            hydrate_evidence_file(&tx, workspace_root, abs, &content)?;
                            parsed += 1;
                            parsed_paths.insert(rel.clone());
                        }
                        SweepFileRole::Config => {
                            refresh_gates(&tx, &content)?;
                            upsert_sync_state_row(&tx, rel, stamp, size, &hash)?;
                            parsed += 1;
                            parsed_paths.insert(rel.clone());
                        }
                    }
                } else {
                    // Hash unchanged (a touch, a `chmod`, or a same-size edit that turned out to
                    // be identical): refresh the stamp and size only.
                    //
                    // A file that was unreadable and is now readable again arrives here with its
                    // content unchanged, so its `read_error` finding and its stale flag must be
                    // cleared — the file is fine now, and a rebuild of the same tree has neither.
                    // Leaving them would keep `qdev validate` exiting 1 forever on a healthy
                    // workspace, which is a convergence break in exactly the case the readability
                    // detection made reachable.
                    clear_findings_for_path(&tx, rel)?;
                    finding_paths.remove(rel);
                    unstale_by_source_path(&tx, rel)?;
                    upsert_sync_state_row(&tx, rel, stamp, size, &hash)?;
                    unchanged += 1;
                }
            }

            // 2b. Whole-graph relation validation (kind pairs, dangling targets, depends_on
            // cycles). Runs every sweep against the full `entities`/`relations` tables, not just
            // the files touched this pass, so an out-of-band edit anywhere (e.g. a target file
            // deleted in a different edit) is caught the next time hydration runs.
            validate_relations_graph(&tx)?;

            // 3. Clear consumed dirty rows: entities whose file actually re-parsed this pass,
            //    plus orphaned rows for entities that were purged. A file that was unreadable,
            //    conflicted or schema-invalid keeps its dirty row so the next boot retries.
            {
                let mut stmt = tx
                    .prepare("SELECT id, source_path FROM entities;")
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to prepare dirty-clear sweep query: {}", e),
                        )
                    })?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to query entities for dirty-clear: {}", e),
                        )
                    })?;
                let mut to_clear: Vec<String> = Vec::new();
                for r in rows {
                    let (id, sp) = r.map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to read entity row for dirty-clear: {}", e),
                        )
                    })?;
                    if dirty_paths.contains(&sp) && parsed_paths.contains(&sp) {
                        to_clear.push(id);
                    }
                }
                for id in &to_clear {
                    tx.execute(
                        "DELETE FROM dirty_entities WHERE id = ?1;",
                        rusqlite::params![id],
                    )
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!("Failed to clear dirty entity '{}': {}", id, e),
                        )
                    })?;
                }
            }
            tx.execute(
                "DELETE FROM dirty_entities WHERE id NOT IN (SELECT id FROM entities);",
                [],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to clear orphaned dirty entities: {}", e),
                )
            })?;

            // Stamp freshness for this sweep pass, inside the same transaction as everything it
            // just parsed, so every boot-time and explicit sweep records when the cache was
            // last known consistent with disk.
            tx.execute(
                "INSERT INTO sync_meta (id, last_synced_at) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET last_synced_at = excluded.last_synced_at;",
                rusqlite::params![crate::write::current_iso8601()],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to stamp sync_meta during sweep: {}", e),
                )
            })?;

            // Count remaining findings before committing
            let findings: usize = tx
                .query_row("SELECT COUNT(*) FROM findings;", [], |row| row.get(0))
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to count findings for sweep summary: {}", e),
                    )
                })?;

            tx.commit().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to commit sweep transaction: {}", e),
                )
            })?;

            Ok(SweepSummary {
                parsed,
                unchanged,
                hashed,
                purged,
                findings,
            })
        })
    }
}

/// Creates every table in `ALL_TABLE_NAMES` that is not already present.
///
/// Deliberately does **not** stamp the version pragmas. `CREATE TABLE IF NOT EXISTS` leaves an
/// existing table alone even when its columns are from an older schema, so stamping here would
/// mark an unmigrated database as current: `inspect_cache_schema` would then report it Valid,
/// `ensure_cache` would skip the rebuild, and every sweep would die on the missing column with
/// no way out but deleting the cache by hand. Only callers that have just dropped the tables
/// (or created the database) may stamp — see `stamp_cache_version`.
pub fn create_schema(conn: &rusqlite::Connection) -> Result<(), QdevError> {
    conn.execute_batch(SCHEMA_DDL).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!(
                "Failed to create SQLite cache schema v{}: {}",
                CACHE_SCHEMA_VERSION, e
            ),
        )
    })?;

    Ok(())
}

/// Stamps `PRAGMA user_version` — the cache's single application-owned version stamp — to this
/// binary's `CACHE_SCHEMA_VERSION`. Only safe on a database whose tables are known to match
/// `SCHEMA_DDL` — a freshly created one, or one whose tables were just dropped and recreated.
///
/// This is the only place the stamp is written. `PRAGMA schema_version` is deliberately not
/// touched: it is SQLite's internal schema cookie, not an application field.
pub fn stamp_cache_version(conn: &rusqlite::Connection) -> Result<(), QdevError> {
    conn.execute_batch(&format!("PRAGMA user_version = {};", CACHE_SCHEMA_VERSION))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to set user_version pragma: {}", e),
            )
        })
}

/// Drops all triggers, views, and tables in the database, suspending foreign-key enforcement for
/// the duration of the drop.
///
/// **Both pragmas are set, and both are load-bearing**, because this helper is called from a
/// caller that has a transaction open (`init::initialize_cache`) and one that does not
/// (`SqliteStore::reset_and_rebuild`), and SQLite's two knobs cover one case each:
///
/// * `PRAGMA foreign_keys = OFF` is documented as a **no-op inside a transaction**, so it is what
///   protects the non-transactional caller and nothing else.
/// * `PRAGMA defer_foreign_keys = ON` works inside a transaction — it postpones enforcement to
///   `COMMIT`, by which point every child table has been dropped alongside its parent, so there
///   is no row left to violate anything. SQLite clears it automatically at each `COMMIT` or
///   `ROLLBACK`, so it needs no reset here.
///
/// Enforcement is on by default on every connection: the bundled SQLite is compiled with
/// `SQLITE_DEFAULT_FOREIGN_KEYS=1` (`libsqlite3-sys`'s `build.rs`), so a caller that never sets
/// the pragma still enforces. Setting only `foreign_keys = OFF` is what made this helper correct
/// for one caller and broken for the other: `qdev init` failed the first `DROP TABLE entities`
/// against any cache holding a child row.
pub fn drop_all_user_tables(conn: &rusqlite::Connection) -> Result<(), QdevError> {
    // 1. Drop triggers
    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='trigger' AND name NOT LIKE 'sqlite_%';",
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to query triggers for migration: {}", e),
            )
        })?;
    let trigger_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to iterate triggers for migration: {}", e),
            )
        })?
        .filter_map(|r| r.ok())
        .collect();
    for trigger in trigger_names {
        let escaped = trigger.replace('"', "\"\"");
        conn.execute(&format!("DROP TRIGGER IF EXISTS \"{}\";", escaped), [])
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to drop trigger {} during migration: {}", trigger, e),
                )
            })?;
    }

    // 2. Drop views
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='view' AND name NOT LIKE 'sqlite_%';")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to query views for migration: {}", e),
            )
        })?;
    let view_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to iterate views for migration: {}", e),
            )
        })?
        .filter_map(|r| r.ok())
        .collect();
    for view in view_names {
        let escaped = view.replace('"', "\"\"");
        conn.execute(&format!("DROP VIEW IF EXISTS \"{}\";", escaped), [])
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to drop view {} during migration: {}", view, e),
                )
            })?;
    }

    // 3. Drop tables with foreign keys disabled
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to query tables for migration: {}", e),
            )
        })?;

    let table_names: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to iterate tables for migration: {}", e),
            )
        })?
        .filter_map(|r| r.ok())
        .collect();

    // `foreign_keys` covers the non-transactional caller, `defer_foreign_keys` the transactional
    // one — see this function's documentation. Neither substitutes for the other.
    conn.execute_batch("PRAGMA foreign_keys = OFF; PRAGMA defer_foreign_keys = ON;")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to suspend foreign keys: {}", e),
            )
        })?;

    for table in table_names {
        let escaped = table.replace('"', "\"\"");
        conn.execute(&format!("DROP TABLE IF EXISTS \"{}\";", escaped), [])
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to drop table {} during migration: {}", table, e),
                )
            })?;
    }

    // Restores the connection default for the non-transactional caller; a no-op inside a
    // transaction, where `defer_foreign_keys` is doing the work and SQLite clears it at COMMIT.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to re-enable foreign keys: {}", e),
            )
        })?;

    Ok(())
}

/// The single `schema_version_mismatch` conflict raised for a cache stamped by a newer binary.
///
/// Shared by `ensure_cache` (boot) and `init::verify_cache_compatible` so both refuse identically:
/// rebuilding such a cache would silently discard whatever the newer binary recorded. The
/// message names `qdev sync --rebuild` because that is the documented in-tool recovery, the only
/// command allowed past this refusal — and also names deleting the cache file, because two of
/// this conflict's three call sites are in `init`, where there is no workspace yet and
/// `requires_workspace` refuses `sync` before it can run.
pub fn newer_cache_conflict(found: u32, supported: u32) -> QdevError {
    QdevError::conflict(
        "schema_version_mismatch",
        format!(
            "Cache database schema version v{} is newer than supported version v{}; \
             run `qdev sync --rebuild` to discard it and rebuild the cache from files, \
             or delete the cache file",
            found, supported
        ),
    )
    .with_details(serde_json::json!({
        "current_version": found,
        "supported_version": supported,
        "recovery": "qdev sync --rebuild, or delete the cache file",
    }))
}

/// Inspects an existing database path and returns whether its schema is valid, mismatched, or
/// stamped newer than this binary supports.
pub fn inspect_cache_schema(path: &Path) -> Result<CacheSchemaStatus, QdevError> {
    if !path.exists() {
        return Ok(CacheSchemaStatus::Mismatch);
    }

    let conn = rusqlite::Connection::open(path).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to open cache database: {}", e),
        )
    })?;

    conn.pragma_update(None, "busy_timeout", BUSY_TIMEOUT_MS)
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to set busy_timeout: {}", e),
            )
        })?;

    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to read user_version: {}", e),
            )
        })?;

    // Only `user_version` is consulted. SQLite's internal `schema_version` cookie moves on every
    // DDL statement — including the `CREATE TABLE`s of a perfectly healthy fresh cache — so
    // comparing it produced nothing but false negatives and destructive rebuilds.
    if user_version > CACHE_SCHEMA_VERSION {
        return Ok(CacheSchemaStatus::NewerThanSupported {
            found: user_version,
            supported: CACHE_SCHEMA_VERSION,
        });
    }
    if user_version != CACHE_SCHEMA_VERSION {
        return Ok(CacheSchemaStatus::Mismatch);
    }

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to query tables: {}", e),
            )
        })?;

    let existing_tables: HashSet<String> = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to iterate tables: {}", e),
            )
        })?
        .filter_map(|r| r.ok())
        .collect();

    for &table in ALL_TABLE_NAMES {
        if !existing_tables.contains(table) {
            return Ok(CacheSchemaStatus::Mismatch);
        }
    }

    if existing_tables.len() != ALL_TABLE_NAMES.len() {
        return Ok(CacheSchemaStatus::Mismatch);
    }

    // The v2 DDL is `CREATE TABLE IF NOT EXISTS`, so running it against a v1 database creates
    // `findings` and stamps the pragmas without adding `entities.stale`. Checking the column
    // keeps such a half-migrated cache reported as a mismatch, so it is rebuilt rather than
    // trusted and then failing on every sweep.
    let has_stale: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('entities') WHERE name = 'stale';",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to inspect entities columns: {}", e),
            )
        })?;
    if !has_stale {
        return Ok(CacheSchemaStatus::Mismatch);
    }

    Ok(CacheSchemaStatus::Valid)
}

/// Boot-time verification and initialization of the SQLite cache database.
/// Ensures `.qdev/cache/cache.sqlite` exists with every table in `ALL_TABLE_NAMES`, WAL mode,
/// busy_timeout=5000, and `PRAGMA user_version` at `CACHE_SCHEMA_VERSION`. Automatically
/// rebuilds from files on a missing cache or a version mismatch, and runs the incremental
/// hydration sweep on every healthy boot.
///
/// A cache stamped *newer* than this binary supports is refused with the same
/// `schema_version_mismatch` conflict `init::verify_cache_compatible` raises, rather than rebuilt.
pub fn ensure_cache(
    workspace_root: &Path,
    storage: &StorageConfig,
) -> Result<SqliteStore, QdevError> {
    let cache_dir = workspace_root.join(&storage.cache_dir);
    if !cache_dir.exists() {
        fs::create_dir_all(&cache_dir).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create cache directory '{}': {}",
                    cache_dir.display(),
                    e
                ),
            )
        })?;
    }

    let cache_db_path = cache_dir.join("cache.sqlite");
    let lock_path = cache_dir.join("write.lock");

    let needs_rebuild = if !cache_db_path.exists() {
        true
    } else {
        match inspect_cache_schema(&cache_db_path) {
            Ok(CacheSchemaStatus::Valid) => false,
            Ok(CacheSchemaStatus::Mismatch) => true,
            Ok(CacheSchemaStatus::NewerThanSupported { found, supported }) => {
                return Err(newer_cache_conflict(found, supported));
            }
            Err(_) => true,
        }
    };

    if needs_rebuild {
        let _guard = crate::write::acquire_write_lock(
            &lock_path,
            std::time::Duration::from_millis(BUSY_TIMEOUT_MS),
        )?;

        let still_needs_rebuild = if !cache_db_path.exists() {
            true
        } else {
            match inspect_cache_schema(&cache_db_path) {
                Ok(CacheSchemaStatus::Valid) => false,
                Ok(CacheSchemaStatus::Mismatch) => true,
                // Re-checked under the write lock: another process may have upgraded the cache
                // between the first inspection and here. Refuse rather than rebuild over it.
                Ok(CacheSchemaStatus::NewerThanSupported { found, supported }) => {
                    return Err(newer_cache_conflict(found, supported));
                }
                Err(_) => true,
            }
        };

        let store = match SqliteStore::open(&cache_db_path) {
            Ok(s) => s,
            Err(_) if still_needs_rebuild => {
                let _ = fs::remove_file(&cache_db_path);
                let _ = fs::remove_file(cache_dir.join("cache.sqlite-wal"));
                let _ = fs::remove_file(cache_dir.join("cache.sqlite-shm"));
                SqliteStore::open(&cache_db_path)?
            }
            Err(e) => return Err(e),
        };
        if still_needs_rebuild {
            store.reset_and_rebuild(workspace_root, storage)?;
        }
        Ok(store)
    } else {
        let store = SqliteStore::open(&cache_db_path)?;
        store.sweep_workspace(workspace_root, storage)?;
        Ok(store)
    }
}

/// Role of a scanned file during a sweep, driving which hydration routine processes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SweepFileRole {
    Markdown,
    Scratch,
    Evidence,
    Config,
}

/// Outcome of hydrating a single Markdown entity file. Shared by the full rebuild and the
/// incremental sweep so both converge on identical findings/stale state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HydrateOutcome {
    Parsed { id: String },
    MergeConflict,
    SchemaViolation { errors: Vec<ValidationError> },
    NoId,
}

/// Checks whether a cached JSON-array-of-strings column (e.g. `owners`, `target_modules`)
/// contains `value` as an exact element. Used for `list_entities`'s `owner`/`module` filters
/// instead of a SQL `LIKE` pattern, which cannot express exact membership without either
/// unescaped-wildcard false positives or SQLite's default case-insensitive matching.
fn json_string_array_contains(raw: Option<&str>, value: &str) -> bool {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .is_some_and(|arr| arr.iter().any(|v| v == value))
}

fn relative_path(workspace_root: &Path, file_path: &Path) -> String {
    file_path
        .strip_prefix(workspace_root)
        .unwrap_or(file_path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Nanoseconds in a second — the unit `file_mtime_size` reports an mtime in.
const NANOS_PER_SEC: i64 = 1_000_000_000;

/// The file's modification time in **nanoseconds** since the epoch, at whatever precision the
/// filesystem actually provides, plus its size.
///
/// Whole seconds (what this returned before) made a same-length edit landing in the same
/// wall-clock second as the previous hydration invisible *forever*, and both APFS and ext4 record
/// sub-second mtimes. Nanoseconds since 1970 fit an `i64` until 2262, and `sync_state.mtime` is
/// already an `INTEGER` column, so the wider value needs no schema change: a value written by an
/// older binary (whole seconds, ~1.7e9) compares unequal to a nanosecond one (~1.7e18), so every
/// file re-parses once after the upgrade and the cache self-heals.
///
/// A whole-second mtime is still possible — a coarse-granularity filesystem, or a file whose
/// nanoseconds happen to be zero — and the sweep's change gate treats that as unresolved rather
/// than as proof of no change.
/// Whether an apparently-unchanged file might still have been edited — the case a stamp
/// comparison cannot settle.
///
/// True when the stamp and size are unchanged *and* the stamp sits exactly on a second boundary.
/// A whole-second stamp is the signature of a filesystem whose timestamp granularity is one
/// second (HFS+, some network mounts): there, equality proves "same second", not "same content".
/// On a full-precision filesystem an edit moves the nanoseconds, so `meta_changed` sees it and
/// this never fires — which is why the price is paid only where it buys something.
///
/// Extracted as a predicate because it is otherwise unreachable in a test on a fine-grained
/// filesystem: the stamp folds in ctime, which no API lets a test set.
pub fn stamp_cannot_resolve_the_edit(
    stored_stamp: i64,
    stamp: i64,
    stored_size: i64,
    size: i64,
) -> bool {
    stored_stamp == stamp && stored_size == size && stamp % NANOS_PER_SEC == 0
}

/// The change stamp stored in `sync_state.mtime`, and the file's size.
///
/// The stamp is the **later of mtime and ctime**, in nanoseconds, from the one `stat` the sweep
/// already makes. Two reasons, both learned the hard way:
///
/// - Nanoseconds rather than whole seconds, because a same-length edit inside one wall-clock
///   second was otherwise invisible forever — including an `id` edit, which left a row for an id
///   no file declared.
/// - ctime as well as mtime, because a permission change bumps ctime and leaves mtime alone. That
///   is how a file becomes unreadable without "changing": the sweep skipped it and recorded no
///   `read_error` while a rebuild reported one. Folding ctime in costs nothing — it comes from the
///   same `stat` — where confirming readability by opening every skipped file cost 1,000 syscalls
///   per boot and doubled the warm sweep (6 ms to 14 ms at N=1000).
///
/// ctime is only available on Unix. On Windows the stamp is mtime alone, so a permission-only
/// change there is not seen until the file is otherwise touched; making a file unreadable on
/// Windows takes an ACL edit, which no qdev operation performs.
fn file_change_stamp(file_path: &Path) -> (i64, u64) {
    let metadata = fs::metadata(file_path).ok();
    let mtime = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| i64::try_from(d.as_nanos()).unwrap_or(i64::MAX))
        .unwrap_or(0);

    #[cfg(unix)]
    let stamp = {
        use std::os::unix::fs::MetadataExt;
        let ctime = metadata
            .as_ref()
            .map(|m| {
                let secs = m.ctime().saturating_mul(1_000_000_000);
                secs.saturating_add(m.ctime_nsec())
            })
            .unwrap_or(0);
        mtime.max(ctime)
    };
    #[cfg(not(unix))]
    let stamp = mtime;

    let size = metadata.map(|m| m.len()).unwrap_or(0);
    (stamp, size)
}

fn upsert_sync_state_row(
    tx: &rusqlite::Transaction,
    rel_path: &str,
    mtime: i64,
    size: u64,
    content_hash: &str,
) -> Result<(), QdevError> {
    tx.execute(
        r#"
INSERT INTO sync_state (path, mtime, size, content_hash)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(path) DO UPDATE SET
    mtime = excluded.mtime,
    size = excluded.size,
    content_hash = excluded.content_hash;
"#,
        rusqlite::params![rel_path, mtime, size, content_hash],
    )
    .map(|_| ())
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to upsert sync_state for '{}': {}", rel_path, e),
        )
    })
}

fn clear_findings_for_path(tx: &rusqlite::Transaction, path: &str) -> Result<(), QdevError> {
    tx.execute(
        "DELETE FROM findings WHERE path = ?1;",
        rusqlite::params![path],
    )
    .map(|_| ())
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to clear findings for '{}': {}", path, e),
        )
    })
}

/// Records the single answer both hydration paths give an unreadable file: the path's findings
/// are reset to one `read_error`, and any rows it owns are retained flagged stale (a full
/// rebuild has no such rows, so there the stale flag is a no-op and the two paths agree).
fn record_read_error(
    tx: &rusqlite::Transaction,
    rel_path: &str,
    err: &std::io::Error,
) -> Result<(), QdevError> {
    clear_findings_for_path(tx, rel_path)?;
    record_finding(
        tx,
        rel_path,
        "read_error",
        "error",
        &format!("File could not be read: {}", err),
    )?;
    flag_stale_by_source_path(tx, rel_path)
}

fn record_finding(
    tx: &rusqlite::Transaction,
    path: &str,
    code: &str,
    severity: &str,
    message: &str,
) -> Result<(), QdevError> {
    tx.execute(
        r#"
INSERT INTO findings (path, code, severity, message, message_key, found_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(path, code, message_key) DO UPDATE SET
    severity = excluded.severity,
    message = excluded.message,
    found_at = excluded.found_at;
"#,
        rusqlite::params![
            path,
            code,
            severity,
            message,
            message,
            crate::write::current_iso8601()
        ],
    )
    .map(|_| ())
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to record finding '{}' for '{}': {}", code, path, e),
        )
    })
}

/// Whole-graph relation validation: recomputes `dangling_relation`, `invalid_relation_kind`,
/// and `dependency_cycle` findings from scratch against the current `entities`/`relations`
/// tables. Clears the three codes for every path first (a relation problem that no longer
/// exists must stop being reported even for a file that was not itself reparsed this pass),
/// then re-derives them:
///   - a relation whose target isn't in `entities` -> `dangling_relation` on the source's path;
///   - otherwise, a `(relation, source_kind, target_kind)` not in `dag::allowed_kind_pairs` ->
///     `invalid_relation_kind` on the source's path;
///   - a cycle in the `depends_on` subgraph -> `dependency_cycle` on every participant's path,
///     naming the full cycle. Repeats after removing each found cycle's closing edge from the
///     working edge list (not from storage) so multiple disjoint cycles are all found.
fn validate_relations_graph(tx: &rusqlite::Transaction) -> Result<(), QdevError> {
    tx.execute(
        "DELETE FROM findings WHERE code IN ('dangling_relation', 'invalid_relation_kind', 'dependency_cycle');",
        [],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to clear relation-validation findings: {}", e),
        )
    })?;

    // id -> (kind, source_path)
    let mut entity_info: HashMap<String, (EntityKind, String)> = HashMap::new();
    {
        let mut stmt = tx
            .prepare(
                // The one place the stale rule is written in SQL rather than through
                // `Store::entity_presence_for_derivation`: this is whole-graph validation, where
                // a per-row helper would trade one query for N. Named as a deliberate exception
                // in `architecture.md` §11 for that reason.
                //
                // `stale = 0` only: a stale row holds pre-edit content retained so reads keep
                // working, and a full rebuild of the same tree has no row for it at all. Counting
                // it as present would suppress the `dangling_relation` a rebuild reports on every
                // edge pointing into it — the convergence this story exists to restore.
                "SELECT id, kind, source_path FROM entities WHERE stale = 0;",
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to prepare entity lookup for relation validation: {}",
                        e
                    ),
                )
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to query entities for relation validation: {}", e),
                )
            })?;
        for r in rows {
            let (id, kind_str, path) = r.map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to read entity row for relation validation: {}", e),
                )
            })?;
            if let Ok(kind) = EntityKind::from_str_loose(&kind_str) {
                entity_info.insert(id, (kind, path));
            }
        }
    }

    let mut relations: Vec<(String, String, String)> = Vec::new();
    {
        let mut stmt = tx
            .prepare(
                // Only edges declared by a *live* entity. A stale entity's retained edges are
                // pre-edit content that a full rebuild of the same tree does not have, so
                // counting them would let a `dependency_cycle` be reported for a cycle that only
                // closes through content the file no longer declares. Edges pointing *into* a
                // stale entity are still here — their source is live — and are reported as
                // dangling, which is what a rebuild does too.
                "SELECT r.source_id, r.relation, r.target_id FROM relations r \
                 JOIN entities e ON e.id = r.source_id AND e.stale = 0 \
                 ORDER BY r.source_id, r.relation, r.target_id;",
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to prepare relation query for validation: {}", e),
                )
            })?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to query relations for validation: {}", e),
                )
            })?;
        for r in rows {
            relations.push(r.map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to read relation row for validation: {}", e),
                )
            })?);
        }
    }

    // Dangling target / invalid kind pair, per relation.
    for (source_id, relation, target_id) in &relations {
        let Some((source_kind, source_path)) = entity_info.get(source_id) else {
            // The owning entity row is missing; relations are cleared alongside their source
            // entity (purge and re-parse both clear owned relation rows first), so this should
            // not normally happen. Nothing to attach a finding to.
            continue;
        };
        match entity_info.get(target_id) {
            None => {
                record_finding(
                    tx,
                    source_path,
                    "dangling_relation",
                    "error",
                    &format!(
                        "Relation '{}' on '{}' targets '{}', which does not exist",
                        relation, source_id, target_id
                    ),
                )?;
            }
            Some((target_kind, _)) => {
                if !crate::dag::is_valid_kind_pair(relation, *source_kind, *target_kind) {
                    record_finding(
                        tx,
                        source_path,
                        "invalid_relation_kind",
                        "error",
                        &format!(
                            "Relation '{}' from {} ({}) to {} ({}) is not an allowed kind pair",
                            relation,
                            source_id,
                            source_kind.as_str(),
                            target_id,
                            target_kind.as_str()
                        ),
                    )?;
                }
            }
        }
    }

    // depends_on cycles: whole-graph scan, repeated (breaking only the working edge list, never
    // storage) until acyclic, so every disjoint cycle out-of-band edits may have introduced is
    // found and reported, not just the first.
    let mut depends_on_edges: Vec<(String, String)> = relations
        .iter()
        .filter(|(_, relation, _)| relation == "depends_on")
        .map(|(source_id, _, target_id)| (source_id.clone(), target_id.clone()))
        .collect();

    while let Some(cycle) = crate::dag::find_dependency_cycle(&depends_on_edges) {
        let message = format!("depends_on cycle: {}", cycle.join(" -> "));
        let mut recorded: HashSet<&str> = HashSet::new();
        for node in &cycle {
            if !recorded.insert(node.as_str()) {
                continue;
            }
            if let Some((_, path)) = entity_info.get(node) {
                record_finding(tx, path, "dependency_cycle", "error", &message)?;
            }
        }
        if cycle.len() < 2 {
            break;
        }
        let (a, b) = (
            cycle[cycle.len() - 2].clone(),
            cycle[cycle.len() - 1].clone(),
        );
        depends_on_edges.retain(|(s, t)| !(*s == a && *t == b));
    }

    Ok(())
}

/// Clears the stale flag on every entity row owned by `source_path`.
///
/// The inverse of [`flag_stale_by_source_path`], for a file that failed to parse or read and now
/// succeeds with content the cache already had: the rows are current again, and a full rebuild of
/// the same tree would have no stale flag to carry.
fn unstale_by_source_path(tx: &rusqlite::Transaction, source_path: &str) -> Result<(), QdevError> {
    tx.execute(
        "UPDATE entities SET stale = 0 WHERE source_path = ?1;",
        rusqlite::params![source_path],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!(
                "Failed to clear the stale flag for '{}': {}",
                source_path, e
            ),
        )
    })?;
    Ok(())
}

/// Flags every entity row owned by `source_path` as stale (previous state retained).
fn flag_stale_by_source_path(
    tx: &rusqlite::Transaction,
    source_path: &str,
) -> Result<(), QdevError> {
    tx.execute(
        "UPDATE entities SET stale = 1 WHERE source_path = ?1;",
        rusqlite::params![source_path],
    )
    .map(|_| ())
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to flag stale entities for '{}': {}", source_path, e),
        )
    })
}

/// Maps a sprint entity id to the integer key used by the `sprints` / `sprint_assignments`
/// tables. Shared by hydration and purge so both paths agree on the key for every id shape.
fn sprint_number(id: &str) -> i64 {
    id.strip_prefix("sprint-")
        .unwrap_or(id)
        .parse::<i64>()
        .unwrap_or(0)
}

/// Deletes the kind-specific detail row a single entity owns (`stories`, `sprints` +
/// `sprint_assignments`, `deferred_work`, `decisions`, `soup_dependencies`, `gate_runs`).
/// Shared by the purge cascade and by re-parse, which must drop the previous kind's row when
/// an entity changes kind in place.
pub(crate) fn delete_kind_detail_row(
    tx: &rusqlite::Transaction,
    id: &str,
    kind: EntityKind,
) -> Result<(), QdevError> {
    let (sql, param): (&str, Box<dyn rusqlite::ToSql>) = match kind {
        EntityKind::Sprint => (
            "DELETE FROM sprints WHERE id = ?1;",
            Box::new(sprint_number(id)),
        ),
        EntityKind::Story => (
            "DELETE FROM stories WHERE id = ?1;",
            Box::new(id.to_string()),
        ),
        EntityKind::DeferredWork => (
            "DELETE FROM deferred_work WHERE id = ?1;",
            Box::new(id.to_string()),
        ),
        EntityKind::Decision => (
            "DELETE FROM decisions WHERE id = ?1;",
            Box::new(id.to_string()),
        ),
        EntityKind::Soup => (
            "DELETE FROM soup_dependencies WHERE id = ?1;",
            Box::new(id.to_string()),
        ),
        EntityKind::Evidence => (
            "DELETE FROM gate_runs WHERE id = ?1;",
            Box::new(id.to_string()),
        ),
        _ => return Ok(()),
    };

    tx.execute(sql, rusqlite::params![param]).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to purge {} row for '{}': {}", kind.as_str(), id, e),
        )
    })?;
    Ok(())
}

/// Deletes the child rows an entity owns before it is re-hydrated, so a re-parse *replaces*
/// them instead of merging into rows the file no longer declares. The full rebuild gets the
/// same effect by truncating every table first; the sweep must do it per entity.
fn clear_owned_child_rows(
    tx: &rusqlite::Transaction,
    id: &str,
    new_kind: EntityKind,
) -> Result<(), QdevError> {
    tx.execute(
        "DELETE FROM constraints WHERE owner_id = ?1;",
        rusqlite::params![id],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to clear constraints for entity '{}': {}", id, e),
        )
    })?;
    tx.execute(
        "DELETE FROM relations WHERE source_id = ?1;",
        rusqlite::params![id],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to clear relations for entity '{}': {}", id, e),
        )
    })?;

    // An in-place kind change leaves the previous kind's detail row behind.
    let prev_kind: Option<String> = tx
        .query_row(
            "SELECT kind FROM entities WHERE id = ?1;",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to read previous kind for entity '{}': {}", id, e),
            )
        })?;
    if let Some(prev) = prev_kind {
        if let Ok(prev_kind) = EntityKind::from_str_loose(&prev) {
            if prev_kind != new_kind {
                delete_kind_detail_row(tx, id, prev_kind)?;
            }
        }
    }

    // Sprint assignments are keyed by sprint number, not entity id, so an upsert alone never
    // removes an assignment the file dropped.
    if new_kind == EntityKind::Sprint {
        tx.execute(
            "DELETE FROM sprint_assignments WHERE sprint_id = ?1;",
            rusqlite::params![sprint_number(id)],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to clear sprint assignments for '{}': {}", id, e),
            )
        })?;
    }

    Ok(())
}

/// Deletes an entity row plus every cache row the entity's own file owned: its detail row, its
/// constraints, and the relations it *declared* (`source_id`).
///
/// Relations pointing **into** the entity (`target_id`) are deliberately left alone: an edge is
/// a fact declared by the file that names it, and that file is unchanged and will not be
/// re-parsed, so deleting its edge here destroys state no later pass restores. Keeping the row
/// is what lets `validate_relations_graph` report `dangling_relation` — the same answer a full
/// rebuild of the tree gives, since the rebuild re-reads the declaring file. The `relations`
/// table has no foreign key precisely so a row pointing at an absent entity is representable.
fn purge_entity_with_children(
    tx: &rusqlite::Transaction,
    id: &str,
    kind: EntityKind,
) -> Result<(), QdevError> {
    if kind == EntityKind::Sprint {
        tx.execute(
            "DELETE FROM sprint_assignments WHERE sprint_id = ?1;",
            rusqlite::params![sprint_number(id)],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to purge sprint assignments for '{}': {}", id, e),
            )
        })?;
    }
    delete_kind_detail_row(tx, id, kind)?;

    tx.execute("DELETE FROM stories WHERE id = ?1;", rusqlite::params![id])
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to purge story row for entity '{}': {}", id, e),
            )
        })?;
    tx.execute(
        "DELETE FROM constraints WHERE owner_id = ?1;",
        rusqlite::params![id],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to purge constraints for entity '{}': {}", id, e),
        )
    })?;
    tx.execute(
        "DELETE FROM relations WHERE source_id = ?1;",
        rusqlite::params![id],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to purge relations for entity '{}': {}", id, e),
        )
    })?;
    tx.execute("DELETE FROM entities WHERE id = ?1;", rusqlite::params![id])
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to purge entity '{}': {}", id, e),
            )
        })?;
    Ok(())
}

fn entity_rows_for_source_path(
    tx: &rusqlite::Transaction,
    source_path: &str,
) -> Result<Vec<(String, EntityKind)>, QdevError> {
    let mut stmt = tx
        .prepare("SELECT id, kind FROM entities WHERE source_path = ?1;")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!(
                    "Failed to prepare source-path entity lookup for '{}': {}",
                    source_path, e
                ),
            )
        })?;
    let rows = stmt
        .query_map(rusqlite::params![source_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!(
                    "Failed to query entities for source path '{}': {}",
                    source_path, e
                ),
            )
        })?;
    let mut owned = Vec::new();
    for r in rows {
        let (id, kind_str) = r.map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!(
                    "Failed to read entity row for source path '{}': {}",
                    source_path, e
                ),
            )
        })?;
        let kind = EntityKind::from_str_loose(&kind_str).unwrap_or(EntityKind::Story);
        owned.push((id, kind));
    }
    Ok(owned)
}

/// Purges every cache row owned by a removed file path: entity rows (with child cascade and the
/// relations those entities declared), scratchpad entries by story id, gate runs by evidence
/// path, gates when qdev.toml disappears, the sync_state row, and the path's findings.
///
/// Rows owned by *other* files are never touched — in particular relations pointing into a
/// purged entity survive and are reported as `dangling_relation`.
fn purge_removed_path(tx: &rusqlite::Transaction, rel_path: &str) -> Result<(), QdevError> {
    for (entity_id, kind) in entity_rows_for_source_path(tx, rel_path)? {
        purge_entity_with_children(tx, &entity_id, kind)?;
    }

    if rel_path.ends_with(".jsonl") {
        if let Some(story_id) = Path::new(rel_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
        {
            tx.execute(
                "DELETE FROM scratchpad_entries WHERE story_id = ?1;",
                rusqlite::params![story_id],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to purge scratchpad entries for removed '{}': {}",
                        rel_path, e
                    ),
                )
            })?;
        }
    }
    if rel_path.ends_with(".json") {
        tx.execute(
            "DELETE FROM gate_runs WHERE evidence_path = ?1;",
            rusqlite::params![rel_path],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!(
                    "Failed to purge gate runs for removed '{}': {}",
                    rel_path, e
                ),
            )
        })?;
    }
    if rel_path == "qdev.toml" {
        tx.execute("DELETE FROM gates;", []).map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to clear gates for removed '{}': {}", rel_path, e),
            )
        })?;
    }

    tx.execute(
        "DELETE FROM sync_state WHERE path = ?1;",
        rusqlite::params![rel_path],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to purge sync_state for '{}': {}", rel_path, e),
        )
    })?;
    clear_findings_for_path(tx, rel_path)?;
    Ok(())
}

/// Refreshes the gates table from qdev.toml content (clears first so removed gates drop).
fn refresh_gates(tx: &rusqlite::Transaction, content: &str) -> Result<(), QdevError> {
    tx.execute("DELETE FROM gates;", []).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to clear gates table: {}", e),
        )
    })?;

    if let Ok(parsed_toml) = toml::from_str::<toml::Value>(content) {
        if let Some(toml::Value::Array(gates)) = parsed_toml.get("gates") {
            for g in gates {
                if let Some(gate_table) = g.as_table() {
                    if let Some(id) = gate_table.get("id").and_then(|v| v.as_str()) {
                        let cmd = gate_table
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        let kind = gate_table.get("kind").and_then(|v| v.as_str());
                        let timeout_ms = gate_table
                            .get("timeout_ms")
                            .and_then(|v| v.as_integer())
                            .filter(|&i| i >= 0)
                            .map(|i| i as u64);
                        let output_adapter =
                            gate_table.get("output_adapter").and_then(|v| v.as_str());
                        let on_trans = gate_table
                            .get("on_transition")
                            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()));
                        let deps = gate_table
                            .get("depends_on")
                            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()));
                        let metric = gate_table.get("metric").and_then(|v| v.as_str());
                        let direction = gate_table.get("direction").and_then(|v| v.as_str());

                        tx.execute(
                            r#"
INSERT INTO gates (id, command, kind, timeout_ms, output_adapter, on_transition, depends_on, metric, direction)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(id) DO UPDATE SET
    command = excluded.command,
    kind = excluded.kind,
    timeout_ms = excluded.timeout_ms,
    output_adapter = excluded.output_adapter,
    on_transition = excluded.on_transition,
    depends_on = excluded.depends_on,
    metric = excluded.metric,
    direction = excluded.direction;
"#,
                            rusqlite::params![
                                id, cmd, kind, timeout_ms, output_adapter, on_trans, deps, metric, direction
                            ],
                        )
                        .map_err(|e| {
                            QdevError::infrastructure_failure(
                                "sqlite_error",
                                format!("Failed to upsert gate '{}': {}", id, e),
                            )
                        })?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Shared per-Markdown-file hydration: records sync_state, then either parses and upserts the
/// entity (clearing stale) or records a merge_conflict / schema_violation finding and flags the
/// previous row stale. Used by both the full rebuild and the incremental sweep.
fn hydrate_markdown_file(
    tx: &rusqlite::Transaction,
    workspace_root: &Path,
    file_path: &Path,
    content: &str,
) -> Result<HydrateOutcome, QdevError> {
    let rel_path = relative_path(workspace_root, file_path);
    // A change *stamp* in nanoseconds, not an mtime in seconds — see `file_change_stamp`.
    let (stamp, size) = file_change_stamp(file_path);
    let content_hash = sha256_digest(content.as_bytes());

    // Upsert sync_state for every scanned file
    upsert_sync_state_row(tx, &rel_path, stamp, size, &content_hash)?;

    // Findings are per-path current state: reset this path's findings, then re-record if the
    // file is invalid. A successful parse therefore leaves no findings for the path.
    clear_findings_for_path(tx, &rel_path)?;

    // Conflict markers: never parse; record finding; keep previous row flagged stale
    if content.contains("<<<<<<<") {
        record_finding(
            tx,
            &rel_path,
            "merge_conflict",
            "error",
            "File contains merge conflict markers ('<<<<<<<') and was not parsed",
        )?;
        flag_stale_by_source_path(tx, &rel_path)?;
        return Ok(HydrateOutcome::MergeConflict);
    }

    let frontmatter = match extract_frontmatter(content) {
        Ok(fm) => fm,
        Err(e) => {
            record_finding(tx, &rel_path, "schema_violation", "error", &e.to_string())?;
            flag_stale_by_source_path(tx, &rel_path)?;
            return Ok(HydrateOutcome::SchemaViolation {
                errors: vec![ValidationError {
                    path: String::new(),
                    message: e.to_string(),
                }],
            });
        }
    };

    let id = match frontmatter.get("id").and_then(|v| v.as_str()) {
        Some(id) if !id.trim().is_empty() => id.to_string(),
        _ => {
            record_finding(
                tx,
                &rel_path,
                "schema_violation",
                "error",
                "Frontmatter has no non-empty 'id'",
            )?;
            flag_stale_by_source_path(tx, &rel_path)?;
            return Ok(HydrateOutcome::NoId);
        }
    };

    let kind = determine_entity_kind(file_path, &id, &frontmatter);
    if let Err(errors) = validate_value_detailed(kind, &frontmatter) {
        let message = errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        record_finding(tx, &rel_path, "schema_violation", "error", &message)?;
        flag_stale_by_source_path(tx, &rel_path)?;
        return Ok(HydrateOutcome::SchemaViolation { errors });
    }

    // In-place id edit: purge rows claiming this source_path under a different id
    // (child cascade + target-side relations) so no orphan rows survive.
    for (old_id, old_kind) in entity_rows_for_source_path(tx, &rel_path)? {
        if old_id != id {
            purge_entity_with_children(tx, &old_id, old_kind)?;
        }
    }

    // Re-parse replaces this entity's child rows: constraints, outbound relations, sprint
    // assignments and any detail row left by a previous kind. Upserts alone would keep rows
    // the file no longer declares, diverging from a full rebuild.
    clear_owned_child_rows(tx, &id, kind)?;

    let title = frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let status = frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let owners = frontmatter.get("owners").map(|v| v.to_string());
    let version = frontmatter
        .get("version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1);

    let c_type = frontmatter
        .get("created_by")
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_str());
    let c_id = frontmatter
        .get("created_by")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str());
    let u_type = frontmatter
        .get("updated_by")
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_str());
    let u_id = frontmatter
        .get("updated_by")
        .and_then(|v| v.get("id"))
        .and_then(|v| v.as_str());

    let updated_at = frontmatter
        .get("updated_at")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        // The stamp is nanoseconds; `iso8601_from_timestamp` takes seconds. Passing it straight
        // through wrote `56692569348-03-01T04:12:02Z` into `updated_at` for every file whose
        // frontmatter omits it — and no test read the column, so the suite stayed green.
        .unwrap_or_else(|| crate::write::iso8601_from_timestamp(stamp / NANOS_PER_SEC));

    // Upsert entities table (successful parse clears the stale flag)
    tx.execute(
        r#"
INSERT INTO entities (
    id, kind, title, status, owners, source_path, content_hash, version,
    created_by_type, created_by_id, updated_by_type, updated_by_id, updated_at, stale
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
ON CONFLICT(id) DO UPDATE SET
    kind = excluded.kind,
    title = excluded.title,
    status = excluded.status,
    owners = excluded.owners,
    source_path = excluded.source_path,
    content_hash = excluded.content_hash,
    version = excluded.version,
    created_by_type = excluded.created_by_type,
    created_by_id = excluded.created_by_id,
    updated_by_type = excluded.updated_by_type,
    updated_by_id = excluded.updated_by_id,
    updated_at = excluded.updated_at,
    stale = 0;
"#,
        rusqlite::params![
            id,
            kind.as_str(),
            title,
            status,
            owners,
            rel_path,
            content_hash,
            version,
            c_type,
            c_id,
            u_type,
            u_id,
            updated_at,
            0i64,
        ],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to upsert entity '{}': {}", id, e),
        )
    })?;

    // Kind-specific: Story
    if kind == EntityKind::Story {
        let epic_id = frontmatter
            .get("epic_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| {
                if let Ok(Identifier::Story { epic, .. }) = id.parse::<Identifier>() {
                    Some(format!("E{}", epic))
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let seq = frontmatter
            .get("seq")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .or_else(|| {
                if let Ok(Identifier::Story { story, .. }) = id.parse::<Identifier>() {
                    Some(story)
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let appetite = frontmatter.get("appetite").and_then(|v| v.as_str());
        let safety_class = frontmatter.get("safety_class").and_then(|v| v.as_str());
        let target_modules = frontmatter.get("target_modules").map(|v| v.to_string());

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
            rusqlite::params![id, epic_id, seq, appetite, safety_class, target_modules],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert story details '{}': {}", id, e),
            )
        })?;
    }

    // Constraints (from frontmatter of any entity)
    if let Some(serde_json::Value::Array(constraints)) = frontmatter.get("constraints") {
        for c in constraints {
            if let Some(c_id) = c.get("id").and_then(|v| v.as_str()) {
                let comp_id = if c_id.contains('/') {
                    c_id.to_string()
                } else {
                    format!("{}/{}", id, c_id)
                };
                let c_kind = c.get("kind").and_then(|v| v.as_str()).unwrap_or("no_go");
                let c_text = c.get("text").and_then(|v| v.as_str()).unwrap_or("");

                tx.execute(
                    r#"
INSERT INTO constraints (id, owner_id, kind, text)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(id) DO UPDATE SET
    owner_id = excluded.owner_id,
    kind = excluded.kind,
    text = excluded.text;
"#,
                    rusqlite::params![comp_id, id, c_kind, c_text],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to upsert constraint '{}': {}", comp_id, e),
                    )
                })?;
            }
        }
    }

    // Relations (from frontmatter of any entity)
    if let Some(serde_json::Value::Object(relations)) = frontmatter.get("relations") {
        for (rel_name, targets_val) in relations {
            if let Some(targets) = targets_val.as_array() {
                for target in targets {
                    if let Some(target_id) = target.as_str() {
                        tx.execute(
                            r#"
INSERT INTO relations (source_id, relation, target_id)
VALUES (?1, ?2, ?3)
ON CONFLICT(source_id, relation, target_id) DO NOTHING;
"#,
                            rusqlite::params![id, rel_name, target_id],
                        )
                        .map_err(|e| {
                            QdevError::infrastructure_failure(
                                "sqlite_error",
                                format!(
                                    "Failed to upsert relation '{}' -> '{}': {}",
                                    id, target_id, e
                                ),
                            )
                        })?;
                    }
                }
            }
        }
    }

    // Kind-specific: Sprint
    if kind == EntityKind::Sprint {
        let sprint_num: i64 = sprint_number(&id);

        let rel_ver = frontmatter
            .get("release_version")
            .or_else(|| frontmatter.get("release"))
            .and_then(|v| v.as_str());
        let started_at = frontmatter.get("started_at").and_then(|v| v.as_str());
        let completed_at = frontmatter.get("completed_at").and_then(|v| v.as_str());

        tx.execute(
            r#"
INSERT INTO sprints (id, title, release_version, status, owners, started_at, completed_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(id) DO UPDATE SET
    title = excluded.title,
    release_version = excluded.release_version,
    status = excluded.status,
    owners = excluded.owners,
    started_at = excluded.started_at,
    completed_at = excluded.completed_at;
"#,
            rusqlite::params![
                sprint_num,
                title,
                rel_ver,
                status,
                owners,
                started_at,
                completed_at
            ],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert sprint '{}': {}", id, e),
            )
        })?;

        if let Some(serde_json::Value::Array(assignments)) = frontmatter.get("assignments") {
            for a in assignments {
                let (story_id, assigned_at, carried_from) = match a {
                    serde_json::Value::String(s) => {
                        (s.clone(), started_at.unwrap_or("").to_string(), None)
                    }
                    serde_json::Value::Object(map) => {
                        let s_id = map
                            .get("story_id")
                            .or_else(|| map.get("story"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let at = map
                            .get("assigned_at")
                            .and_then(|v| v.as_str())
                            .unwrap_or(started_at.unwrap_or(""))
                            .to_string();
                        let carried = map.get("carried_from").and_then(|v| {
                            v.as_i64().or_else(|| {
                                v.as_str().and_then(|s| {
                                    s.strip_prefix("sprint-").unwrap_or(s).parse::<i64>().ok()
                                })
                            })
                        });
                        (s_id, at, carried)
                    }
                    _ => continue,
                };

                if !story_id.is_empty() {
                    tx.execute(
                        r#"
INSERT INTO sprint_assignments (sprint_id, story_id, assigned_at, carried_from)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT(sprint_id, story_id) DO UPDATE SET
    assigned_at = excluded.assigned_at,
    carried_from = excluded.carried_from;
"#,
                        rusqlite::params![sprint_num, story_id, assigned_at, carried_from],
                    )
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "sqlite_error",
                            format!(
                                "Failed to upsert sprint assignment for '{}': {}",
                                story_id, e
                            ),
                        )
                    })?;
                }
            }
        }
    }

    // Kind-specific: DeferredWork
    if kind == EntityKind::DeferredWork {
        let origin = frontmatter.get("origin_story_id").and_then(|v| v.as_str());
        let target_module = frontmatter
            .get("target_module")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let safety_risk = frontmatter.get("safety_risk").and_then(|v| v.as_str());
        let rationale = frontmatter.get("rationale").and_then(|v| v.as_str());
        let gate = frontmatter.get("gate").and_then(|v| v.as_str());
        let resolution = frontmatter.get("resolution").and_then(|v| v.as_str());

        tx.execute(
            r#"
INSERT INTO deferred_work (id, origin_story_id, target_module, status, safety_risk, rationale, gate, resolution)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
ON CONFLICT(id) DO UPDATE SET
    origin_story_id = excluded.origin_story_id,
    target_module = excluded.target_module,
    status = excluded.status,
    safety_risk = excluded.safety_risk,
    rationale = excluded.rationale,
    gate = excluded.gate,
    resolution = excluded.resolution;
"#,
            rusqlite::params![
                id,
                origin,
                target_module,
                status,
                safety_risk,
                rationale,
                gate,
                resolution
            ],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert deferred work '{}': {}", id, e),
            )
        })?;
    }

    // Kind-specific: Decision
    if kind == EntityKind::Decision {
        let subject_id = frontmatter
            .get("subject_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let dec_type = frontmatter.get("decision_type").and_then(|v| v.as_str());
        let topic = frontmatter.get("topic").and_then(|v| v.as_str());
        let context = frontmatter.get("context").and_then(|v| v.as_str());
        let ruling = frontmatter.get("ruling").and_then(|v| v.as_str());
        let created_at = frontmatter
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or(&updated_at);

        tx.execute(
            r#"
INSERT INTO decisions (id, subject_id, decision_type, topic, context, ruling, author_type, author_id, created_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT(id) DO UPDATE SET
    subject_id = excluded.subject_id,
    decision_type = excluded.decision_type,
    topic = excluded.topic,
    context = excluded.context,
    ruling = excluded.ruling,
    author_type = excluded.author_type,
    author_id = excluded.author_id,
    created_at = excluded.created_at;
"#,
            rusqlite::params![
                id, subject_id, dec_type, topic, context, ruling, c_type, c_id, created_at
            ],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert decision '{}': {}", id, e),
            )
        })?;
    }

    // Kind-specific: Soup
    if kind == EntityKind::Soup {
        let name = frontmatter.get("name").and_then(|v| v.as_str());
        let ver = frontmatter
            .get("dependency_version")
            .or_else(|| frontmatter.get("version"))
            .and_then(|v| v.as_str());
        let license = frontmatter.get("license").and_then(|v| v.as_str());
        let cve_status = frontmatter.get("cve_status").and_then(|v| v.as_str());
        let intro_story = frontmatter
            .get("introduced_by_story")
            .and_then(|v| v.as_str());
        let eval_rel = frontmatter
            .get("evaluated_for_release")
            .and_then(|v| v.as_str());

        tx.execute(
            r#"
INSERT INTO soup_dependencies (id, name, version, license, cve_status, introduced_by_story, evaluated_for_release)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(id) DO UPDATE SET
    name = excluded.name,
    version = excluded.version,
    license = excluded.license,
    cve_status = excluded.cve_status,
    introduced_by_story = excluded.introduced_by_story,
    evaluated_for_release = excluded.evaluated_for_release;
"#,
            rusqlite::params![id, name, ver, license, cve_status, intro_story, eval_rel],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert SOUP dependency '{}': {}", id, e),
            )
        })?;
    }

    // Kind-specific: Evidence
    if kind == EntityKind::Evidence {
        let story_id = frontmatter.get("story_id").and_then(|v| v.as_str());
        let gate_id = frontmatter
            .get("gate_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let commit_sha = frontmatter
            .get("commit_sha")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let ev_status = frontmatter.get("status").and_then(|v| v.as_str());
        let exit_code = frontmatter
            .get("exit_code")
            .and_then(|v| v.as_i64())
            .map(|i| i as i32);
        let duration_ms = frontmatter.get("duration_ms").and_then(|v| v.as_u64());
        let metric_val = frontmatter.get("metric_value").and_then(|v| v.as_f64());
        let summary = frontmatter.get("summary").and_then(|v| v.as_str());
        let output_hash = frontmatter.get("output_hash").and_then(|v| v.as_str());
        let ran_at = frontmatter.get("ran_at").and_then(|v| v.as_str());

        tx.execute(
            r#"
INSERT INTO gate_runs (
    id, story_id, gate_id, commit_sha, status, exit_code, duration_ms,
    metric_value, summary, evidence_path, output_hash, run_by_type, run_by_id, ran_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
ON CONFLICT(id) DO UPDATE SET
    story_id = excluded.story_id,
    gate_id = excluded.gate_id,
    commit_sha = excluded.commit_sha,
    status = excluded.status,
    exit_code = excluded.exit_code,
    duration_ms = excluded.duration_ms,
    metric_value = excluded.metric_value,
    summary = excluded.summary,
    evidence_path = excluded.evidence_path,
    output_hash = excluded.output_hash,
    run_by_type = excluded.run_by_type,
    run_by_id = excluded.run_by_id,
    ran_at = excluded.ran_at;
"#,
            rusqlite::params![
                id,
                story_id,
                gate_id,
                commit_sha,
                ev_status,
                exit_code,
                duration_ms,
                metric_val,
                summary,
                rel_path,
                output_hash,
                c_type,
                c_id,
                ran_at
            ],
        )
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to upsert gate run '{}': {}", id, e),
            )
        })?;
    }

    Ok(HydrateOutcome::Parsed { id })
}

/// Shared scratchpad (.jsonl) hydration: records sync_state and replaces the story's entries.
fn hydrate_scratch_file(
    tx: &rusqlite::Transaction,
    workspace_root: &Path,
    file_path: &Path,
    content: &str,
) -> Result<(), QdevError> {
    let rel_path = relative_path(workspace_root, file_path);
    let (mtime, size) = file_change_stamp(file_path);
    let content_hash = sha256_digest(content.as_bytes());
    upsert_sync_state_row(tx, &rel_path, mtime, size, &content_hash)?;

    let story_id = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();

    // Re-parse replaces the story's entries so lines removed from the file don't linger
    tx.execute(
        "DELETE FROM scratchpad_entries WHERE story_id = ?1;",
        rusqlite::params![story_id],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!(
                "Failed to clear scratchpad entries for '{}': {}",
                story_id, e
            ),
        )
    })?;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(entry_json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let seq = entry_json.get("seq").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let at = entry_json.get("at").and_then(|v| v.as_str()).unwrap_or("");
            let (author_type, author_id) =
                if let Some(author_obj) = entry_json.get("author").and_then(|v| v.as_object()) {
                    let t = author_obj
                        .get("type")
                        .and_then(|v| v.as_str())
                        .or_else(|| entry_json.get("author_type").and_then(|v| v.as_str()));
                    let id = author_obj
                        .get("id")
                        .and_then(|v| v.as_str())
                        .or_else(|| author_obj.get("name").and_then(|v| v.as_str()))
                        .or_else(|| entry_json.get("author_id").and_then(|v| v.as_str()));
                    (t, id)
                } else {
                    (
                        entry_json.get("author_type").and_then(|v| v.as_str()),
                        entry_json.get("author_id").and_then(|v| v.as_str()),
                    )
                };
            let entry_kind = entry_json.get("kind").and_then(|v| v.as_str());
            let text = entry_json.get("text").and_then(|v| v.as_str());

            tx.execute(
                r#"
INSERT INTO scratchpad_entries (story_id, seq, at, author_type, author_id, kind, text)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT(story_id, seq) DO UPDATE SET
    at = excluded.at,
    author_type = excluded.author_type,
    author_id = excluded.author_id,
    kind = excluded.kind,
    text = excluded.text;
"#,
                rusqlite::params![story_id, seq, at, author_type, author_id, entry_kind, text],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!(
                        "Failed to upsert scratchpad entry for '{}:{}': {}",
                        story_id, seq, e
                    ),
                )
            })?;
        }
    }
    Ok(())
}

/// Shared evidence (.json) hydration: records sync_state and replaces gate runs from the file.
fn hydrate_evidence_file(
    tx: &rusqlite::Transaction,
    workspace_root: &Path,
    file_path: &Path,
    content: &str,
) -> Result<(), QdevError> {
    let rel_path = relative_path(workspace_root, file_path);
    let (mtime, size) = file_change_stamp(file_path);
    let content_hash = sha256_digest(content.as_bytes());
    upsert_sync_state_row(tx, &rel_path, mtime, size, &content_hash)?;

    // Re-parse replaces gate runs originating from this evidence file
    tx.execute(
        "DELETE FROM gate_runs WHERE evidence_path = ?1;",
        rusqlite::params![rel_path],
    )
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!(
                "Failed to clear gate runs for evidence '{}': {}",
                rel_path, e
            ),
        )
    })?;

    if let Ok(ev_json) = serde_json::from_str::<serde_json::Value>(content) {
        let ev_id = ev_json
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| {
                file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string()
            });

        if !ev_id.is_empty() {
            let story_id = ev_json.get("story_id").and_then(|v| v.as_str());
            let gate_id = ev_json
                .get("gate_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let commit_sha = ev_json
                .get("commit_sha")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ev_status = ev_json.get("status").and_then(|v| v.as_str());
            let exit_code = ev_json
                .get("exit_code")
                .and_then(|v| v.as_i64())
                .map(|i| i as i32);
            let duration_ms = ev_json.get("duration_ms").and_then(|v| v.as_u64());
            let metric_val = ev_json.get("metric_value").and_then(|v| v.as_f64());
            let summary = ev_json.get("summary").and_then(|v| v.as_str());
            let output_hash = ev_json.get("output_hash").and_then(|v| v.as_str());
            let ran_at = ev_json.get("ran_at").and_then(|v| v.as_str());
            let run_by_type = ev_json
                .get("run_by")
                .or_else(|| ev_json.get("created_by"))
                .and_then(|v| v.get("type"))
                .and_then(|v| v.as_str());
            let run_by_id = ev_json
                .get("run_by")
                .or_else(|| ev_json.get("created_by"))
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str());

            tx.execute(
                r#"
INSERT INTO gate_runs (
    id, story_id, gate_id, commit_sha, status, exit_code, duration_ms,
    metric_value, summary, evidence_path, output_hash, run_by_type, run_by_id, ran_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
ON CONFLICT(id) DO UPDATE SET
    story_id = excluded.story_id,
    gate_id = excluded.gate_id,
    commit_sha = excluded.commit_sha,
    status = excluded.status,
    exit_code = excluded.exit_code,
    duration_ms = excluded.duration_ms,
    metric_value = excluded.metric_value,
    summary = excluded.summary,
    evidence_path = excluded.evidence_path,
    output_hash = excluded.output_hash,
    run_by_type = excluded.run_by_type,
    run_by_id = excluded.run_by_id,
    ran_at = excluded.ran_at;
"#,
                rusqlite::params![
                    ev_id,
                    story_id,
                    gate_id,
                    commit_sha,
                    ev_status,
                    exit_code,
                    duration_ms,
                    metric_val,
                    summary,
                    rel_path,
                    output_hash,
                    run_by_type,
                    run_by_id,
                    ran_at
                ],
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to upsert gate run from JSON '{}': {}", ev_id, e),
                )
            })?;
        }
    }
    Ok(())
}

pub(crate) fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) {
    if !dir.exists() || !dir.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.is_symlink() {
                collect_markdown_files(&path, files);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if ext.eq_ignore_ascii_case("md") {
                        files.push(path);
                    }
                }
            }
        }
    }
}

fn collect_files_with_ext(dir: &Path, extension: &str, files: &mut Vec<PathBuf>) {
    if !dir.exists() || !dir.is_dir() {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files_with_ext(&path, extension, files);
            } else if path.is_file() {
                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    if ext.eq_ignore_ascii_case(extension) {
                        files.push(path);
                    }
                }
            }
        }
    }
}

/// Infers an entity's kind from its `kind:` frontmatter field, its file path's directory
/// convention, or (last resort) its identifier's own grammar — defaulting to `Story`. This is
/// the single source of truth for kind inference; hydration and `qdev validate --fix-ids` both
/// call it so a file's inferred kind never diverges between the two.
pub fn determine_entity_kind(
    file_path: &Path,
    id: &str,
    frontmatter: &serde_json::Value,
) -> EntityKind {
    if let Some(kind_str) = frontmatter.get("kind").and_then(|v| v.as_str()) {
        if let Ok(k) = EntityKind::from_str_loose(kind_str) {
            return k;
        }
    }

    let path_str = file_path
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase();
    if path_str.contains("/stories/") {
        return EntityKind::Story;
    } else if path_str.contains("/epics/") {
        return EntityKind::Epic;
    } else if path_str.contains("/requirements/") {
        return EntityKind::Requirement;
    } else if path_str.contains("/adrs/") {
        return EntityKind::Adr;
    } else if path_str.contains("/hazards/") {
        return EntityKind::Hazard;
    } else if path_str.contains("/prd/") {
        return EntityKind::Prd;
    } else if path_str.contains("/sprints/") {
        return EntityKind::Sprint;
    } else if path_str.contains("/releases/") {
        return EntityKind::Release;
    } else if path_str.contains("/dw/") {
        return EntityKind::DeferredWork;
    } else if path_str.contains("/decisions/") {
        return EntityKind::Decision;
    } else if path_str.contains("/scratch/") {
        return EntityKind::Scratchpad;
    } else if path_str.contains("/soup/") {
        return EntityKind::Soup;
    } else if path_str.contains("/evidence/") {
        return EntityKind::Evidence;
    }

    if let Ok(identifier) = id.parse::<Identifier>() {
        match identifier.kind() {
            IdentifierKind::Epic => return EntityKind::Epic,
            IdentifierKind::Story => return EntityKind::Story,
            IdentifierKind::Adr => return EntityKind::Adr,
            IdentifierKind::FunctionalRequirement | IdentifierKind::NonFunctionalRequirement => {
                return EntityKind::Requirement
            }
            IdentifierKind::Hazard => return EntityKind::Hazard,
            IdentifierKind::Prd => return EntityKind::Prd,
            IdentifierKind::DeferredWork => return EntityKind::DeferredWork,
            IdentifierKind::Decision => return EntityKind::Decision,
            _ => {}
        }
    }

    EntityKind::Story
}

fn sha256_digest(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    format!("{:x}", hasher.finalize())
}
