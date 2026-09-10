use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

use qdev_core::store::{ensure_cache, SqliteStore, ALL_TABLE_NAMES};
use qdev_core::{
    gitignore_entries, init, standard_directories, verify_cache_compatible, ExitCode, InitLayout,
    InitOptions, StorageConfig, CACHE_SCHEMA_VERSION, STANDARD_DIRECTORIES,
};

#[test]
fn test_init_fresh_workspace_scaffolding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let options = InitOptions {
        root: root.clone(),
        name: "Qubric".to_string(),
        developer: "simon".to_string(),
        teams: vec!["core-platform".to_string()],
        layout: InitLayout::default(),
    };

    let result = init(&options).expect("init should succeed on fresh directory");

    assert_eq!(result.cache_schema_version, CACHE_SCHEMA_VERSION);
    assert!(!result.cache_migrated);
    assert!(result.gitignore_updated);
    assert!(result.created_files.contains(&"qdev.toml".to_string()));
    assert!(result
        .created_files
        .contains(&".qdev.local.toml".to_string()));
    assert!(result.created_files.contains(&".gitignore".to_string()));
    assert!(result
        .created_files
        .contains(&".qdev/cache/cache.sqlite".to_string()));

    // Verify scaffolded config is strictly valid per load_config
    let loaded_config =
        qdev_core::load_config(&root).expect("Scaffolded config must load and pass validation");
    assert_eq!(loaded_config.config.project.name, "Qubric");
    assert_eq!(loaded_config.config.identity.developer_id, "simon");
    assert_eq!(
        loaded_config.config.identity.teams,
        vec!["core-platform".to_string()]
    );

    // Verify all standard directories exist
    for &dir in STANDARD_DIRECTORIES {
        let dir_path = root.join(dir);
        assert!(
            dir_path.is_dir(),
            "Directory {} was not created",
            dir_path.display()
        );
    }

    // Verify qdev.toml contents
    let qdev_toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert!(qdev_toml.contains("name = \"Qubric\""));
    assert!(qdev_toml.contains("\"core-platform\" = [\"simon\"]"));

    // Verify .qdev.local.toml contents
    let local_toml = fs::read_to_string(root.join(".qdev.local.toml")).unwrap();
    assert!(local_toml.contains("developer_id = \"simon\""));
    assert!(local_toml.contains("core-platform"));

    // Verify .gitignore contents
    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".qdev/cache/"));
    assert!(gitignore.contains(".qdev/leases/"));
    assert!(gitignore.contains(".qdev.local.toml"));

    // Verify SQLite cache database
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    assert!(cache_db_path.is_file());

    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);

    // Verify WAL mode
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(journal_mode.to_lowercase(), "wal");

    // Verify standard tables exist
    let required_tables = [
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
    ];
    for table in required_tables {
        let count: u32 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1;",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Table {} must exist in cache.sqlite", table);
    }
}

#[test]
fn test_gitignore_deduplication() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    // Pre-create .gitignore with some existing entries including leading-slash root-anchored rules
    let initial_gitignore = "target/\n/.qdev/cache/\n*.log\n";
    fs::write(root.join(".gitignore"), initial_gitignore).unwrap();

    let options = InitOptions {
        root: root.clone(),
        name: "Demo".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };

    let result = init(&options).unwrap();
    assert!(result.gitignore_updated);

    let updated = fs::read_to_string(root.join(".gitignore")).unwrap();
    // Must contain target/, *.log, /.qdev/cache/, .qdev/leases/, .qdev.local.toml
    assert!(updated.contains("target/"));
    assert!(updated.contains("*.log"));
    assert!(updated.contains(".qdev/leases/"));
    assert!(updated.contains(".qdev.local.toml"));

    // Must not duplicate .qdev/cache/ despite leading slash in existing entry
    let occurrences = updated.matches(".qdev/cache/").count();
    assert_eq!(
        occurrences, 1,
        "Expected exactly 1 occurrence of .qdev/cache/, found {}",
        occurrences
    );

    // Running init again should not modify .gitignore
    let result_again = init(&options).unwrap();
    assert!(!result_again.gitignore_updated);
}

