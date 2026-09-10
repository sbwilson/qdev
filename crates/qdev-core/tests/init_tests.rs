use std::fs;
use tempfile::TempDir;

use qdev_core::{
    check_cache_status, gitignore_entries, init, standard_directories, CacheStatus, ExitCode,
    InitLayout, InitOptions, StorageConfig, CACHE_SCHEMA_VERSION, STANDARD_DIRECTORIES,
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
        allow_migration: false,
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
        allow_migration: false,
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
        allow_migration: false,
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

#[test]
fn test_cache_migration_refused_without_confirmation() {
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

    // check_cache_status should report NeedsMigration
    let status = check_cache_status(&root, &StorageConfig::default()).unwrap();
    assert_eq!(
        status,
        CacheStatus::NeedsMigration {
            current_version: 0,
            target_version: CACHE_SCHEMA_VERSION
        }
    );

    // Attempt init without allow_migration -> fails with PolicyRefusal (ExitCode 3)
    let options = InitOptions {
        root: root.clone(),
        name: "MigrateTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        allow_migration: false,
        layout: InitLayout::default(),
    };

    let err = init(&options).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_confirmation");
    assert!(err.message().contains("migration"));
    assert_eq!(err.details().unwrap()["flag"], "--yes");
    // Verify filesystem was not modified before refusal
    assert!(!root.join("qdev.toml").exists());
}

#[test]
fn test_cache_migration_succeeds_with_confirmation_and_drops_old_tables() {
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
        allow_migration: true,
        layout: InitLayout::default(),
    };

    let result = init(&options).expect("Migration should succeed with allow_migration=true");
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

    // After migration, status is UpToDate
    let status = check_cache_status(&root, &StorageConfig::default()).unwrap();
    assert_eq!(
        status,
        CacheStatus::UpToDate {
            version: CACHE_SCHEMA_VERSION
        }
    );
}

#[test]
fn test_check_cache_status_future_version_conflict() {
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

    let err = check_cache_status(&root, &StorageConfig::default()).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "schema_version_mismatch");

    let options = InitOptions {
        root: root.clone(),
        name: "FutureTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        allow_migration: true,
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
        allow_migration: false,
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
        allow_migration: false,
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
        allow_migration: false,
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
        allow_migration: false,
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
        allow_migration: false,
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
        allow_migration: false,
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
        allow_migration: true,
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
        allow_migration: false,
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
fn test_check_cache_status_inspects_the_configured_cache() {
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
    let err = check_cache_status(&root, &storage).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "schema_version_mismatch");
    assert!(err.message().contains("local/cache/cache.sqlite") || err.message().contains("newer"));

    assert_eq!(
        check_cache_status(&root, &StorageConfig::default()).unwrap(),
        CacheStatus::NotInitialized
    );
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
