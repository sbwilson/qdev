use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

use qdev_core::rusqlite;
use qdev_core::store::{ALL_TABLE_NAMES, CACHE_SCHEMA_VERSION};

fn dump_all_tables(db_path: &Path) -> BTreeMap<String, Vec<Vec<String>>> {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let mut table_dump = BTreeMap::new();

    for &table in ALL_TABLE_NAMES {
        let query = format!("SELECT * FROM \"{}\" ORDER BY 1 ASC;", table);
        let mut stmt = match conn.prepare(&query) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let col_count = stmt.column_count();
        let rows = stmt
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
            .unwrap();

        let mut row_list = Vec::new();
        for r in rows {
            row_list.push(r.unwrap());
        }
        table_dump.insert(table.to_string(), row_list);
    }

    table_dump
}

fn init_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "CacheTestProject",
            "--developer",
            "alice",
            "--team",
            "core-platform",
        ])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_cli_boot_with_empty_cache() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. Initialize workspace
    init_workspace(root);

    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    assert!(cache_db_path.exists());

    // 2. Remove DB file, leaving .qdev/cache/ existing but empty
    fs::remove_file(&cache_db_path).unwrap();
    assert!(!cache_db_path.exists());
    assert!(root.join(".qdev/cache").is_dir());

    // 3. Run read-only command (qdev status)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["status"])
        .assert()
        .success()
        .code(0);

    // 4. Verify cache.sqlite was recreated
    assert!(
        cache_db_path.exists(),
        "cache.sqlite must be recreated on boot"
    );

    // 5. Verify pragmas and all 14 tables exist
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        user_version, CACHE_SCHEMA_VERSION,
        "user_version must match CACHE_SCHEMA_VERSION"
    );

    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        journal_mode.to_lowercase(),
        "wal",
        "journal_mode must be WAL"
    );

    for &table in ALL_TABLE_NAMES {
        let count: u32 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1;",
                rusqlite::params![table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Table '{}' must exist", table);
    }
}

#[test]
fn test_cli_boot_with_missing_cache_dir() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. Initialize workspace
    init_workspace(root);

    let cache_dir = root.join(".qdev/cache");
    assert!(cache_dir.is_dir());

    // 2. Remove the entire .qdev/cache directory
    fs::remove_dir_all(&cache_dir).unwrap();
    assert!(!cache_dir.exists());

    // 3. Run command (qdev config show --json)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("\"schema_version\": \"1\""));

    // 4. Verify directory and DB were created
    let cache_db_path = cache_dir.join("cache.sqlite");
    assert!(
        cache_dir.is_dir(),
        ".qdev/cache directory must be recreated"
    );
    assert!(cache_db_path.exists(), "cache.sqlite must be recreated");

    // 5. Verify user_version and all 15 tables
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);

    for &table in ALL_TABLE_NAMES {
        let count: u32 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1;",
                rusqlite::params![table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Table '{}' must exist", table);
    }
}

#[test]
fn test_cli_boot_with_schema_mismatch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. Initialize workspace
    init_workspace(root);

    // 2. Create a story markdown file in docs/specs/stories/E12S1.md
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("E12S1.md"),
        r#"---
id: E12S1
kind: story
title: Schema Mismatch Rebuild Story
status: draft
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
created_at: 2026-09-07T00:00:00Z
updated_at: 2026-09-07T00:00:00Z
---

## Acceptance Criteria
- Auto-rebuilds on schema mismatch.
"#,
    )
    .unwrap();

    // 3. Corrupt the cache: set user_version = 0 and create a legacy table
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch("PRAGMA user_version = 0; CREATE TABLE old_legacy_table (val TEXT);")
            .unwrap();
    }

    // 4. Run CLI command (qdev status)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["status"])
        .assert()
        .success()
        .code(0);

    // 5. Verify schema was dropped and recreated
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        user_version, CACHE_SCHEMA_VERSION,
        "user_version must be reset to CACHE_SCHEMA_VERSION"
    );

    let has_legacy: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='old_legacy_table';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!has_legacy, "Legacy table must have been dropped");

    for &table in ALL_TABLE_NAMES {
        let count: u32 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1;",
                rusqlite::params![table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "Table '{}' must exist after rebuild", table);
    }

    // 6. Verify entity E12S1 was rebuilt into entities table
    let title: String = conn
        .query_row("SELECT title FROM entities WHERE id = 'E12S1';", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title, "Schema Mismatch Rebuild Story");
}

