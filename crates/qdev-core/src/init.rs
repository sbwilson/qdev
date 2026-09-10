use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::StorageConfig;
use crate::errors::QdevError;

pub use crate::store::{
    create_schema, drop_all_user_tables, stamp_cache_version, BUSY_TIMEOUT_MS, CACHE_SCHEMA_VERSION,
};

/// The default layout's directories, as a convenience for callers that have no `StorageConfig`.
///
/// `standard_directories(&InitLayout::default())` is the authority — this constant exists only so
/// tests and callers can name the default set without building a layout. A debug assertion keeps
/// the two in agreement, because a hand-maintained copy of a derived list is how the layout
/// drifted from what `init` created in the first place.
pub const STANDARD_DIRECTORIES: &[&str] = &[
    ".qdev/cache",
    ".qdev/gates",
    ".qdev/leases",
    "docs/specs/prd",
    "docs/specs/requirements",
    "docs/specs/epics",
    "docs/specs/stories",
    "docs/specs/adrs",
    "docs/specs/hazards",
    "docs/state/sprints",
    "docs/state/releases",
    "docs/state/dw",
    "docs/state/decisions",
    "docs/state/scratch",
    "docs/state/evidence",
    "docs/state/baselines",
    "docs/state/soup",
];

/// The entity directories under `specs_dir`, and under `state_dir`. Shared with the CLI so the
/// human-facing `init` output names the directories `init` actually creates.
pub const SPEC_SUBDIRECTORIES: &[&str] =
    &["prd", "requirements", "epics", "stories", "adrs", "hazards"];
pub const STATE_SUBDIRECTORIES: &[&str] = &[
    "sprints",
    "releases",
    "dw",
    "decisions",
    "scratch",
    "evidence",
    "baselines",
    "soup",
];

/// The layout `init` builds against. Two `[storage]` values, because they answer two questions:
///
/// * `effective` is the merged configuration every other command uses — the loader's one answer.
///   The cache `init` creates, stamps, inspects and reports comes from here, so what `init`
///   leaves behind is what the next command will open.
/// * `project` is what `qdev.toml` alone says. `.gitignore` is committed and the gate scripts
///   under `<qdev_dir>/gates/` are committed, so those follow the project layout: a developer who
///   relocates their cache in `.qdev.local.toml` must not rewrite the shared ignore file into
///   something only their machine understands, nor move the gate scripts out from under everyone
///   else. Only `cache_dir` can differ between the two — `specs_dir` and `state_dir` are rejected
///   in the local file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InitLayout {
    pub effective: StorageConfig,
    pub project: StorageConfig,
}

impl InitLayout {
    pub fn new(effective: StorageConfig, project: StorageConfig) -> Self {
        Self { effective, project }
    }

    /// The cache directory that will actually be used, relative to the workspace root.
    pub fn cache_dir(&self) -> &str {
        trim_dir(&self.effective.cache_dir)
    }

    /// The cache database `init` creates and stamps, relative to the workspace root.
    pub fn cache_db_relpath(&self) -> String {
        format!("{}/cache.sqlite", self.cache_dir())
    }

    /// `.qdev` in the default layout: the parent of the *project* cache directory, which hosts the
    /// committed gate scripts and the lease files.
    pub fn qdev_dir(&self) -> String {
        parent_dir_of(trim_dir(&self.project.cache_dir))
    }
}

/// Was `init`'s private normalization of a `[storage]` value. The loader now normalizes once, so
/// this only guards against a `StorageConfig` built by hand (tests, `Default`) — it must never be
/// the only place a value is trimmed, or `init` and the other commands drift apart again.
fn trim_dir(dir: &str) -> &str {
    dir.trim().trim_end_matches('/')
}

fn parent_dir_of(dir: &str) -> String {
    Path::new(dir)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| ".qdev".to_string())
}

/// The directories `init` scaffolds, resolved against `layout`. `STANDARD_DIRECTORIES` is the
/// same list for the default layout.
pub fn standard_directories(layout: &InitLayout) -> Vec<String> {
    let specs = trim_dir(&layout.effective.specs_dir);
    let state = trim_dir(&layout.effective.state_dir);
    let qdev_dir = layout.qdev_dir();
    let mut dirs = vec![
        layout.cache_dir().to_string(),
        format!("{}/gates", qdev_dir),
        format!("{}/leases", qdev_dir),
    ];
    for sub in SPEC_SUBDIRECTORIES {
        dirs.push(format!("{}/{}", specs, sub));
    }
    for sub in STATE_SUBDIRECTORIES {
        dirs.push(format!("{}/{}", state, sub));
    }
    dirs
}

