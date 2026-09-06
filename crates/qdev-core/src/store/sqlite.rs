use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::id::{Identifier, IdentifierKind};
use crate::schema::{extract_frontmatter, EntityKind};
use crate::store::{
    ConstraintRecord, DecisionRecord, DeferredWorkRecord, DirtyEntityRecord, EntityFilter,
    EntityRecord, GateRecord, GateRunRecord, RelationRecord, ScratchpadRecord, SoupRecord,
    SprintAssignmentRecord, SprintRecord, Store, StoryRecord, SyncStateRecord,
};
use crate::write::Author;

pub const CACHE_SCHEMA_VERSION: u32 = 1;
pub const CACHE_USER_VERSION: u32 = 1;
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
];

pub const SCHEMA_V1_DDL: &str = r#"
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
    updated_at TEXT
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
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheSchemaStatus {
    Valid,
    Mismatch,
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

        create_schema_v1(&conn)?;

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

    /// Drops all existing tables and triggers, recreates schema v1, rebuilds all cache rows from
    /// Markdown entity files in the workspace, and sets user_version = 1 and schema_version = 1.
    pub fn reset_and_rebuild(
        &self,
        workspace_root: &Path,
        storage: &StorageConfig,
    ) -> Result<(), QdevError> {
        self.with_conn_mut(|conn| {
            drop_all_user_tables(conn)?;
            create_schema_v1(conn)?;
            Ok(())
        })?;

        self.rebuild_from_workspace(workspace_root, storage)?;

        // Ensure user_version and schema_version pragmas are 1 after rebuild
        self.with_conn(|conn| {
            conn.execute_batch(&format!(
                "PRAGMA user_version = {};\nPRAGMA schema_version = {};\n",
                CACHE_USER_VERSION, CACHE_SCHEMA_VERSION
            ))
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to record cache pragmas: {}", e),
                )
            })?;
            Ok(())
        })?;

        Ok(())
    }

    /// Scans the workspace specification and state directories and rebuilds all cache tables.
    pub fn rebuild_from_workspace(
        &self,
        workspace_root: &Path,
        storage: &StorageConfig,
    ) -> Result<(), QdevError> {
        // Collect all file paths to parse in deterministic sorted order
        let mut entity_files = Vec::new();

        let specs_dir = workspace_root.join(&storage.specs_dir);
        let state_dir = workspace_root.join(&storage.state_dir);

        collect_markdown_files(&specs_dir, &mut entity_files);
        collect_markdown_files(&state_dir, &mut entity_files);

        // Sort by path for deterministic rebuild order
        entity_files.sort();

        // Also collect scratchpad (.jsonl) and evidence (.json) files
        let scratch_dir = state_dir.join("scratch");
        let mut scratch_files = Vec::new();
        collect_files_with_ext(&scratch_dir, "jsonl", &mut scratch_files);
        scratch_files.sort();

        let evidence_dir = state_dir.join("evidence");
        let mut evidence_files = Vec::new();
        collect_files_with_ext(&evidence_dir, "json", &mut evidence_files);
        evidence_files.sort();

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
"#,
            )
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to clear tables for rebuild: {}", e),
                )
            })?;

            // 1. Process gates from qdev.toml if present
            let config_path = workspace_root.join("qdev.toml");
            if config_path.exists() {
                if let Ok(content) = fs::read_to_string(&config_path) {
                    if let Ok(parsed_toml) = toml::from_str::<toml::Value>(&content) {
                        if let Some(toml::Value::Array(gates)) = parsed_toml.get("gates") {
                            for g in gates {
                                if let Some(gate_table) = g.as_table() {
                                    if let Some(id) = gate_table.get("id").and_then(|v| v.as_str()) {
                                        let cmd = gate_table
                                            .get("command")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or_default();
                                        let kind = gate_table
                                            .get("kind")
                                            .and_then(|v| v.as_str());
                                        let timeout_ms = gate_table
                                            .get("timeout_ms")
                                            .and_then(|v| v.as_integer())
                                            .filter(|&i| i >= 0)
                                            .map(|i| i as u64);
                                        let output_adapter = gate_table
                                            .get("output_adapter")
                                            .and_then(|v| v.as_str());
                                        let on_trans = gate_table
                                            .get("on_transition")
                                            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()));
                                        let deps = gate_table
                                            .get("depends_on")
                                            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| v.to_string()));
                                        let metric = gate_table
                                            .get("metric")
                                            .and_then(|v| v.as_str());
                                        let direction = gate_table
                                            .get("direction")
                                            .and_then(|v| v.as_str());

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
                                                format!("Failed to rebuild gate '{}': {}", id, e),
                                            )
                                        })?;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 2. Process all Markdown entity files
            for file_path in &entity_files {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let rel_path = file_path
                    .strip_prefix(workspace_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .replace('\\', "/");

                let metadata = fs::metadata(file_path).ok();
                let mtime = metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let size = metadata.map(|m| m.len()).unwrap_or(0);
                let content_hash = sha256_digest(content.as_bytes());

                // Upsert sync_state
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
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to upsert sync_state for '{}': {}", rel_path, e),
                    )
                })?;

                let frontmatter = match extract_frontmatter(&content) {
                    Ok(fm) => fm,
                    Err(_) => continue,
                };

                let id = match frontmatter.get("id").and_then(|v| v.as_str()) {
                    Some(id) if !id.trim().is_empty() => id.to_string(),
                    _ => continue,
                };

                let kind = determine_entity_kind(file_path, &id, &frontmatter);
                let title = frontmatter.get("title").and_then(|v| v.as_str()).map(str::to_string);
                let status = frontmatter.get("status").and_then(|v| v.as_str()).map(str::to_string);
                let owners = frontmatter.get("owners").map(|v| v.to_string());
                let version = frontmatter.get("version").and_then(|v| v.as_u64()).unwrap_or(1);

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
                    .unwrap_or_else(|| iso8601_from_timestamp(mtime));

                // Upsert entities table
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
    created_by_type = excluded.created_by_type,
    created_by_id = excluded.created_by_id,
    updated_by_type = excluded.updated_by_type,
    updated_by_id = excluded.updated_by_id,
    updated_at = excluded.updated_at;
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
                    ],
                )
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to rebuild entity '{}': {}", id, e),
                    )
                })?;

                // 2a. Kind-specific: Story
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
                            format!("Failed to rebuild story details '{}': {}", id, e),
                        )
                    })?;
                }

                // 2b. Constraints (from frontmatter of any entity)
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
                                    format!("Failed to rebuild constraint '{}': {}", comp_id, e),
                                )
                            })?;
                        }
                    }
                }

                // 2c. Relations (from frontmatter of any entity)
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
                                                "Failed to rebuild relation '{}' -> '{}': {}",
                                                id, target_id, e
                                            ),
                                        )
                                    })?;
                                }
                            }
                        }
                    }
                }

                // 2d. Kind-specific: Sprint
                if kind == EntityKind::Sprint {
                    let sprint_num: i64 = if let Some(stripped) = id.strip_prefix("sprint-") {
                        stripped.parse().unwrap_or(0)
                    } else {
                        id.parse().unwrap_or(0)
                    };

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
                            format!("Failed to rebuild sprint '{}': {}", id, e),
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
                                    let carried = map
                                        .get("carried_from")
                                        .and_then(|v| {
                                            v.as_i64().or_else(|| {
                                                v.as_str().and_then(|s| {
                                                    s.strip_prefix("sprint-")
                                                        .unwrap_or(s)
                                                        .parse::<i64>()
                                                        .ok()
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
                                            "Failed to rebuild sprint assignment for '{}': {}",
                                            story_id, e
                                        ),
                                    )
                                })?;
                            }
                        }
                    }
                }

                // 2e. Kind-specific: DeferredWork
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
                            format!("Failed to rebuild deferred work '{}': {}", id, e),
                        )
                    })?;
                }

                // 2f. Kind-specific: Decision
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
                            format!("Failed to rebuild decision '{}': {}", id, e),
                        )
                    })?;
                }

                // 2g. Kind-specific: Soup
                if kind == EntityKind::Soup {
                    let name = frontmatter.get("name").and_then(|v| v.as_str());
                    let ver = frontmatter
                        .get("dependency_version")
                        .or_else(|| frontmatter.get("version"))
                        .and_then(|v| v.as_str());
                    let license = frontmatter.get("license").and_then(|v| v.as_str());
                    let cve_status = frontmatter.get("cve_status").and_then(|v| v.as_str());
                    let intro_story = frontmatter.get("introduced_by_story").and_then(|v| v.as_str());
                    let eval_rel = frontmatter.get("evaluated_for_release").and_then(|v| v.as_str());

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
                            format!("Failed to rebuild SOUP dependency '{}': {}", id, e),
                        )
                    })?;
                }

                // 2h. Kind-specific: Evidence
                if kind == EntityKind::Evidence {
                    let story_id = frontmatter.get("story_id").and_then(|v| v.as_str());
                    let gate_id = frontmatter.get("gate_id").and_then(|v| v.as_str()).unwrap_or("");
                    let commit_sha = frontmatter.get("commit_sha").and_then(|v| v.as_str()).unwrap_or("");
                    let ev_status = frontmatter.get("status").and_then(|v| v.as_str());
                    let exit_code = frontmatter.get("exit_code").and_then(|v| v.as_i64()).map(|i| i as i32);
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
                            format!("Failed to rebuild gate run '{}': {}", id, e),
                        )
                    })?;
                }
            }

            // 3. Process scratchpad (.jsonl) files
            for file_path in &scratch_files {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                let story_id = file_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default();

                let rel_path = file_path
                    .strip_prefix(workspace_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .replace('\\', "/");

                let metadata = fs::metadata(file_path).ok();
                let mtime = metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let size = metadata.map(|m| m.len()).unwrap_or(0);
                let content_hash = sha256_digest(content.as_bytes());

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
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to record sync_state for scratchpad '{}': {}", rel_path, e),
                    )
                })?;

                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(entry_json) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        let seq = entry_json.get("seq").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                        let at = entry_json
                            .get("at")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let (author_type, author_id) = if let Some(author_obj) = entry_json.get("author").and_then(|v| v.as_object()) {
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
                            rusqlite::params![
                                story_id,
                                seq,
                                at,
                                author_type,
                                author_id,
                                entry_kind,
                                text
                            ],
                        )
                        .map_err(|e| {
                            QdevError::infrastructure_failure(
                                "sqlite_error",
                                format!(
                                    "Failed to rebuild scratchpad entry for '{}:{}': {}",
                                    story_id, seq, e
                                ),
                            )
                        })?;
                    }
                }
            }

            // 4. Process evidence JSON files
            for file_path in &evidence_files {
                let content = match fs::read_to_string(file_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let rel_path = file_path
                    .strip_prefix(workspace_root)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .replace('\\', "/");

                let metadata = fs::metadata(file_path).ok();
                let mtime = metadata
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let size = metadata.map(|m| m.len()).unwrap_or(0);
                let content_hash = sha256_digest(content.as_bytes());

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
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to record sync_state for evidence '{}': {}", rel_path, e),
                    )
                })?;

                if let Ok(ev_json) = serde_json::from_str::<serde_json::Value>(&content) {
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
                        let gate_id = ev_json.get("gate_id").and_then(|v| v.as_str()).unwrap_or("");
                        let commit_sha = ev_json.get("commit_sha").and_then(|v| v.as_str()).unwrap_or("");
                        let ev_status = ev_json.get("status").and_then(|v| v.as_str());
                        let exit_code = ev_json.get("exit_code").and_then(|v| v.as_i64()).map(|i| i as i32);
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
                                format!("Failed to rebuild gate run from JSON '{}': {}", ev_id, e),
                            )
                        })?;
                    }
                }
            }

            tx.commit().map_err(|e| {
                QdevError::infrastructure_failure(
                    "sqlite_error",
                    format!("Failed to commit rebuild transaction: {}", e),
                )
            })?;

            Ok(())
        })?;

        Ok(())
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
    s.epic_id, s.seq, s.appetite, s.safety_class, s.target_modules
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
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn list_entities(&self, filter: &EntityFilter) -> Result<Vec<EntityRecord>, QdevError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    r#"
SELECT
    e.id, e.kind, e.title, e.status, e.owners, e.source_path, e.content_hash, e.version,
    e.created_by_type, e.created_by_id, e.updated_by_type, e.updated_by_id, e.updated_at,
    s.epic_id, s.seq, s.appetite, s.safety_class, s.target_modules