#[test]
fn test_cli_table_by_table_identical_cache_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. Initialize workspace
    init_workspace(root);

    // 2. Create story entity via `qdev create story E12`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "create",
            "story",
            "E12",
            "--title",
            "CLI Created Story",
            "--appetite",
            "small",
            "--safety-class",
            "ClassB",
        ])
        .assert()
        .success()
        .code(0);

    // Also create additional entities in docs to test multi-table rebuilding
    let sprint_dir = root.join("docs/state/sprints");
    fs::create_dir_all(&sprint_dir).unwrap();
    fs::write(
        sprint_dir.join("sprint-1.md"),
        r#"---
id: sprint-1
kind: sprint
title: Sprint 1
status: active
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
owners: ["alice"]
started_at: 2026-09-07T00:00:00Z
assignments:
  - story_id: E12S1
    assigned_at: 2026-09-07T00:00:00Z
---
## Sprint Goals
"#,
    )
    .unwrap();

    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(
        dw_dir.join("DW-1.md"),
        r#"---
id: DW-1
kind: deferred_work
title: Postponed optimization
origin_story_id: E12S1
target_module: core
status: open
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
safety_risk: acceptable_with_mitigation
rationale: Postponed optimization
gate: ratchet
resolution: Planned for sprint 2
---
## Deferred Work
"#,
    )
    .unwrap();

    let soup_dir = root.join("docs/state/soup");
    fs::create_dir_all(&soup_dir).unwrap();
    fs::write(
        soup_dir.join("rusqlite.md"),
        r#"---
id: rusqlite@0.31.0
kind: soup
name: rusqlite
status: accepted
version: 1
dependency_version: "0.31.0"
license: MIT
cve_status: clean
introduced_by_story: E12S1
evaluated_for_release: 0.1.0
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
## SOUP Details
"#,
    )
    .unwrap();

    // Trigger initial cache build by removing cache.sqlite so ensure_cache rebuilds it from all files
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    if cache_db_path.exists() {
        fs::remove_file(&cache_db_path).unwrap();
    }

    // Run `qdev status` to build initial cache from files
    let mut cmd_status = Command::cargo_bin("qdev").unwrap();
    cmd_status
        .current_dir(root)
        .args(["status"])
        .assert()
        .success()
        .code(0);

    // 3. Record all 14 table contents from cache.sqlite
    let initial_dump = dump_all_tables(&cache_db_path);

    // Verify non-empty data in populated tables
    assert!(
        !initial_dump["entities"].is_empty(),
        "entities must not be empty"
    );
    assert!(
        !initial_dump["stories"].is_empty(),
        "stories must not be empty"
    );
    assert!(
        !initial_dump["sprints"].is_empty(),
        "sprints must not be empty"
    );
    assert!(
        !initial_dump["sprint_assignments"].is_empty(),
        "sprint_assignments must not be empty"
    );
    assert!(
        !initial_dump["deferred_work"].is_empty(),
        "deferred_work must not be empty"
    );
    assert!(
        !initial_dump["soup_dependencies"].is_empty(),
        "soup_dependencies must not be empty"
    );
    assert!(
        !initial_dump["sync_state"].is_empty(),
        "sync_state must not be empty"
    );

    // 4. Delete the entire .qdev/cache/ directory
    let cache_dir = root.join(".qdev/cache");
    fs::remove_dir_all(&cache_dir).unwrap();
    assert!(!cache_dir.exists());

    // 5. Re-run `qdev status` to trigger cache rebuild
    let mut cmd_status2 = Command::cargo_bin("qdev").unwrap();
    cmd_status2
        .current_dir(root)
        .args(["status"])
        .assert()
        .success()
        .code(0);

    assert!(cache_db_path.exists(), "cache.sqlite must be recreated");

    // 6. Dump all 14 tables again
    let rebuilt_dump = dump_all_tables(&cache_db_path);

    // 7. Table-by-table comparison: all 14 tables must match exactly
    assert_eq!(
        initial_dump.keys().collect::<Vec<_>>(),
        rebuilt_dump.keys().collect::<Vec<_>>()
    );

    for (table, initial_rows) in &initial_dump {
        let rebuilt_rows = &rebuilt_dump[table];
        // `sync_meta` records *when* each pass ran, not the content it describes, so it is
        // expected to advance between the initial build and the rebuild — comparing its rows
        // makes this test fail whenever the two straddle a second boundary. Its shape is
        // asserted instead, so a pass that failed to stamp is still caught.
        if table == "sync_meta" {
            for (label, rows) in [("initial", initial_rows), ("rebuilt", rebuilt_rows)] {
                assert_eq!(
                    rows.len(),
                    1,
                    "{label} must leave exactly one sync_meta row"
                );
                let stamp = rows[0]
                    .last()
                    .expect("sync_meta row has a timestamp column");
                assert!(
                    stamp.len() == 20 && stamp.ends_with('Z') && stamp.contains('T'),
                    "{label} stamped a malformed last_synced_at: {stamp:?}"
                );
            }
            continue;
        }
        assert_eq!(
            initial_rows, rebuilt_rows,
            "Table '{}' rows differ between initial build and rebuild!\nInitial: {:?}\nRebuilt: {:?}",
            table, initial_rows, rebuilt_rows
        );
    }
}

