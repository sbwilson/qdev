use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

use qdev_core::schema::EntityKind;
use qdev_core::store::{
    ensure_cache, inspect_cache_schema, CacheSchemaStatus, ConstraintRecord, DecisionRecord,
    DeferredWorkRecord, EntityFilter, EntityRecord, FindingRecord, GateRecord, GateRunRecord,
    RelationRecord, ScratchpadRecord, SoupRecord, SprintAssignmentRecord, SprintRecord,
    SqliteStore, Store, StoryRecord, ALL_TABLE_NAMES, BUSY_TIMEOUT_MS, CACHE_SCHEMA_VERSION,
    CACHE_USER_VERSION,
};
use qdev_core::write::Author;
use qdev_core::StorageConfig;

#[test]
fn test_schema_creation_and_all_tables_exist() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("test.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::store::create_schema_v2(conn).unwrap();
            Ok(())
        })
        .unwrap();

    store.with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert_eq!(ALL_TABLE_NAMES.len(), 16);
        for &expected in ALL_TABLE_NAMES {
            assert!(
                tables.contains(&expected.to_string()),
                "Table '{}' missing from schema",
                expected
            );
        }
        Ok(())
    }).unwrap();
}

#[test]
fn test_wal_mode_and_busy_timeout_pragmas() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();

    store
        .with_conn(|conn| {
            let mode: String = conn
                .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
                .unwrap();
            assert_eq!(mode.to_lowercase(), "wal");

            let timeout: u64 = conn
                .query_row("PRAGMA busy_timeout;", [], |r| r.get(0))
                .unwrap();
            assert_eq!(timeout, BUSY_TIMEOUT_MS);
            Ok(())
        })
        .unwrap();
}

#[test]
fn test_user_version_and_schema_version_pragmas() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::store::create_schema_v2(conn).unwrap();
            Ok(())
        })
        .unwrap();

    store
        .with_conn(|conn| {
            let user_ver: u32 = conn
                .query_row("PRAGMA user_version;", [], |r| r.get(0))
                .unwrap();
            assert_eq!(user_ver, CACHE_USER_VERSION);

            let schema_ver: u32 = conn
                .query_row("PRAGMA schema_version;", [], |r| r.get(0))
                .unwrap();
            assert_eq!(schema_ver, CACHE_SCHEMA_VERSION);
            Ok(())
        })
        .unwrap();
}

#[test]
fn test_store_entity_crud_and_filters() {
    let store = SqliteStore::open_in_memory().unwrap();

    let record = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("CoreResponse Buffer Layout".to_string()),
        status: Some("ready".to_string()),
        owners: Some("[\"simon\"]".to_string()),
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "hash123".to_string(),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("agent", "claude")),
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: Some("[\"bridge\"]".to_string()),
    };

    // Upsert
    store.upsert_entity(&record).unwrap();

    // Get
    let fetched = store
        .get_entity("E12S4")
        .unwrap()
        .expect("Entity must exist");
    assert_eq!(fetched.id, "E12S4");
    assert_eq!(fetched.kind, EntityKind::Story);
    assert_eq!(fetched.title.as_deref(), Some("CoreResponse Buffer Layout"));
    assert_eq!(fetched.status.as_deref(), Some("ready"));
    assert_eq!(fetched.created_by, Some(Author::new("human", "simon")));
    assert_eq!(fetched.updated_by, Some(Author::new("agent", "claude")));

    // Filter by kind
    let filter = EntityFilter {
        kind: Some(EntityKind::Story),
        status: Some("ready".to_string()),
        ..Default::default()
    };
    let list = store.list_entities(&filter).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, "E12S4");

    // Filter non-matching
    let empty_filter = EntityFilter {
        kind: Some(EntityKind::Epic),
        status: None,
        ..Default::default()
    };
    let empty_list = store.list_entities(&empty_filter).unwrap();
    assert!(empty_list.is_empty());

    // Delete
    let deleted = store.delete_entity("E12S4").unwrap();
    assert!(deleted);
    assert!(store.get_entity("E12S4").unwrap().is_none());
}