/// The `.gitignore` entries `init` ensures, resolved against `layout`.
///
/// The file is committed, so it covers the *project* cache directory — meaningful to everyone —
/// and, when a developer has relocated the cache in `.qdev.local.toml`, that directory too, so
/// the cache every command actually uses is never untracked only by luck. That asymmetry is the
/// price of a locally overridable `cache_dir`, and it is deliberate.
pub fn gitignore_entries(layout: &InitLayout) -> Vec<String> {
    let project_cache = trim_dir(&layout.project.cache_dir);
    let effective_cache = layout.cache_dir();
    let mut entries = vec![format!("{}/", project_cache)];
    if effective_cache != project_cache {
        entries.push(format!("{}/", effective_cache));
    }
    entries.push(format!("{}/leases/", layout.qdev_dir()));
    entries.push(crate::config::LOCAL_CONFIG_FILENAME.to_string());
    entries
}

/// Options passed into the core `init` function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOptions {
    pub root: PathBuf,
    pub name: String,
    pub developer: String,
    pub teams: Vec<String>,
    pub allow_migration: bool,
    /// The resolved `[storage]` layout. `init` does not read configuration itself: the caller has
    /// already loaded it through the loader every other command uses, so what `init` scaffolds,
    /// stamps, gitignores and reports is the layout the next command will use.
    pub layout: InitLayout,
}

/// Status of the SQLite cache schema in an existing workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheStatus {
    NotInitialized,
    NeedsMigration {
        current_version: u32,
        target_version: u32,
    },
    UpToDate {
        version: u32,
    },
}

/// Result returned by the core `init` routine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitResult {
    pub root: String,
    pub created_files: Vec<String>,
    pub created_directories: Vec<String>,
    pub gitignore_updated: bool,
    pub cache_schema_version: u32,
    pub cache_migrated: bool,
    pub already_initialized: bool,
    /// The layout this run resolved and built — reported so a wrapper, and the human-facing
    /// output, name the paths `init` actually created rather than the default ones.
    pub storage: StorageConfig,
    /// The directory hosting `gates/` and `leases/`: `.qdev` in the default layout.
    pub qdev_dir: String,
}

/// Inspect the existing SQLite cache database if present to determine its schema state.
///
/// `storage` is the resolved configuration, so the cache inspected here is the cache every other
/// command opens: a workspace whose effective cache is stamped by a newer binary is refused by
/// `init` too, naming that file rather than an abandoned one.
pub fn check_cache_status(root: &Path, storage: &StorageConfig) -> Result<CacheStatus, QdevError> {
    let cache_db_path = root.join(trim_dir(&storage.cache_dir)).join("cache.sqlite");
    if !cache_db_path.exists() {
        return Ok(CacheStatus::NotInitialized);
    }

    let conn = rusqlite::Connection::open(&cache_db_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!(
                "Failed to open existing cache database at {}: {}",
                cache_db_path.display(),
                e
            ),
        )
    })?;

    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to set busy_timeout=5000 on cache database: {}", e),
            )
        })?;

    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to read user_version from cache database: {}", e),
            )
        })?;

    if user_version < CACHE_SCHEMA_VERSION {
        Ok(CacheStatus::NeedsMigration {
            current_version: user_version,
            target_version: CACHE_SCHEMA_VERSION,
        })
    } else if user_version > CACHE_SCHEMA_VERSION {
        // Same conflict `ensure_cache` raises on boot, from the same constructor, so the two
        // paths cannot drift apart again.
        Err(crate::store::newer_cache_conflict(
            user_version,
            CACHE_SCHEMA_VERSION,
        ))
    } else {
        Ok(CacheStatus::UpToDate {
            version: user_version,
        })
    }
}

