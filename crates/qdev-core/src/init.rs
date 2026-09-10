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
    /// The resolved `[storage]` layout. `init` does not read configuration itself: the caller has
    /// already loaded it through the loader every other command uses, so what `init` scaffolds,
    /// stamps, gitignores and reports is the layout the next command will use.
    pub layout: InitLayout,
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
    /// Files re-parsed by a cache migration, `None` when nothing was migrated.
    ///
    /// A migration drops every table and repopulates from the files under the effective
    /// `[storage]` layout, so this is the number that says whether it worked: a migration
    /// reporting `Some(0)` on a workspace full of stories means the rebuild found nothing — a
    /// wrong `[storage]` layout, say — which would otherwise be an exit 0 with no signal at all.
    ///
    /// It counts *files*, which is `SweepSummary::parsed` verbatim and what `sync --rebuild`
    /// reports: entity Markdown, plus the scratch, evidence and config files a rebuild also
    /// reads. Deriving an entity count here would be a second, differently-defined number for
    /// the same pass.
    pub cache_files_rehydrated: Option<usize>,
}

/// Refuse a workspace whose cache was stamped by a newer binary than this one supports.
///
/// The one question `init` asks before it touches the filesystem, and the only one whose answer
/// changes what `init` does: a newer cache is an exit-5 refusal, and every other state — older,
/// structurally incomplete, absent — is repaired by `initialize_cache` without asking. This used
/// to be a `check_cache_status` returning a three-variant `CacheStatus`, but with the migration
/// unconditional the `Ok` variants described a decision nobody made; a reader could not tell the
/// pre-flight refusal from a report.
///
/// Detection runs through `inspect_cache_schema`, the boot path's own inspector, so `init` and
/// boot classify the same file identically — including a cache stamped current but missing a
/// table, which a `PRAGMA user_version` comparison called healthy while boot rebuilt it.
///
/// `storage` is the resolved configuration, so the cache inspected here is the cache every other
/// command opens: a workspace whose effective cache is stamped by a newer binary is refused by
/// `init` too, naming that file rather than an abandoned one.
pub fn verify_cache_compatible(root: &Path, storage: &StorageConfig) -> Result<(), QdevError> {
    let cache_db_path = root.join(trim_dir(&storage.cache_dir)).join("cache.sqlite");
    if !cache_db_path.exists() {
        return Ok(());
    }

    match crate::store::inspect_cache_schema(&cache_db_path)? {
        crate::store::CacheSchemaStatus::NewerThanSupported { found, supported } => {
            // Same conflict `ensure_cache` raises on boot, from the same constructor, so the two
            // paths cannot drift apart again.
            Err(crate::store::newer_cache_conflict(found, supported))
        }
        // Valid, or a mismatch `initialize_cache` repairs.
        _ => Ok(()),
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

    // Inspect the cache upfront, before anything on the filesystem is touched, so a cache
    // stamped by a newer binary is the exit-5 refusal `verify_cache_compatible` raises and
    // nothing is scaffolded first. An *older* or structurally incomplete cache is not refused
    // here or anywhere: `init` rebuilds it, as every other command's boot already does — the
    // cache is a rebuildable index (AD-3/FR-101), so confirming a rebuild protects nothing, and
    // a refusal the next command ignores is worse than no refusal. See
    // spec-init-cache-migration.md.
    verify_cache_compatible(&options.root, &options.layout.effective)?;

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
    let (cache_schema_version, cache_migrated, migration_summary) =
        initialize_cache(&cache_db_path, cache_db_existed, root, &layout.effective)?;
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
        cache_files_rehydrated: migration_summary.map(|s| s.parsed),
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
    workspace_root: &Path,
    storage: &crate::config::StorageConfig,
) -> Result<(u32, bool, Option<crate::store::SweepSummary>), QdevError> {
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
        Ok((CACHE_SCHEMA_VERSION, false, None))
    } else {
        // An existing cache is classified by `inspect_cache_schema`, the boot path's own
        // inspector, rather than by a `PRAGMA user_version` comparison of its own. The pragma
        // answers a narrower question than boot asks: a cache stamped current but missing a
        // table or a column is `Mismatch` to boot, which rebuilds it, and was "up to date" to
        // `init`, which reported `already_initialized` and repaired nothing. One inspector means
        // `init` and the next command cannot disagree about whether the file is healthy.
        drop(conn);
        match crate::store::inspect_cache_schema(cache_db_path)? {
            crate::store::CacheSchemaStatus::Valid => Ok((CACHE_SCHEMA_VERSION, false, None)),
            crate::store::CacheSchemaStatus::Mismatch => {
                // Older or structurally incomplete — rebuilt, unconditionally, needing no
                // confirmation from anyone.
                //
                // Delegated to `reset_and_rebuild`, the boot path's own migration, rather than
                // reimplemented: a hand-rolled drop/create/stamp here was a second copy kept in
                // step by comment, and it had already drifted — it stamped *before*
                // repopulating, so a failed rebuild left a cache stamped current and empty,
                // where the shared implementation stamps only after the rows are back. One
                // implementation cannot drift from itself.
                //
                // `reset_and_rebuild` deliberately takes no advisory lock (`ensure_cache` holds
                // it when it calls this), so the lock is acquired here — the same rule
                // `qdev sync --rebuild` follows. `init` previously dropped and recreated the
                // cache holding no lock at all.
                let lock_path = cache_db_path
                    .parent()
                    .unwrap_or(cache_db_path)
                    .join("write.lock");
                let _guard = crate::write::acquire_write_lock(
                    &lock_path,
                    std::time::Duration::from_millis(BUSY_TIMEOUT_MS),
                )?;
                let store = crate::store::SqliteStore::open(cache_db_path)?;
                let summary = store.reset_and_rebuild(workspace_root, storage)?;
                Ok((CACHE_SCHEMA_VERSION, true, Some(summary)))
            }
            // Reachable only by losing a race with a newer binary: `init`'s own pre-flight
            // (`verify_cache_compatible`) read this same file before anything was scaffolded and
            // refused then. Raised from the shared constructor rather than asserted away, so the
            // window closes with a refusal instead of a rebuild over a newer cache.
            crate::store::CacheSchemaStatus::NewerThanSupported { found, supported } => {
                Err(crate::store::newer_cache_conflict(found, supported))
            }
        }
    }
}