#[test]
fn test_store_story_details_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    // Must insert parent entity for foreign key constraint
    let entity = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Buffer".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    store.upsert_entity(&entity).unwrap();

    let story = StoryRecord {
        id: "E12S4".to_string(),
        epic_id: "E12".to_string(),
        seq: 4,
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: Some("[\"bridge\"]".to_string()),
    };

    store.upsert_story_details(&story).unwrap();

    let fetched = store
        .get_story_details("E12S4")
        .unwrap()
        .expect("Story details exist");
    assert_eq!(fetched.epic_id, "E12");
    assert_eq!(fetched.seq, 4);
    assert_eq!(fetched.appetite.as_deref(), Some("small"));
    assert_eq!(fetched.safety_class.as_deref(), Some("ClassB"));

    let all_stories = store.list_story_details().unwrap();
    assert_eq!(all_stories.len(), 1);

    let deleted = store.delete_story_details("E12S4").unwrap();
    assert!(deleted);
    assert!(store.get_story_details("E12S4").unwrap().is_none());
}

#[test]
fn test_store_constraint_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let c = ConstraintRecord {
        id: "E12S4/NG-1".to_string(),
        owner_id: "E12S4".to_string(),
        kind: "no_go".to_string(),
        text: "Do not implement Swift decoding".to_string(),
    };
    store.upsert_constraint(&c).unwrap();

    let fetched = store
        .get_constraint("E12S4/NG-1")
        .unwrap()
        .expect("Constraint exists");
    assert_eq!(fetched.owner_id, "E12S4");
    assert_eq!(fetched.kind, "no_go");
    assert_eq!(fetched.text, "Do not implement Swift decoding");

    let for_owner = store.get_constraints_for_owner("E12S4").unwrap();
    assert_eq!(for_owner.len(), 1);
    assert_eq!(for_owner[0].id, "E12S4/NG-1");

    let deleted = store.delete_constraint("E12S4/NG-1").unwrap();
    assert!(deleted);
    assert!(store.get_constraint("E12S4/NG-1").unwrap().is_none());
}

#[test]
fn test_store_relation_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let rel = RelationRecord {
        source_id: "E12S4".to_string(),
        relation: "depends_on".to_string(),
        target_id: "E12S3".to_string(),
    };
    store.upsert_relation(&rel).unwrap();

    let for_source = store.get_relations_for_source("E12S4").unwrap();
    assert_eq!(for_source.len(), 1);
    assert_eq!(for_source[0].target_id, "E12S3");

    let for_target = store.get_relations_for_target("E12S3").unwrap();
    assert_eq!(for_target.len(), 1);
    assert_eq!(for_target[0].source_id, "E12S4");

    let deleted = store
        .delete_relation("E12S4", "depends_on", "E12S3")
        .unwrap();
    assert!(deleted);
    assert!(store.get_relations_for_source("E12S4").unwrap().is_empty());
}

#[test]
fn test_store_sprint_and_assignments_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let sprint = SprintRecord {
        id: 5,
        title: Some("The Rust Core Port".to_string()),
        release_version: Some("0.1.0".to_string()),
        status: Some("active".to_string()),
        owners: Some("[\"simon\"]".to_string()),
        started_at: Some("2026-09-01T00:00:00Z".to_string()),
        completed_at: None,
    };
    store.upsert_sprint(&sprint).unwrap();

    let fetched_sprint = store.get_sprint(5).unwrap().expect("Sprint exists");
    assert_eq!(fetched_sprint.title.as_deref(), Some("The Rust Core Port"));

    let assignment = SprintAssignmentRecord {
        sprint_id: 5,
        story_id: "E12S4".to_string(),
        assigned_at: "2026-09-01T00:00:00Z".to_string(),
        carried_from: Some(4),
    };
    store.upsert_sprint_assignment(&assignment).unwrap();

    let assignments = store.get_sprint_assignments(5).unwrap();
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].story_id, "E12S4");
    assert_eq!(assignments[0].carried_from, Some(4));

    let story_assignments = store.get_assignments_for_story("E12S4").unwrap();
    assert_eq!(story_assignments.len(), 1);

    store.delete_sprint_assignment(5, "E12S4").unwrap();
    assert!(store.get_sprint_assignments(5).unwrap().is_empty());
}