#[test]
fn test_never_overwrite_existing_configs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let existing_qdev = "[project]\nname = \"ExistingName\"\n[teams]\ncustom = [\"bob\"]\n";
    let existing_local = "[identity]\ndeveloper_id = \"existing_dev\"\n";

    fs::write(root.join("qdev.toml"), existing_qdev).unwrap();
    fs::write(root.join(".qdev.local.toml"), existing_local).unwrap();

    let options = InitOptions {
        root: root.clone(),
        name: "NewName".to_string(),
        developer: "new_dev".to_string(),
        teams: vec!["new_team".to_string()],
        layout: InitLayout::default(),
    };

    let result = init(&options).unwrap();

    // Configs should not be in created_files
    assert!(!result.created_files.contains(&"qdev.toml".to_string()));
    assert!(!result
        .created_files
        .contains(&".qdev.local.toml".to_string()));

    // Content should remain untouched
    let qdev_content = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert_eq!(qdev_content, existing_qdev);

    let local_content = fs::read_to_string(root.join(".qdev.local.toml")).unwrap();
    assert_eq!(local_content, existing_local);
}

/// An older cache is migrated with nobody's permission. There is no confirmation to give and no
/// flag to pass: the cache is a rebuildable index, every other command's boot already migrates it
/// silently, and a refusal the next command ignores is worse than no refusal.
#[test]
fn test_older_cache_migrates_without_any_confirmation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    // Create .qdev/cache/cache.sqlite with user_version = 0
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_file = cache_dir.join("cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_file).unwrap();
        conn.execute_batch("PRAGMA user_version = 0;").unwrap();
    }

    // The pre-flight is a refusal, not a report: an older cache passes it, and `init` rebuilds.
    verify_cache_compatible(&root, &StorageConfig::default())
        .expect("an older cache is compatible — it is rebuilt, not refused");

    let options = InitOptions {
        root: root.clone(),
        name: "MigrateTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };

    let result = init(&options).expect("an older cache migrates with no confirmation");
    assert!(result.cache_migrated);
    assert_eq!(result.cache_schema_version, CACHE_SCHEMA_VERSION);
    assert!(root.join("qdev.toml").exists());
    verify_cache_compatible(&root, &StorageConfig::default()).unwrap();
}

#[test]
fn test_cache_migration_succeeds_and_drops_old_tables() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    // Create .qdev/cache/cache.sqlite with user_version = 0 and an outdated legacy table
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_file = cache_dir.join("cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_file).unwrap();
        conn.execute_batch(
            "PRAGMA user_version = 0; CREATE TABLE legacy_outdated (id INTEGER PRIMARY KEY);",
        )
        .unwrap();
    }

    let options = InitOptions {
        root: root.clone(),
        name: "MigrateTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };

    let result = init(&options).expect("Migration should succeed");
    assert_eq!(result.cache_schema_version, CACHE_SCHEMA_VERSION);
    assert!(result.cache_migrated);

    let conn = rusqlite::Connection::open(&cache_file).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);

    // Outdated table must be dropped
    let count: u32 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='legacy_outdated';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0, "Outdated tables must be dropped on migration");

    // After migration the cache is the one boot calls healthy, structure included.
    verify_cache_compatible(&root, &StorageConfig::default()).unwrap();
    assert_eq!(
        qdev_core::inspect_cache_schema(&cache_file).unwrap(),
        qdev_core::CacheSchemaStatus::Valid
    );
}

#[test]
fn test_verify_cache_compatible_future_version_conflict() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_file = cache_dir.join("cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_file).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION + 1
        ))
        .unwrap();
    }

    let err = verify_cache_compatible(&root, &StorageConfig::default()).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "schema_version_mismatch");

    let options = InitOptions {
        root: root.clone(),
        name: "FutureTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };
    let init_err = init(&options).unwrap_err();
    assert_eq!(init_err.exit_code(), ExitCode::Conflict);
}

#[test]
fn test_init_validation_non_empty_fields() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    // Empty name
    let opt_no_name = InitOptions {
        root: root.clone(),
        name: "   ".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };
    let err = init(&opt_no_name).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);

    // Empty developer
    let opt_no_dev = InitOptions {
        root: root.clone(),
        name: "Demo".to_string(),
        developer: "   ".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };
    let err = init(&opt_no_dev).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);

    // Empty teams
    let opt_no_teams = InitOptions {
        root: root.clone(),
        name: "Demo".to_string(),
        developer: "alice".to_string(),
        teams: vec!["  ".to_string()],
        layout: InitLayout::default(),
    };
    let err = init(&opt_no_teams).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_idempotent_rerun_schema_v2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let options = InitOptions {
        root: root.clone(),
        name: "IdempotentTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };

    // First run: initializes
    let res1 = init(&options).unwrap();
    assert_eq!(res1.cache_schema_version, CACHE_SCHEMA_VERSION);
    assert!(!res1.cache_migrated);
    assert!(res1.created_files.contains(&"qdev.toml".to_string()));
    assert!(res1.created_files.contains(&".qdev.local.toml".to_string()));

    // Second run: idempotent re-run
    let res2 = init(&options).unwrap();
    assert_eq!(res2.cache_schema_version, CACHE_SCHEMA_VERSION);
    assert!(!res2.cache_migrated);
    assert!(res2.created_files.is_empty());
    assert!(!res2.gitignore_updated);
    assert!(res2.already_initialized);
}

