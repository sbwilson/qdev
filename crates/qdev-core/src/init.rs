use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::QdevError;

pub use crate::store::{
    create_schema_v2, drop_all_user_tables, BUSY_TIMEOUT_MS, CACHE_SCHEMA_VERSION,
};

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

pub const GITIGNORE_ENTRIES: &[&str] = &[".qdev/cache/", ".qdev/leases/", ".qdev.local.toml"];

/// Options passed into the core `init` function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitOptions {
    pub root: PathBuf,
    pub name: String,
    pub developer: String,
    pub teams: Vec<String>,
    pub allow_migration: bool,
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
}

/// Inspect the existing SQLite cache database if present to determine its schema state.
pub fn check_cache_status(root: &Path) -> Result<CacheStatus, QdevError> {
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
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
        Err(QdevError::conflict(
            "schema_version_mismatch",
            format!(
                "Cache database schema version v{} is newer than supported version v{}",
                user_version, CACHE_SCHEMA_VERSION
            ),
        )
        .with_details(serde_json::json!({
            "current_version": user_version,
            "supported_version": CACHE_SCHEMA_VERSION,
        })))
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
    match check_cache_status(&options.root)? {
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

    let qdev_toml_path = root.join("qdev.toml");
    let local_toml_path = root.join(".qdev.local.toml");
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");

    let qdev_toml_existed = qdev_toml_path.exists();
    let local_toml_existed = local_toml_path.exists();
    let cache_db_existed = cache_db_path.exists();

    let mut created_directories = Vec::new();
    let mut created_files = Vec::new();

    // 1. Create standard directory structure
    for &dir in STANDARD_DIRECTORIES {
        let dir_path = root.join(dir);
        if !dir_path.exists() {
            fs::create_dir_all(&dir_path).map_err(|e| {
                QdevError::infrastructure_failure(
                    "io_error",
                    format!("Failed to create directory {}: {}", dir_path.display(), e),
                )
            })?;
            created_directories.push(dir.to_string());
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
    let gitignore_updated = update_gitignore(&gitignore_path)?;
    if !gitignore_existed && gitignore_path.exists() {
        created_files.push(".gitignore".to_string());
    }

    // 5. Initialize or migrate SQLite cache schema
    let (cache_schema_version, cache_migrated) =
        initialize_cache(&cache_db_path, cache_db_existed, options.allow_migration)?;
    if !cache_db_existed && cache_db_path.exists() {
        created_files.push(".qdev/cache/cache.sqlite".to_string());
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
    })
}

fn update_gitignore(gitignore_path: &Path) -> Result<bool, QdevError> {
    if !gitignore_path.exists() {
        let mut content = String::new();
        for &entry in GITIGNORE_ENTRIES {
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

    for &required in GITIGNORE_ENTRIES {
        let req_norm = normalize_pattern(required);
        let found = existing_lines
            .iter()
            .any(|line| normalize_pattern(line) == req_norm);
        if !found {
            missing_entries.push(required);
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
        new_content.push_str(entry);
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
            create_schema_v2(&conn)?;
            conn.execute_batch(&format!("PRAGMA user_version = {};", CACHE_SCHEMA_VERSION))
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to set user_version: {}", e),
                    )
                })?;
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
            create_schema_v2(&conn)?;
            conn.execute_batch(&format!("PRAGMA user_version = {};", CACHE_SCHEMA_VERSION))
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "sqlite_error",
                        format!("Failed to set user_version: {}", e),
                    )
                })?;
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
        Err(QdevError::conflict(
            "schema_version_mismatch",
            format!(
                "Cache database schema version v{} is newer than supported version v{}",
                user_version, CACHE_SCHEMA_VERSION
            ),
        )
        .with_details(serde_json::json!({
            "current_version": user_version,
            "supported_version": CACHE_SCHEMA_VERSION,
        })))
    } else {
        // Schema is current
        Ok((user_version, false))
    }
}