#[test]
fn test_store_decision_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let dec = DecisionRecord {
        id: "DEC-2b91".to_string(),
        subject_id: "E12S4".to_string(),
        decision_type: Some("human_ruling".to_string()),
        topic: Some("SQLite Sync".to_string()),
        context: Some("AD-2".to_string()),
        ruling: Some("Use bundled rusqlite".to_string()),
        author_type: Some("human".to_string()),
        author_id: Some("simon".to_string()),
        created_at: Some("2026-09-07T00:00:00Z".to_string()),
    };
    store.upsert_decision(&dec).unwrap();

    let fetched = store
        .get_decision("DEC-2b91")
        .unwrap()
        .expect("Decision exists");
    assert_eq!(fetched.subject_id, "E12S4");
    assert_eq!(fetched.decision_type.as_deref(), Some("human_ruling"));

    let for_subject = store.get_decisions_for_subject("E12S4").unwrap();
    assert_eq!(for_subject.len(), 1);

    store.delete_decision("DEC-2b91").unwrap();
    assert!(store.get_decision("DEC-2b91").unwrap().is_none());
}

#[test]
fn test_store_deferred_work_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let dw = DeferredWorkRecord {
        id: "DW-7f3a".to_string(),
        origin_story_id: Some("E12S4".to_string()),
        target_module: "bridge".to_string(),
        status: Some("open".to_string()),
        safety_risk: Some("negligible".to_string()),
        rationale: Some("Edge case optimization".to_string()),
        gate: Some("c-abi-round-trip".to_string()),
        resolution: None,
    };
    store.upsert_deferred_work(&dw).unwrap();

    let fetched = store
        .get_deferred_work("DW-7f3a")
        .unwrap()
        .expect("DW exists");
    assert_eq!(fetched.target_module, "bridge");
    assert_eq!(fetched.safety_risk.as_deref(), Some("negligible"));

    let list = store.list_deferred_work().unwrap();
    assert_eq!(list.len(), 1);

    store.delete_deferred_work("DW-7f3a").unwrap();
    assert!(store.get_deferred_work("DW-7f3a").unwrap().is_none());
}

#[test]
fn test_store_scratchpad_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let entry = ScratchpadRecord {
        story_id: "E12S4".to_string(),
        seq: 1,
        at: "2026-09-07T00:00:00Z".to_string(),
        author_type: Some("agent".to_string()),
        author_id: Some("claude-code".to_string()),
        kind: Some("note".to_string()),
        text: Some("Initial spike complete".to_string()),
    };
    store.upsert_scratchpad_entry(&entry).unwrap();

    let entries = store.get_scratchpad_entries("E12S4").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].text.as_deref(), Some("Initial spike complete"));

    store.delete_scratchpad_entry("E12S4", 1).unwrap();
    assert!(store.get_scratchpad_entries("E12S4").unwrap().is_empty());
}

#[test]
fn test_store_gates_and_runs_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let gate = GateRecord {
        id: "fmt".to_string(),
        command: "cargo fmt --check".to_string(),
        kind: Some("check".to_string()),
        timeout_ms: Some(60000),
        output_adapter: None,
        on_transition: None,
        depends_on: None,
        metric: None,
        direction: None,
    };
    store.upsert_gate(&gate).unwrap();

    let fetched_gate = store.get_gate("fmt").unwrap().expect("Gate exists");
    assert_eq!(fetched_gate.command, "cargo fmt --check");

    let run = GateRunRecord {
        id: "8f1b2c4-fmt".to_string(),
        story_id: Some("E12S4".to_string()),
        gate_id: "fmt".to_string(),
        commit_sha: "8f1b2c4".to_string(),
        status: Some("pass".to_string()),
        exit_code: Some(0),
        duration_ms: Some(1500),
        metric_value: None,
        summary: Some("Formatted".to_string()),
        evidence_path: "docs/state/evidence/fmt.json".to_string(),
        output_hash: None,
        run_by_type: Some("human".to_string()),
        run_by_id: Some("simon".to_string()),
        ran_at: Some("2026-09-07T00:00:00Z".to_string()),
    };
    store.upsert_gate_run(&run).unwrap();

    let fetched_run = store
        .get_gate_run("8f1b2c4-fmt")
        .unwrap()
        .expect("Run exists");
    assert_eq!(fetched_run.commit_sha, "8f1b2c4");
    assert_eq!(fetched_run.status.as_deref(), Some("pass"));

    store.delete_gate_run("8f1b2c4-fmt").unwrap();
    assert!(store.get_gate_run("8f1b2c4-fmt").unwrap().is_none());
}