#[test]
fn test_multiple_teams() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let options = InitOptions {
        root: root.clone(),
        name: "MultiTeam".to_string(),
        developer: "alice".to_string(),
        teams: vec![
            "frontend".to_string(),
            "backend".to_string(),
            "frontend".to_string(),
        ], // duplicated team
        layout: InitLayout::default(),
    };

    let result = init(&options).unwrap();
    assert!(result.created_files.contains(&"qdev.toml".to_string()));

    let qdev_toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert!(qdev_toml.contains("\"frontend\" = [\"alice\"]"));
    assert!(qdev_toml.contains("\"backend\" = [\"alice\"]"));
    // Deduplication check: frontend should only appear once as a key in qdev.toml
    assert_eq!(qdev_toml.matches("\"frontend\" =").count(), 1);

    let local_toml = fs::read_to_string(root.join(".qdev.local.toml")).unwrap();
    assert!(local_toml.contains("frontend"));
    assert!(local_toml.contains("backend"));

    // Validates via load_config
    let cfg = qdev_core::load_config(&root).unwrap();
    assert_eq!(cfg.config.identity.teams.len(), 2);
}

#[test]
fn test_teams_comma_separated_splitting() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let options = InitOptions {
        root: root.clone(),
        name: "CommaTeam".to_string(),
        developer: "bob".to_string(),
        teams: vec!["frontend,backend".to_string(), "ops, backend".to_string()],
        layout: InitLayout::default(),
    };

    let result = init(&options).unwrap();
    assert!(result.created_files.contains(&"qdev.toml".to_string()));

    let qdev_toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert!(qdev_toml.contains("\"frontend\" = [\"bob\"]"));
    assert!(qdev_toml.contains("\"backend\" = [\"bob\"]"));
    assert!(qdev_toml.contains("\"ops\" = [\"bob\"]"));
    // Deduplicated: backend only appears once
    assert_eq!(qdev_toml.matches("\"backend\" =").count(), 1);

    let cfg = qdev_core::load_config(&root).unwrap();
    assert_eq!(cfg.config.identity.teams.len(), 3);
}

#[test]
fn test_cache_migration_drops_views_triggers_and_escaped_tables() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_file = cache_dir.join("cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_file).unwrap();
        conn.execute_batch(
            r#"
            PRAGMA user_version = 0;
            CREATE TABLE "legacy""table" (id INTEGER PRIMARY KEY, val TEXT);
            CREATE VIEW legacy_view AS SELECT id, val FROM "legacy""table";
            CREATE TRIGGER legacy_trigger AFTER INSERT ON "legacy""table" BEGIN
                SELECT 1;
            END;
            "#,
        )
        .unwrap();
    }

    let options = InitOptions {
        root: root.clone(),
        name: "CleanMigrate".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };

    let result =
        init(&options).expect("Migration should drop triggers, views, and escaped tables cleanly");
    assert_eq!(result.cache_schema_version, CACHE_SCHEMA_VERSION);
    assert!(result.cache_migrated);

    let conn = rusqlite::Connection::open(&cache_file).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);

    // Verify all legacy objects are gone
    let legacy_tables: u32 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name = 'legacy\"table';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_tables, 0);

    let legacy_views: u32 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='view';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_views, 0);

    let legacy_triggers: u32 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='trigger';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_triggers, 0);
}