FROM entities e
LEFT JOIN stories s ON e.id = s.id
WHERE (?1 IS NULL OR e.kind = ?1)
  AND (?2 IS NULL OR e.status = ?2)
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

            let rows = stmt
                .query_map(rusqlite::params![kind_filter, status_filter], |row| {
                    let kind_str: String = row.get(1)?;
                    let kind = EntityKind::from_str_loose(&kind_str).unwrap_or(EntityKind::Story);
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
                    })
                })
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
                "DELETE FROM relations WHERE source_id = ?1 OR target_id = ?1;",
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
}

/// Creates all 14 schema tables if not present and sets PRAGMA user_version = 1 and PRAGMA schema_version = 1.
pub fn create_schema_v1(conn: &rusqlite::Connection) -> Result<(), QdevError> {
    conn.execute_batch(SCHEMA_V1_DDL).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to create SQLite cache schema v1: {}", e),
        )
    })?;

    conn.execute_batch(&format!(
        "PRAGMA user_version = {};\nPRAGMA schema_version = {};\n",
        CACHE_USER_VERSION, CACHE_SCHEMA_VERSION
    ))
    .map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!(
                "Failed to set user_version and schema_version pragmas: {}",
                e
            ),
        )
    })?;

    Ok(())
}

/// Drops all triggers, views, and tables in the database (disabling foreign keys during drop).
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

    conn.execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to disable foreign keys: {}", e),
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

    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to re-enable foreign keys: {}", e),
            )
        })?;

    Ok(())
}

/// Inspects an existing database path and returns whether its schema is valid or mismatched.
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

    let schema_version: u32 = conn
        .query_row("PRAGMA schema_version;", [], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to read schema_version: {}", e),
            )
        })?;

    if user_version != CACHE_USER_VERSION || schema_version != CACHE_SCHEMA_VERSION {
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

    Ok(CacheSchemaStatus::Valid)
}

/// Boot-time verification and initialization of the SQLite cache database.
/// Ensures `.qdev/cache/cache.sqlite` exists with all 14 tables, WAL mode, busy_timeout=5000,
/// and schema_version = user_version = 1. Automatically rebuilds from files on missing cache or version mismatch.
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
        SqliteStore::open(&cache_db_path)
    }
}

fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) {
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

fn determine_entity_kind(
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

fn iso8601_from_timestamp(secs: i64) -> String {
    let s = (secs.rem_euclid(60)) as u64;
    let m = ((secs / 60).rem_euclid(60)) as u64;
    let h = ((secs / 3600).rem_euclid(24)) as u64;
    let mut days = secs / 86400;
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