/// Initialize a qdev workspace at the specified root directory.
pub fn init(options: &InitOptions) -> Result<InitResult, QdevError> {
    let name = options.name.trim();
    if name.is_empty() {
        return Err(QdevError::usage_error("Project name cannot be empty")
            .with_details(serde_json::json!({ "field": "name" })));
    }

    let developer = options.developer.trim();
    if developer.is_empty() {
        return Err(QdevError::usage_error("Developer ID cannot be empty")
            .with_details(serde_json::json!({ "field": "developer" })));
    }

    let mut deduped_teams = Vec::new();
    for team in &options.teams {
        for t in team.split(',') {
            let trimmed = t.trim();
            if !trimmed.is_empty() && !deduped_teams.contains(&trimmed.to_string()) {
                deduped_teams.push(trimmed.to_string());
            }
        }
    }
    if deduped_teams.is_empty() {
        return Err(
            QdevError::usage_error("At least one team must be specified")
                .with_details(serde_json::json!({ "field": "teams" })),
        );
    }

    // Validate cache status upfront before modifying filesystem
    match check_cache_status(&options.root, &options.layout.effective)? {
        CacheStatus::NeedsMigration {
            current_version,
            target_version,
        } if !options.allow_migration => {
            return Err(QdevError::policy_refusal(
                "needs_confirmation",
                format!(
                    "Cache schema migration from v{} to v{} requires confirmation or --yes",
                    current_version, target_version
                ),
            )
            .with_details(serde_json::json!({
                "current_version": current_version,
                "target_version": target_version,
                "flag": "--yes",
            })));
        }
        _ => {}
    }

    let root = &options.root;
    if !root.exists() {
        fs::create_dir_all(root).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to create root directory {}: {}", root.display(), e),
            )
        })?;
    }

    // One resolver: the layout came from the loader, so a workspace already configured for a
    // non-default layout has its directories, cache and .gitignore built against the layout the
    // next command will use.
    let layout = &options.layout;
    let cache_db_relpath = layout.cache_db_relpath();

    let qdev_toml_path = root.join(crate::config::PROJECT_CONFIG_FILENAME);
    let local_toml_path = root.join(crate::config::LOCAL_CONFIG_FILENAME);
    let cache_db_path = root.join(&cache_db_relpath);

    let qdev_toml_existed = qdev_toml_path.exists();
    let local_toml_existed = local_toml_path.exists();
    let cache_db_existed = cache_db_path.exists();

    let mut created_directories = Vec::new();
    let mut created_files = Vec::new();

    // 1. Create standard directory structure
    for dir in standard_directories(layout) {
        let dir_path = root.join(&dir);
        if !dir_path.exists() {
            fs::create_dir_all(&dir_path).map_err(|e| {
                QdevError::infrastructure_failure(
                    "io_error",
                    format!("Failed to create directory {}: {}", dir_path.display(), e),
                )
            })?;
            created_directories.push(dir);
        }
    }

    // 2. Scaffold qdev.toml if not already present
    if !qdev_toml_existed {
        let mut content = format!("[project]\nname = {:?}\n\n[teams]\n", name);
        for team in &deduped_teams {
            content.push_str(&format!("{:?} = [{:?}]\n", team, developer));
        }

        fs::write(&qdev_toml_path, &content).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to write {}: {}", qdev_toml_path.display(), e),
            )
        })?;
        created_files.push("qdev.toml".to_string());
    }

    // 3. Scaffold .qdev.local.toml if not already present
    if !local_toml_existed {
        let teams_json = serde_json::to_string(&deduped_teams).unwrap_or_else(|_| "[]".to_string());

        let content = format!(
            "[identity]\ndeveloper_id = {:?}\nteams = {}\n",
            developer, teams_json
        );

        fs::write(&local_toml_path, &content).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to write {}: {}", local_toml_path.display(), e),
            )
        })?;
        created_files.push(".qdev.local.toml".to_string());
    }

    // 4. Update .gitignore at root
    let gitignore_path = root.join(".gitignore");
    let gitignore_existed = gitignore_path.exists();
    let gitignore_updated = update_gitignore(&gitignore_path, layout)?;
    if !gitignore_existed && gitignore_path.exists() {
        created_files.push(".gitignore".to_string());
    }

    // 5. Initialize or migrate SQLite cache schema
    let (cache_schema_version, cache_migrated) =
        initialize_cache(&cache_db_path, cache_db_existed, options.allow_migration)?;
    if !cache_db_existed && cache_db_path.exists() {
        created_files.push(cache_db_relpath.clone());
    }

    let already_initialized =
        qdev_toml_existed && local_toml_existed && cache_db_existed && !cache_migrated;

    Ok(InitResult {
        root: root.to_string_lossy().to_string(),
        created_files,
        created_directories,
        gitignore_updated,
        cache_schema_version,
        cache_migrated,
        already_initialized,
        storage: layout.effective.clone(),
        qdev_dir: layout.qdev_dir(),
    })
}