/// `[storage]` may relocate the cache locally, and `.gitignore` is committed — so the ignore file
/// covers the project cache directory *and* the locally configured one. Without the second entry
/// the cache every command actually opens is untracked only by luck; without the first, the
/// committed file stops being meaningful to anyone whose machine is configured differently.
#[test]
fn test_gitignore_covers_both_the_project_and_the_local_cache_directory() {
    let layout = InitLayout::new(
        StorageConfig {
            cache_dir: "local/cache".to_string(),
            ..StorageConfig::default()
        },
        StorageConfig::default(),
    );

    let entries = gitignore_entries(&layout);
    assert!(
        entries.contains(&".qdev/cache/".to_string()),
        "the project cache must stay covered for everyone else, got: {entries:?}"
    );
    assert!(
        entries.contains(&"local/cache/".to_string()),
        "the locally relocated cache must be covered too, got: {entries:?}"
    );
    assert!(entries.contains(&".qdev/leases/".to_string()));
    assert!(entries.contains(&".qdev.local.toml".to_string()));

    // The committed gate scripts follow the project layout, not a developer's cache override.
    let dirs = standard_directories(&layout);
    assert!(dirs.contains(&"local/cache".to_string()));
    assert!(dirs.contains(&".qdev/gates".to_string()));
    assert!(
        !dirs.contains(&".qdev/cache".to_string()),
        "only one cache directory is scaffolded, got: {dirs:?}"
    );
}

/// Everything `init` reports it created is a path it actually created: the cache path in
/// `created_files` is derived from the resolved layout rather than hardcoded.
#[test]
fn test_init_reports_the_cache_it_actually_created() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let layout = InitLayout::new(
        StorageConfig {
            specs_dir: "planning/specs".to_string(),
            state_dir: "planning/state".to_string(),
            cache_dir: "local/cache".to_string(),
        },
        StorageConfig {
            specs_dir: "planning/specs".to_string(),
            state_dir: "planning/state".to_string(),
            cache_dir: ".qdev/cache".to_string(),
        },
    );

    let options = InitOptions {
        root: root.clone(),
        name: "Configured".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: layout.clone(),
    };

    let result = init(&options).unwrap();

    assert!(result
        .created_files
        .contains(&"local/cache/cache.sqlite".to_string()));
    for path in result
        .created_files
        .iter()
        .chain(result.created_directories.iter())
    {
        assert!(
            root.join(path).exists(),
            "reported path {path} does not exist on disk"
        );
    }
    assert!(!root.join(".qdev/cache").exists());
    assert_eq!(result.storage, layout.effective);
    assert_eq!(result.qdev_dir, ".qdev");
}

/// The cache `init` inspects is the cache every other command opens, so a newer stamp on the
/// *effective* cache is refused rather than reported as an up-to-date workspace.
#[test]
fn test_verify_cache_compatible_inspects_the_configured_cache() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let storage = StorageConfig {
        cache_dir: "local/cache".to_string(),
        ..StorageConfig::default()
    };

    fs::create_dir_all(root.join("local/cache")).unwrap();
    {
        let conn = rusqlite::Connection::open(root.join("local/cache/cache.sqlite")).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION + 1
        ))
        .unwrap();
    }

    // The default-layout cache does not exist at all, so the private reader used to report
    // `NotInitialized` here while every other command refused the workspace.
    let err = verify_cache_compatible(&root, &storage).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "schema_version_mismatch");
    assert!(err.message().contains("local/cache/cache.sqlite") || err.message().contains("newer"));

    verify_cache_compatible(&root, &StorageConfig::default())
        .expect("the default-layout cache does not exist, so there is nothing to refuse");
}

/// `STANDARD_DIRECTORIES` is a hand-maintained copy of what `standard_directories` builds for the
/// default layout, and a hand-maintained copy of a derived list is how `init`'s reporting drifted
/// from what it creates. This keeps the two in agreement.
#[test]
fn test_standard_directories_matches_the_default_layout() {
    let derived = qdev_core::standard_directories(&qdev_core::InitLayout::default());
    let mut derived_sorted = derived.clone();
    derived_sorted.sort();
    let mut listed: Vec<String> = qdev_core::STANDARD_DIRECTORIES
        .iter()
        .map(|s| s.to_string())
        .collect();
    listed.sort();
    assert_eq!(
        derived_sorted, listed,
        "STANDARD_DIRECTORIES must list exactly what standard_directories builds for the \
         default layout"
    );
}