#[test]
fn test_store_soup_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    let soup = SoupRecord {
        id: "rusqlite@0.31.0".to_string(),
        name: Some("rusqlite".to_string()),
        version: Some("0.31.0".to_string()),
        license: Some("MIT".to_string()),
        cve_status: Some("clean".to_string()),
        introduced_by_story: Some("E12S4".to_string()),
        evaluated_for_release: Some("0.1.0".to_string()),
    };
    store.upsert_soup(&soup).unwrap();

    let fetched = store
        .get_soup("rusqlite@0.31.0")
        .unwrap()
        .expect("Soup exists");
    assert_eq!(fetched.license.as_deref(), Some("MIT"));

    store.delete_soup("rusqlite@0.31.0").unwrap();
    assert!(store.get_soup("rusqlite@0.31.0").unwrap().is_none());
}

#[test]
fn test_store_sync_state_crud() {
    let store = SqliteStore::open_in_memory().unwrap();

    store
        .upsert_sync_state(
            "docs/specs/stories/E12S4.md",
            1234567,
            890,
            Some("sha256abc"),
        )
        .unwrap();

    let fetched = store
        .get_sync_state("docs/specs/stories/E12S4.md")
        .unwrap()
        .expect("Sync state exists");
    assert_eq!(fetched.mtime, 1234567);
    assert_eq!(fetched.size, 890);
    assert_eq!(fetched.content_hash.as_deref(), Some("sha256abc"));

    store
        .delete_sync_state("docs/specs/stories/E12S4.md")
        .unwrap();
    assert!(store
        .get_sync_state("docs/specs/stories/E12S4.md")
        .unwrap()
        .is_none());
}

#[test]
fn test_upsert_entity_round_trips_stale() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut record = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Buffer Layout".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: true,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    };

    store.upsert_entity(&record).unwrap();
    assert!(
        store.get_entity("E12S4").unwrap().unwrap().stale,
        "a stale record must not silently round-trip as fresh"
    );

    record.stale = false;
    store.upsert_entity(&record).unwrap();
    assert!(!store.get_entity("E12S4").unwrap().unwrap().stale);
}