fn update_gitignore(gitignore_path: &Path, layout: &InitLayout) -> Result<bool, QdevError> {
    let required_entries = gitignore_entries(layout);
    if !gitignore_path.exists() {
        let mut content = String::new();
        for entry in &required_entries {
            content.push_str(entry);
            content.push('\n');
        }
        fs::write(gitignore_path, content).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to write .gitignore: {}", e),
            )
        })?;
        return Ok(true);
    }

    let existing = fs::read_to_string(gitignore_path).map_err(|e| {
        QdevError::infrastructure_failure("io_error", format!("Failed to read .gitignore: {}", e))
    })?;

    let existing_lines: Vec<&str> = existing.lines().map(|l| l.trim()).collect();
    let mut missing_entries = Vec::new();

    fn normalize_pattern(s: &str) -> &str {
        s.trim_start_matches('/').trim_end_matches('/')
    }

    for required in &required_entries {
        let req_norm = normalize_pattern(required);
        let found = existing_lines
            .iter()
            .any(|line| normalize_pattern(line) == req_norm);
        if !found {
            missing_entries.push(required.clone());
        }
    }

    if missing_entries.is_empty() {
        return Ok(false);
    }

    let mut new_content = existing.clone();
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    for entry in missing_entries {
        new_content.push_str(&entry);
        new_content.push('\n');
    }

    fs::write(gitignore_path, new_content).map_err(|e| {
        QdevError::infrastructure_failure("io_error", format!("Failed to update .gitignore: {}", e))
    })?;

    Ok(true)
}

fn initialize_cache(
    cache_db_path: &Path,
    cache_db_existed: bool,
    allow_migration: bool,
) -> Result<(u32, bool), QdevError> {
    // Ensure parent directory exists
    if let Some(parent) = cache_db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create cache directory {}: {}",
                    parent.display(),
                    e
                ),
            )
        })?;
    }

    let conn = rusqlite::Connection::open(cache_db_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "sqlite_error",
            format!("Failed to open cache SQLite database: {}", e),
        )
    })?;

    // Per AD-4: WAL mode and 5000ms busy timeout
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

    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to read user_version from cache database: {}", e),
            )
        })?;

    if !cache_db_existed {
        // Fresh database creation wrapped in transaction
        conn.execute_batch("BEGIN IMMEDIATE;").map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to begin transaction: {}", e),
            )
        })?;

        let res = (|| -> Result<(), QdevError> {
            create_schema(&conn)?;
            // Stamped through the shared writer, not a hand-rolled pragma: one stamping path
            // means a fresh cache is inspected `Valid` on the very next command instead of
            // being torn down and rebuilt.
            stamp_cache_version(&conn)?;
            Ok(())
        })();

        if let Err(e) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(e);
        }

        conn.execute_batch("COMMIT;").map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to commit transaction: {}", e),
            )
        })?;
        Ok((CACHE_SCHEMA_VERSION, false))
    } else if user_version < CACHE_SCHEMA_VERSION {
        // Older cache schema migration
        if !allow_migration {
            return Err(QdevError::policy_refusal(
                "needs_confirmation",
                format!(
                    "Cache schema migration from v{} to v{} requires confirmation or --yes",
                    user_version, CACHE_SCHEMA_VERSION
                ),
            )
            .with_details(serde_json::json!({
                "current_version": user_version,
                "target_version": CACHE_SCHEMA_VERSION,
                "flag": "--yes",
            })));
        }

        // Migration wrapped in transaction
        conn.execute_batch("BEGIN IMMEDIATE;").map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to begin transaction: {}", e),
            )
        })?;

        let res = (|| -> Result<(), QdevError> {
            drop_all_user_tables(&conn)?;
            create_schema(&conn)?;
            // Stamped through the shared writer, not a hand-rolled pragma: one stamping path
            // means a fresh cache is inspected `Valid` on the very next command instead of
            // being torn down and rebuilt.
            stamp_cache_version(&conn)?;
            Ok(())
        })();

        if let Err(e) = res {
            let _ = conn.execute_batch("ROLLBACK;");
            return Err(e);
        }

        conn.execute_batch("COMMIT;").map_err(|e| {
            QdevError::infrastructure_failure(
                "sqlite_error",
                format!("Failed to commit transaction: {}", e),
            )
        })?;
        Ok((CACHE_SCHEMA_VERSION, true))
    } else if user_version > CACHE_SCHEMA_VERSION {
        // Same conflict `ensure_cache` raises on boot, from the same constructor, so the two
        // paths cannot drift apart again.
        Err(crate::store::newer_cache_conflict(
            user_version,
            CACHE_SCHEMA_VERSION,
        ))
    } else {
        // Schema is current
        Ok((user_version, false))
    }
}
