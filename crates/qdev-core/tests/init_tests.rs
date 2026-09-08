use std::fs;
use tempfile::TempDir;

use qdev_core::{
    check_cache_status, init, CacheStatus, ExitCode, InitOptions, STANDARD_DIRECTORIES,
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
    };

    let result = init(&options).expect("init should succeed on fresh directory");

    assert_eq!(result.cache_schema_version, 2);
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
    assert_eq!(user_version, 2);

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
    let status = check_cache_status(&root).unwrap();
    assert_eq!(
        status,
        CacheStatus::NeedsMigration {
            current_version: 0,
            target_version: 2
        }
    );

    // Attempt init without allow_migration -> fails with PolicyRefusal (ExitCode 3)
    let options = InitOptions {
        root: root.clone(),
        name: "MigrateTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        allow_migration: false,
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
    };

    let result = init(&options).expect("Migration should succeed with allow_migration=true");
    assert_eq!(result.cache_schema_version, 2);
    assert!(result.cache_migrated);

    let conn = rusqlite::Connection::open(&cache_file).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 2);

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
    let status = check_cache_status(&root).unwrap();
    assert_eq!(status, CacheStatus::UpToDate { version: 2 });
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
        conn.execute_batch("PRAGMA user_version = 3;").unwrap();
    }

    let err = check_cache_status(&root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "schema_version_mismatch");

    let options = InitOptions {
        root: root.clone(),
        name: "FutureTest".to_string(),
        developer: "alice".to_string(),
        teams: vec!["core".to_string()],
        allow_migration: true,
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
    };

    // First run: initializes
    let res1 = init(&options).unwrap();
    assert_eq!(res1.cache_schema_version, 2);
    assert!(!res1.cache_migrated);
    assert!(res1.created_files.contains(&"qdev.toml".to_string()));
    assert!(res1.created_files.contains(&".qdev.local.toml".to_string()));

    // Second run: idempotent re-run
    let res2 = init(&options).unwrap();
    assert_eq!(res2.cache_schema_version, 2);
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
    };

    let result =
        init(&options).expect("Migration should drop triggers, views, and escaped tables cleanly");
    assert_eq!(result.cache_schema_version, 2);
    assert!(result.cache_migrated);

    let conn = rusqlite::Connection::open(&cache_file).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 2);

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