#[test]
fn test_store_findings_crud() {
    let store = SqliteStore::open_in_memory().unwrap();
    let conflicted = "docs/specs/stories/E12S4.md";
    let invalid = "docs/specs/stories/E12S5.md";

    let finding = FindingRecord {
        path: conflicted.to_string(),
        code: "merge_conflict".to_string(),
        severity: "error".to_string(),
        message: Some("File contains merge conflict markers".to_string()),
        found_at: "2026-09-07T00:00:00Z".to_string(),
    };
    store.upsert_finding(&finding).unwrap();
    store
        .upsert_finding(&FindingRecord {
            path: conflicted.to_string(),
            code: "schema_violation".to_string(),
            severity: "error".to_string(),
            message: Some("bad version".to_string()),
            found_at: "2026-09-07T00:00:00Z".to_string(),
        })
        .unwrap();
    store
        .upsert_finding(&FindingRecord {
            path: invalid.to_string(),
            code: "schema_violation".to_string(),
            severity: "error".to_string(),
            message: None,
            found_at: "2026-09-07T00:00:01Z".to_string(),
        })
        .unwrap();

    // Read back one row - every column must round-trip to its own field.
    let fetched = store
        .get_finding(conflicted, "merge_conflict")
        .unwrap()
        .expect("finding exists");
    assert_eq!(fetched.path, conflicted);
    assert_eq!(fetched.code, "merge_conflict");
    assert_eq!(fetched.severity, "error");
    assert_eq!(
        fetched.message.as_deref(),
        Some("File contains merge conflict markers")
    );
    assert_eq!(fetched.found_at, "2026-09-07T00:00:00Z");
    assert!(store
        .get_finding(conflicted, "read_error")
        .unwrap()
        .is_none());
    assert!(store
        .get_finding(invalid, "merge_conflict")
        .unwrap()
        .is_none());

    // Per-path and global reads.
    let for_path = store.get_findings_for_path(conflicted).unwrap();
    assert_eq!(for_path.len(), 2);
    assert!(for_path.iter().all(|f| f.path == conflicted));
    assert_eq!(store.list_findings().unwrap().len(), 3);

    // (path, code) is the primary key: re-upserting replaces severity/message/found_at.
    store
        .upsert_finding(&FindingRecord {
            path: conflicted.to_string(),
            code: "merge_conflict".to_string(),
            severity: "warning".to_string(),
            message: Some("still conflicted".to_string()),
            found_at: "2026-09-08T00:00:00Z".to_string(),
        })
        .unwrap();
    assert_eq!(store.get_findings_for_path(conflicted).unwrap().len(), 2);
    let updated = store
        .get_finding(conflicted, "merge_conflict")
        .unwrap()
        .unwrap();
    assert_eq!(updated.severity, "warning");
    assert_eq!(updated.message.as_deref(), Some("still conflicted"));
    assert_eq!(updated.found_at, "2026-09-08T00:00:00Z");

    // Deleting one path leaves the other path's findings alone.
    assert_eq!(store.delete_findings_for_path(conflicted).unwrap(), 2);
    assert!(store.get_findings_for_path(conflicted).unwrap().is_empty());
    assert_eq!(store.list_findings().unwrap().len(), 1);
    assert_eq!(store.delete_findings_for_path(conflicted).unwrap(), 0);
}

#[test]
fn test_store_dirty_entities() {
    let store = SqliteStore::open_in_memory().unwrap();

    store
        .mark_entity_dirty("E12S4", "2026-09-07T00:00:00Z")
        .unwrap();
    let dirties = store.get_dirty_entities().unwrap();
    assert_eq!(dirties.len(), 1);
    assert_eq!(dirties[0].id, "E12S4");

    store
        .mark_entity_dirty("E12S5", "2026-09-07T00:00:01Z")
        .unwrap();
    assert_eq!(store.get_dirty_entities().unwrap().len(), 2);

    store.clear_dirty_entity("E12S4").unwrap();
    let remaining = store.get_dirty_entities().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, "E12S5");

    store.clear_all_dirty_entities().unwrap();
    assert!(store.get_dirty_entities().unwrap().is_empty());
}

#[test]
fn test_schema_version_mismatch_triggers_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create qdev.toml
    fs::write(root.join("qdev.toml"), "[project]\nname = \"TestProj\"\n").unwrap();

    // Create an entity file: Story E1S1
    let story_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&story_dir).unwrap();
    fs::write(
        story_dir.join("E1S1.md"),
        r#"---
id: E1S1
title: "First Story"
status: draft
version: 1
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

    let storage = StorageConfig::default();

    // 1. First ensure_cache builds the cache
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(
        store.get_entity("E1S1").unwrap().unwrap().title.as_deref(),
        Some("First Story")
    );

    // 2. Corrupt schema version by setting user_version = 0 and injecting outdated legacy table
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
        conn.execute_batch("PRAGMA user_version = 0; CREATE TABLE legacy_dummy (x INT);")
            .unwrap();
    }

    assert_eq!(
        inspect_cache_schema(&cache_db_path).unwrap(),
        CacheSchemaStatus::Mismatch
    );

    // 3. Re-running ensure_cache should automatically detect mismatch, drop legacy table, rebuild from files
    let store2 = ensure_cache(root, &storage).unwrap();

    // Verify user_version and schema_version are back to the v2 values
    store2.with_conn(|conn| {
        let user_ver: u32 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0)).unwrap();
        assert_eq!(user_ver, CACHE_USER_VERSION);
        let schema_ver: u32 = conn.query_row("PRAGMA schema_version;", [], |r| r.get(0)).unwrap();
        assert_eq!(schema_ver, CACHE_SCHEMA_VERSION);

        // Verify legacy table was dropped
        let has_legacy: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='legacy_dummy';",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_legacy, "Legacy table must be dropped on schema rebuild");
        Ok(())
    }).unwrap();

    // Verify entities were rebuilt from Markdown files
    let rebuilt_story = store2
        .get_entity("E1S1")
        .unwrap()
        .expect("Story must be rebuilt");
    assert_eq!(rebuilt_story.title.as_deref(), Some("First Story"));
}

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