/// Dumps every table in `ALL_TABLE_NAMES` as ordered stringified rows, for table-by-table
/// comparison of two caches.
fn dump_all_tables(db_path: &std::path::Path) -> BTreeMap<String, Vec<Vec<String>>> {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let mut dump = BTreeMap::new();

    for &table in ALL_TABLE_NAMES {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM \"{}\";", table))
            .unwrap_or_else(|e| panic!("table {} must exist after migration: {}", table, e));
        let col_count = stmt.column_count();
        let mut rows: Vec<Vec<String>> = stmt
            .query_map([], |row| {
                let mut vals = Vec::new();
                for i in 0..col_count {
                    let val: rusqlite::types::Value = row.get(i)?;
                    vals.push(match val {
                        rusqlite::types::Value::Null => "NULL".to_string(),
                        rusqlite::types::Value::Integer(i) => i.to_string(),
                        rusqlite::types::Value::Real(f) => format!("{:.4}", f),
                        rusqlite::types::Value::Text(t) => t,
                        rusqlite::types::Value::Blob(b) => format!("{:?}", b),
                    });
                }
                Ok(vals)
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // Sorted here rather than in SQL: `relations` has no single ordering column, and a
        // rebuild is only required to produce the same *set* of rows.
        rows.sort();
        dump.insert(table.to_string(), rows);
    }

    dump
}

/// `init` migrates a cache that has rows in it.
///
/// The bug this pins: `init` wraps the drop-and-rebuild in `BEGIN IMMEDIATE`, and
/// `drop_all_user_tables` used to guard itself with `PRAGMA foreign_keys = OFF`, which SQLite
/// documents as a no-op inside a transaction. So `DROP TABLE entities` failed against any child
/// row (`stories.id REFERENCES entities(id)`) and every real workspace was unmigratable — while
/// the boot path, which calls the same helper with no transaction open, worked. Every migration
/// fixture in this file until now built an *empty* cache, which is exactly why a green suite
/// never caught it: an empty cache has no child row to violate anything.
///
/// It also pins the invariant the two paths must share: after either migration the cache holds
/// what a full rebuild of the same tree produces.
#[test]
fn test_populated_older_cache_migrates_and_equals_a_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let storage = StorageConfig::default();

    let options = InitOptions {
        root: root.clone(),
        name: "PopulatedMigrate".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };
    init(&options).expect("first init should succeed");

    // Two stories, one depending on the other, so the cache holds `entities` rows *and* child
    // rows in `stories`, `constraints` and `relations` — and no dangling edge, so neither path
    // records a finding.
    let story_dir = root.join("docs/specs/stories");
    fs::write(
        story_dir.join("E1S1.md"),
        r#"---
id: E1S1
title: "Depends On Another"
status: ready
version: 1
appetite: small
safety_class: ClassB
target_modules: ["foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: "No swift"
relations:
  depends_on: ["E1S2"]
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
"#,
    )
    .unwrap();
    fs::write(
        story_dir.join("E1S2.md"),
        r#"---
id: E1S2
title: "Depended Upon"
status: done
version: 1
appetite: tiny
safety_class: ClassA
target_modules: ["foundation"]
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
"#,
    )
    .unwrap();

    // Populate the cache the way a real workspace does — a boot-time sweep.
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    drop(ensure_cache(&root, &storage).expect("sweep should hydrate the two stories"));

    let populated = dump_all_tables(&cache_db_path);
    assert_eq!(
        populated["entities"].len(),
        2,
        "fixture must hold entity rows, or it pins nothing"
    );
    assert_eq!(
        populated["stories"].len(),
        2,
        "fixture must hold child rows in `stories`, or the foreign key is never exercised"
    );
    assert!(!populated["constraints"].is_empty());
    assert!(!populated["relations"].is_empty());

    // Stamp it back to an older version, leaving every row in place.
    {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION - 1
        ))
        .unwrap();
    }

    let result = init(&options).expect("a populated older cache must migrate, not fail on an FK");
    assert!(result.cache_migrated);
    assert_eq!(result.cache_schema_version, CACHE_SCHEMA_VERSION);
    let migrated = dump_all_tables(&cache_db_path);

    // The reference answer: a full rebuild of the same tree, the boot path's own repair.
    let store = SqliteStore::open(&cache_db_path).unwrap();
    store
        .reset_and_rebuild(&root, &storage)
        .expect("rebuild should succeed");
    drop(store);
    let rebuilt = dump_all_tables(&cache_db_path);

    assert!(
        migrated["findings"].is_empty() && rebuilt["findings"].is_empty(),
        "fixture is meant to be finding-free, so the comparison below is not comparing timestamps"
    );
    for (table, rebuilt_rows) in &rebuilt {
        // `sync_meta` holds `last_synced_at`, a wall-clock stamp of when the pass ran rather
        // than anything derived from the files, so it cannot be equal across two passes.
        if table == "sync_meta" {
            continue;
        }
        assert_eq!(
            &migrated[table], rebuilt_rows,
            "table '{}' differs between `init`'s migration and a full rebuild of the same tree",
            table
        );
    }
    assert!(
        !migrated["entities"].is_empty(),
        "the migrated cache must be repopulated from the Markdown files, not left empty"
    );
}