/// Story 1.14 acceptance: `qdev init` must produce a cache that the very next command finds
/// `Valid` and leaves alone. Before the fix, `init` stamped only `user_version` while
/// `inspect_cache_schema` also demanded SQLite's schema cookie equal the version, so a fresh
/// cache was declared invalid and the next command dropped and rebuilt every table.
#[test]
fn test_init_produces_cache_valid_on_next_command() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    assert_eq!(
        qdev_core::inspect_cache_schema(&cache_db_path).unwrap(),
        qdev_core::CacheSchemaStatus::Valid,
        "the cache `qdev init` just wrote must inspect Valid"
    );

    // A marker index is dropped along with its table by any rebuild, so its survival is proof
    // that the next command did not rebuild. (A marker table would trip the unrelated
    // "no extra tables" check and cause the very rebuild this test is looking for.)
    {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch("CREATE INDEX init_marker_idx ON entities(kind);")
            .unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["list", "stories"])
        .assert()
        .success();

    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let survived: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='index' AND name='init_marker_idx';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        survived,
        "the command after `qdev init` must not rebuild the cache"
    );

    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);
}

/// A cache stamped newer than this binary supports is refused on boot (exit 5) for every
/// command, `qdev doctor` included — except `qdev sync --rebuild`, the recovery path the
/// refusal message itself names.
#[test]
fn test_newer_cache_refused_everywhere_except_sync_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    // An entity file so "rebuilt from files" has something to prove: the row must come back
    // from the Markdown, not merely leave a re-stamped empty cache behind.
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("E12S1.md"),
        r#"---
id: E12S1
kind: story
title: Newer Cache Recovery Story
status: draft
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
created_at: 2026-09-07T00:00:00Z
updated_at: 2026-09-07T00:00:00Z
---

## Acceptance Criteria
- Rebuilt from files after a newer-cache refusal.
"#,
    )
    .unwrap();

    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    let stamp_future = || {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION + 1
        ))
        .unwrap();
    };

    stamp_future();
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["doctor"])
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("newer than supported version"))
        .stderr(predicate::str::contains("qdev sync --rebuild"));

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["list", "stories"])
        .assert()
        .failure()
        .code(5);

    // A plain `qdev sync` is refused too: only the explicit `--rebuild` is the recovery path.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sync"])
        .assert()
        .failure()
        .code(5);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sync", "--rebuild"])
        .assert()
        .success()
        .code(0);

    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        user_version, CACHE_SCHEMA_VERSION,
        "`sync --rebuild` must re-stamp the cache to the supported version"
    );

    let rebuilt: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM entities WHERE id = 'E12S1';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        rebuilt,
        "`sync --rebuild` must repopulate the cache from the Markdown files, not just re-stamp it"
    );

    // And the workspace is healthy again afterwards.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["doctor"]).assert().success();
}