#[test]
fn test_table_by_table_identical_cache_after_deletion() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. Create a rich set of workspace files
    // qdev.toml with gates
    fs::write(
        root.join("qdev.toml"),
        r#"[project]
name = "IdenticalTest"

[[gates]]
id = "fmt"
command = "cargo fmt --check"
kind = "check"
timeout_ms = 60000
"#,
    )
    .unwrap();

    // Story with constraints and relations
    let story_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&story_dir).unwrap();
    fs::write(
        story_dir.join("E1S1.md"),
        r#"---
id: E1S1
title: "Story One"
status: ready
version: 2
appetite: small
safety_class: ClassB
target_modules: ["foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: "No swift"
  - id: RH-1
    kind: rabbit_hole
    text: "Watch zero len"
relations:
  depends_on: ["E1S0"]
  traces_to: ["FR-101"]
created_by:
  type: human
  id: alice
updated_by:
  type: agent
  id: bot
---
"#,
    )
    .unwrap();

    // Sprint with assignments
    let sprint_dir = root.join("docs/state/sprints");
    fs::create_dir_all(&sprint_dir).unwrap();
    fs::write(
        sprint_dir.join("sprint-1.md"),
        r#"---
id: sprint-1
title: "Sprint 1"
status: active
release_version: 0.1.0
started_at: "2026-09-01T00:00:00Z"
owners: ["alice"]
assignments:
  - story_id: E1S1
    assigned_at: "2026-09-01T00:00:00Z"
version: 1
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

    // Deferred work
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(
        dw_dir.join("DW-1111.md"),
        r#"---
id: DW-1111
title: "DW One"
status: open
origin_story_id: E1S1
target_module: foundation
safety_risk: negligible
version: 1
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

    // Decision
    let dec_dir = root.join("docs/state/decisions");
    fs::create_dir_all(&dec_dir).unwrap();
    fs::write(
        dec_dir.join("DEC-2222.md"),
        r#"---
id: DEC-2222
title: "Decision One"
status: done
subject_id: E1S1
decision_type: human_ruling
topic: "Architecture"
ruling: "Approved"
version: 1
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

    // Scratchpad
    let scratch_dir = root.join("docs/state/scratch");
    fs::create_dir_all(&scratch_dir).unwrap();
    fs::write(
        scratch_dir.join("E1S1.jsonl"),
        "{\"seq\":1,\"at\":\"2026-09-07T00:00:00Z\",\"author_type\":\"human\",\"author_id\":\"alice\",\"kind\":\"note\",\"text\":\"Spike started\"}\n",
    ).unwrap();

    // SOUP
    let soup_dir = root.join("docs/state/soup");
    fs::create_dir_all(&soup_dir).unwrap();
    fs::write(
        soup_dir.join("rusqlite@0.31.0.md"),
        r#"---
id: rusqlite@0.31.0
status: done
name: rusqlite
dependency_version: 0.31.0
license: MIT
cve_status: clean
version: 1
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

    // Evidence
    let ev_dir = root.join("docs/state/evidence/E1S1");
    fs::create_dir_all(&ev_dir).unwrap();
    fs::write(
        ev_dir.join("abc-fmt.json"),
        r#"{
  "id": "abc-fmt",
  "story_id": "E1S1",
  "gate_id": "fmt",
  "commit_sha": "abc",
  "status": "pass",
  "exit_code": 0,
  "duration_ms": 100,
  "summary": "Formatted"
}"#,
    )
    .unwrap();

    let storage = StorageConfig::default();

    // 2. Build initial cache
    let _store1 = ensure_cache(root, &storage).unwrap();
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    assert!(cache_db_path.exists());

    // 3. Dump all 14 tables
    let initial_dump = dump_all_tables(&cache_db_path);

    // Verify non-trivial data was populated in tables
    assert!(
        !initial_dump["entities"].is_empty(),
        "entities must not be empty"
    );
    assert!(
        !initial_dump["stories"].is_empty(),
        "stories must not be empty"
    );
    assert!(
        !initial_dump["constraints"].is_empty(),
        "constraints must not be empty"
    );
    assert!(
        !initial_dump["relations"].is_empty(),
        "relations must not be empty"
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
        !initial_dump["decisions"].is_empty(),
        "decisions must not be empty"
    );
    assert!(
        !initial_dump["deferred_work"].is_empty(),
        "deferred_work must not be empty"
    );
    assert!(
        !initial_dump["scratchpad_entries"].is_empty(),
        "scratchpad_entries must not be empty"
    );
    assert!(!initial_dump["gates"].is_empty(), "gates must not be empty");
    assert!(
        !initial_dump["gate_runs"].is_empty(),
        "gate_runs must not be empty"
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

    // 5. Re-run ensure_cache
    let _store2 = ensure_cache(root, &storage).unwrap();
    assert!(cache_db_path.exists());

    // 6. Dump all 14 tables again
    let rebuilt_dump = dump_all_tables(&cache_db_path);

    // 7. Table-by-table comparison
    assert_eq!(
        initial_dump.keys().collect::<Vec<_>>(),
        rebuilt_dump.keys().collect::<Vec<_>>()
    );

    for (table, initial_rows) in &initial_dump {
        let rebuilt_rows = &rebuilt_dump[table];
        assert_eq!(
            initial_rows, rebuilt_rows,
            "Table '{}' rows differ between initial build and rebuild from files!\nInitial: {:?}\nRebuilt: {:?}",
            table, initial_rows, rebuilt_rows
        );
    }
}

#[test]
fn test_delete_entity_with_child_rows() {
    let store = SqliteStore::open_in_memory().unwrap();

    let entity = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Child Row Test".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "hash123".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: Some("[\"core\"]".to_string()),
    };
    store.upsert_entity(&entity).unwrap();

    let constraint = ConstraintRecord {
        id: "E12S4/NG-1".to_string(),
        owner_id: "E12S4".to_string(),
        kind: "no_go".to_string(),
        text: "No breaking changes".to_string(),
    };
    store.upsert_constraint(&constraint).unwrap();

    let relation = RelationRecord {
        source_id: "E12S4".to_string(),
        relation: "relates_to".to_string(),
        target_id: "E12".to_string(),
    };
    store.upsert_relation(&relation).unwrap();

    // Verify child rows exist before deletion
    assert!(store.get_story_details("E12S4").unwrap().is_some());
    assert!(store.get_constraint("E12S4/NG-1").unwrap().is_some());
    assert!(!store.get_relations_for_source("E12S4").unwrap().is_empty());

    // Deleting entity should cleanly delete children and avoid FK violations
    let deleted = store.delete_entity("E12S4").unwrap();
    assert!(deleted);
    assert!(store.get_entity("E12S4").unwrap().is_none());
    assert!(store.get_story_details("E12S4").unwrap().is_none());
    assert!(store.get_constraint("E12S4/NG-1").unwrap().is_none());
    assert!(store.get_relations_for_source("E12S4").unwrap().is_empty());
}

#[test]
fn test_delete_sprint_with_child_rows() {
    let store = SqliteStore::open_in_memory().unwrap();

    let sprint = SprintRecord {
        id: 1,
        title: Some("Sprint 1".to_string()),
        release_version: Some("0.1.0".to_string()),
        status: Some("active".to_string()),
        owners: None,
        started_at: Some("2026-09-07T00:00:00Z".to_string()),
        completed_at: None,
    };
    store.upsert_sprint(&sprint).unwrap();

    let assignment = SprintAssignmentRecord {
        sprint_id: 1,
        story_id: "E1S1".to_string(),
        assigned_at: "2026-09-07T00:00:00Z".to_string(),
        carried_from: None,
    };
    store.upsert_sprint_assignment(&assignment).unwrap();

    assert!(!store.list_sprint_assignments().unwrap().is_empty());

    // Deleting sprint should cleanly delete assignments first and avoid FK violations
    let deleted = store.delete_sprint(1).unwrap();
    assert!(deleted);
    assert!(store.get_sprint(1).unwrap().is_none());
    assert!(store.list_sprint_assignments().unwrap().is_empty());
}

#[test]
fn test_upsert_entity_preserves_story_details() {
    let store = SqliteStore::open_in_memory().unwrap();

    let entity = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Detail Preservation".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-07T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        appetite: Some("medium".to_string()),
        safety_class: Some("ClassA".to_string()),
        target_modules: Some("[\"core\"]".to_string()),
    };
    store.upsert_entity(&entity).unwrap();

    let fetched = store.get_entity("E12S4").unwrap().expect("Entity exists");
    assert_eq!(fetched.epic_id.as_deref(), Some("E12"));
    assert_eq!(fetched.seq, Some(4));
    assert_eq!(fetched.appetite.as_deref(), Some("medium"));
    assert_eq!(fetched.safety_class.as_deref(), Some("ClassA"));

    let details = store
        .get_story_details("E12S4")
        .unwrap()
        .expect("Details exist");
    assert_eq!(details.epic_id, "E12");
    assert_eq!(details.seq, 4);
    assert_eq!(details.appetite.as_deref(), Some("medium"));
}

#[test]
fn test_inspect_cache_schema_nonexistent_file() {
    let temp = TempDir::new().unwrap();
    let missing_path = temp.path().join("does_not_exist.sqlite");
    assert!(!missing_path.exists());

    let status = inspect_cache_schema(&missing_path).unwrap();
    assert_eq!(status, CacheSchemaStatus::Mismatch);
    assert!(
        !missing_path.exists(),
        "inspect_cache_schema must not create empty 0-byte file"
    );
}

#[test]
fn test_rebuild_from_workspace_scratchpad_nested_author() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(root.join("qdev.toml"), "[project]\nname = \"NestedTest\"\n").unwrap();

    let scratch_dir = root.join("docs/state/scratch");
    fs::create_dir_all(&scratch_dir).unwrap();
    fs::write(
        scratch_dir.join("E12S4.jsonl"),
        r#"{"seq": 1, "at": "2026-09-07T00:00:00Z", "author": {"type": "agent", "name": "bot_v1"}, "kind": "note", "text": "Nested author note"}"#,
    ).unwrap();

    let storage = StorageConfig::default();
    let store = ensure_cache(root, &storage).unwrap();

    let entries = store.get_scratchpad_entries("E12S4").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].author_type.as_deref(), Some("agent"));
    assert_eq!(entries[0].author_id.as_deref(), Some("bot_v1"));
    assert_eq!(entries[0].text.as_deref(), Some("Nested author note"));
}

#[test]
fn test_schema_version_only_mismatch_triggers_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let cache_dir = root.join(".qdev").join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_db = cache_dir.join("cache.sqlite");

    let storage = StorageConfig::default();
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(
        inspect_cache_schema(&cache_db).unwrap(),
        CacheSchemaStatus::Valid
    );

    // Alter only schema_version while keeping user_version = 1
    store
        .with_conn(|conn| {
            conn.execute_batch("PRAGMA schema_version = 42;").unwrap();
            Ok(())
        })
        .unwrap();

    assert_eq!(
        inspect_cache_schema(&cache_db).unwrap(),
        CacheSchemaStatus::Mismatch
    );

    // Rebuilding through ensure_cache restores valid schema and resets schema_version to 1
    let store2 = ensure_cache(root, &storage).unwrap();
    assert_eq!(
        inspect_cache_schema(&cache_db).unwrap(),
        CacheSchemaStatus::Valid
    );

    store2
        .with_conn(|conn| {
            let sv: u32 = conn
                .query_row("PRAGMA schema_version;", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sv, CACHE_SCHEMA_VERSION);
            Ok(())
        })
        .unwrap();
}