/// A cache stamped current but *structurally* incomplete is repaired, not reported healthy.
///
/// `init` used to compare `PRAGMA user_version` and nothing else, so a cache missing a table was
/// "up to date" to `init` — which reported `already_initialized` and repaired nothing — while the
/// very next command's boot classified the same file `Mismatch` and dropped and rebuilt it. Both
/// now ask `inspect_cache_schema`.
#[test]
fn test_structurally_incomplete_cache_is_repaired_rather_than_called_up_to_date() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    let storage = StorageConfig::default();

    let options = InitOptions {
        root: root.clone(),
        name: "Structural".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };
    init(&options).expect("first init should succeed");

    fs::write(
        root.join("docs/specs/stories/E1S1.md"),
        r#"---
id: E1S1
title: "Structurally Incomplete Cache"
status: ready
version: 1
appetite: small
safety_class: ClassA
target_modules: ["foundation"]
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
"#,
    )
    .unwrap();

    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    drop(ensure_cache(&root, &storage).expect("sweep should hydrate the story"));

    // Drop one table, leaving the stamp at the current version: version-only detection cannot
    // see this, `inspect_cache_schema` can.
    {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch("DROP TABLE findings;").unwrap();
        let user_version: u32 = conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            user_version, CACHE_SCHEMA_VERSION,
            "the stamp must stay current, or this fixture is just the older-cache case again"
        );
    }
    assert_eq!(
        qdev_core::inspect_cache_schema(&cache_db_path).unwrap(),
        qdev_core::CacheSchemaStatus::Mismatch
    );

    let result = init(&options).expect("a structurally incomplete cache must be repaired");
    assert!(
        result.cache_migrated,
        "init must report the rebuild it performed"
    );
    assert!(
        !result.already_initialized,
        "a cache no command can use is not an initialized workspace"
    );
    assert_eq!(
        qdev_core::inspect_cache_schema(&cache_db_path).unwrap(),
        qdev_core::CacheSchemaStatus::Valid
    );

    let repaired = dump_all_tables(&cache_db_path);
    assert_eq!(
        repaired["entities"].len(),
        1,
        "the repaired cache must be repopulated from the Markdown files"
    );
    assert_eq!(
        result.cache_files_rehydrated,
        Some(2),
        "the story and `qdev.toml` are both re-parsed by a rebuild"
    );
}

/// The migration repopulates through the *effective* layout, so a workspace with a configured
/// `[storage]` comes back with its rows — not with an empty cache and exit 0.
#[test]
fn test_migration_under_a_configured_layout_repopulates_from_the_configured_tree() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let storage = StorageConfig {
        specs_dir: "planning/specs".to_string(),
        state_dir: "planning/state".to_string(),
        cache_dir: "local/cache".to_string(),
    };
    let layout = InitLayout::new(storage.clone(), storage.clone());

    let options = InitOptions {
        root: root.clone(),
        name: "ConfiguredMigrate".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        layout: layout.clone(),
    };
    init(&options).expect("first init should succeed");

    fs::write(
        root.join("planning/specs/stories/E1S1.md"),
        r#"---
id: E1S1
title: "Configured Layout Story"
status: ready
version: 1
appetite: small
safety_class: ClassA
target_modules: ["foundation"]
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
"#,
    )
    .unwrap();

    let cache_db_path = root.join("local/cache/cache.sqlite");
    drop(ensure_cache(&root, &storage).expect("sweep should hydrate the configured tree"));
    assert_eq!(dump_all_tables(&cache_db_path)["entities"].len(), 1);

    {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION - 1
        ))
        .unwrap();
    }

    let result = init(&options).expect("a configured-layout cache must migrate");
    assert!(result.cache_migrated);
    assert_eq!(
        result.cache_files_rehydrated,
        Some(2),
        "the migration must repopulate from the configured specs_dir, not the default one \
         (the story, plus `qdev.toml`)"
    );
    assert_eq!(
        dump_all_tables(&cache_db_path)["entities"].len(),
        1,
        "the configured cache must hold its rows again after the migration"
    );
    assert!(
        !root.join(".qdev/cache/cache.sqlite").exists(),
        "the migration must not create a default-layout cache alongside the configured one"
    );
}
