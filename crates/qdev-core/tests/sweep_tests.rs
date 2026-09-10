//! Incremental hydration sweep tests (spec-1-7).
//!
//! Covers the I/O & edge-case matrix from the spec plus the warm-sweep benchmark bound.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tempfile::TempDir;

use qdev_core::rusqlite;
use qdev_core::store::{
    ensure_cache, inspect_cache_schema, CacheSchemaStatus, EntityRecord, FindingRecord,
    SqliteStore, Store, SweepSummary, ALL_TABLE_NAMES, CACHE_SCHEMA_VERSION,
};
use qdev_core::StorageConfig;

// ---------------------------------------------------------------------------
// Fixtures / helpers
// ---------------------------------------------------------------------------

fn storage() -> StorageConfig {
    StorageConfig::default()
}

/// Writes a minimal valid `qdev.toml` (plus optional gates TOML body) at the workspace root.
fn write_qdev_toml(root: &Path, gates: &str) {
    let body = if gates.is_empty() {
        String::new()
    } else {
        format!("\n{gates}\n")
    };
    fs::write(
        root.join("qdev.toml"),
        format!("[project]\nname = \"SweepTest\"\n{body}"),
    )
    .unwrap();
}

/// Schema-valid story frontmatter + body.
fn story_md(id: &str, title: &str) -> String {
    format!(
        r#"---
id: {id}
title: "{title}"
status: draft
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
Body for {id}
"#
    )
}

fn story_path(root: &Path, id: &str) -> PathBuf {
    root.join("docs/specs/stories").join(format!("{id}.md"))
}

fn write_story(root: &Path, id: &str, title: &str) {
    let path = story_path(root, id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, story_md(id, title)).unwrap();
}

/// Builds a workspace: `qdev.toml` (no gates) plus the given story entities.
fn make_workspace(root: &Path, stories: &[&str]) {
    write_qdev_toml(root, "");
    for id in stories {
        write_story(root, id, &format!("Story {id}"));
    }
}

fn cache_db(root: &Path) -> PathBuf {
    root.join(".qdev/cache/cache.sqlite")
}

/// Directly reads a column from the `entities` table for assertions.
fn entity_stale(store: &SqliteStore, id: &str) -> bool {
    store.get_entity(id).unwrap().unwrap().stale
}

/// Dumps every table as sorted stringified rows. `findings.found_at` (second-resolution wall
/// clock) is dropped so a rebuild and a sweep of the same tree compare equal.
fn dump_tables(db_path: &Path) -> BTreeMap<String, Vec<Vec<String>>> {
    let conn = rusqlite::Connection::open(db_path).unwrap();
    let mut dump = BTreeMap::new();

    for &table in ALL_TABLE_NAMES {
        let mut stmt = match conn.prepare(&format!("SELECT * FROM \"{}\";", table)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let col_count = stmt.column_count();
        let keep = if table == "findings" {
            col_count - 1 // drop found_at
        } else {
            col_count
        };
        let rows = stmt
            .query_map([], |row| {
                let mut vals = Vec::new();
                for i in 0..keep {
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
        let mut row_list: Vec<Vec<String>> = rows.map(|r| r.unwrap()).collect();
        row_list.sort();
        dump.insert(table.to_string(), row_list);
    }
    dump
}

/// A story carrying two constraints and two relations, so re-parse can drop one of each.
fn story_with_children(id: &str, constraints: &[&str], depends_on: &[&str]) -> String {
    let mut s = format!("---\nid: {id}\ntitle: \"Story {id}\"\nstatus: draft\nversion: 1\n");
    if !constraints.is_empty() {
        s.push_str("constraints:\n");
        for c in constraints {
            s.push_str(&format!(
                "  - id: {c}\n    kind: no_go\n    text: \"{c} text\"\n"
            ));
        }
    }
    if !depends_on.is_empty() {
        let targets = depends_on
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ");
        s.push_str(&format!("relations:\n  depends_on: [{targets}]\n"));
    }
    s.push_str(
        "created_by:\n  type: human\n  id: alice\nupdated_by:\n  type: human\n  id: alice\n---\nBody\n",
    );
    s
}

/// A sprint entity file with the given story assignments.
fn sprint_md(id: &str, assignments: &[&str]) -> String {
    let mut s = format!(
        "---\nid: {id}\ntitle: \"Sprint\"\nstatus: active\nversion: 1\nstarted_at: \"2026-09-01T00:00:00Z\"\n"
    );
    if !assignments.is_empty() {
        s.push_str("assignments:\n");
        for a in assignments {
            s.push_str(&format!(
                "  - story_id: {a}\n    assigned_at: \"2026-09-01T00:00:00Z\"\n"
            ));
        }
    }
    s.push_str(
        "created_by:\n  type: human\n  id: alice\nupdated_by:\n  type: human\n  id: alice\n---\nBody\n",
    );
    s
}

fn write_at(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn count_rows(root: &Path, sql: &str) -> i64 {
    rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row(sql, [], |r| r.get(0))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Warm no-op boot
// ---------------------------------------------------------------------------

#[test]
fn test_warm_noop_sweep_reparses_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    let before: EntityRecord = store.get_entity("E1S1").unwrap().unwrap();

    let summary: SweepSummary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 0, "warm sweep must not re-parse any file");
    assert_eq!(summary.purged, 0, "warm sweep must not purge anything");
    assert!(
        summary.unchanged >= 2,
        "scanned files should be reported unchanged (got {})",
        summary.unchanged
    );

    let after = store.get_entity("E1S1").unwrap().unwrap();
    assert_eq!(
        after.content_hash, before.content_hash,
        "row must be untouched"
    );
    assert!(!after.stale);
}

// ---------------------------------------------------------------------------
// Single modified file
// ---------------------------------------------------------------------------

#[test]
fn test_single_modified_file_reparsed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    // Change only E1S1 (a different-length title guarantees a size change).
    fs::write(
        story_path(root, "E1S1"),
        story_md("E1S1", "A Much Longer Changed Title"),
    )
    .unwrap();

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 1, "only the modified file is re-parsed");
    assert_eq!(summary.purged, 0);

    let e1 = store.get_entity("E1S1").unwrap().unwrap();
    assert_eq!(e1.title.as_deref(), Some("A Much Longer Changed Title"));
    assert!(!e1.stale);

    // The untouched file is not re-parsed and not stale.
    let e2 = store.get_entity("E1S2").unwrap().unwrap();
    assert!(!e2.stale);
}

// ---------------------------------------------------------------------------
// Touch, same content (hash unchanged -> no re-parse)
// ---------------------------------------------------------------------------

#[test]
fn test_touch_same_content_no_reparsed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    // Simulate a touch: corrupt the stored mtime so the meta check marks the file a
    // candidate, while leaving content (and thus hash) unchanged.
    {
        let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
        conn.execute(
            "UPDATE sync_state SET mtime = mtime + 999999 WHERE path = 'docs/specs/stories/E1S1.md';",
            [],
        )
        .unwrap();
    }

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(
        summary.parsed, 0,
        "unchanged hash must not trigger a re-parse"
    );
    assert_eq!(summary.purged, 0);

    let e = store.get_entity("E1S1").unwrap().unwrap();
    assert!(!e.stale);

    // sync_state mtime is restored to the real file mtime.
    let stored_mtime: i64 = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row(
            "SELECT mtime FROM sync_state WHERE path = 'docs/specs/stories/E1S1.md';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let actual_mtime = fs::metadata(story_path(root, "E1S1"))
        .unwrap()
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert_eq!(
        stored_mtime, actual_mtime,
        "sync_state mtime refreshed to file mtime"
    );
}

// ---------------------------------------------------------------------------
// In-place id edit (old-id cascade purged, no orphans)
// ---------------------------------------------------------------------------

#[test]
fn test_in_place_id_edit_purges_old_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert!(store.get_entity("E1S1").unwrap().is_some());

    // Same file, new id in frontmatter (E1S2).
    let path = story_path(root, "E1S1");
    fs::write(&path, story_md("E1S2", "Re-identified")).unwrap();

    store.sweep_workspace(root, &storage).unwrap();

    assert!(
        store.get_entity("E1S1").unwrap().is_none(),
        "old id row must be purged"
    );
    assert!(
        store.get_entity("E1S2").unwrap().is_some(),
        "new id row must be upserted"
    );
    assert_eq!(
        store.get_entity("E1S2").unwrap().unwrap().title.as_deref(),
        Some("Re-identified")
    );
}

// ---------------------------------------------------------------------------
// Branch switch: add / change / remove
// ---------------------------------------------------------------------------

#[test]
fn test_branch_switch_add_change_remove_purge() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert!(store.get_entity("E1S1").unwrap().is_some());
    assert!(store.get_entity("E1S2").unwrap().is_some());

    // Change E1S1, add E1S3, remove E1S2.
    fs::write(story_path(root, "E1S1"), story_md("E1S1", "Changed")).unwrap();
    write_story(root, "E1S3", "Brand New");
    fs::remove_file(story_path(root, "E1S2")).unwrap();

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 2, "changed + added files re-parsed");
    assert_eq!(summary.purged, 1, "removed file purged");

    assert_eq!(
        store.get_entity("E1S1").unwrap().unwrap().title.as_deref(),
        Some("Changed")
    );
    assert!(
        store.get_entity("E1S3").unwrap().is_some(),
        "added file present"
    );
    assert!(
        store.get_entity("E1S2").unwrap().is_none(),
        "removed file gone"
    );

    // Removed file's sync_state row is purged.
    let n: i64 = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sync_state WHERE path = 'docs/specs/stories/E1S2.md';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 0, "removed file sync_state purged");
}

// ---------------------------------------------------------------------------
// Merge conflict: finding + stale, others continue, non-fatal
// ---------------------------------------------------------------------------

#[test]
fn test_merge_conflict_finding_and_stale() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert!(store
        .get_findings_for_path("docs/specs/stories/E1S1.md")
        .unwrap()
        .is_empty());

    // Inject conflict markers into E1S1 (E1S2 stays valid).
    let conflicted =
        story_md("E1S1", "Story E1S1") + "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\n";
    fs::write(story_path(root, "E1S1"), conflicted).unwrap();

    // The sweep must succeed (findings are non-fatal).
    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(
        summary.parsed, 0,
        "a conflicted file is never parsed/upserted"
    );
    assert_eq!(summary.findings, 1, "one finding recorded");

    let f1: Vec<FindingRecord> = store
        .get_findings_for_path("docs/specs/stories/E1S1.md")
        .unwrap();
    assert_eq!(f1.len(), 1);
    assert_eq!(f1[0].code, "merge_conflict");
    assert_eq!(f1[0].severity, "error");

    // The healthy file has no finding and is not stale.
    assert!(store
        .get_findings_for_path("docs/specs/stories/E1S2.md")
        .unwrap()
        .is_empty());
    assert!(!entity_stale(&store, "E1S2"));

    // E1S1's previous row is retained and flagged stale.
    assert!(entity_stale(&store, "E1S1"));
    assert_eq!(
        store.get_entity("E1S1").unwrap().unwrap().title.as_deref(),
        Some("Story E1S1")
    );
}

// ---------------------------------------------------------------------------
// Schema violation: finding + stale retained, cleared once fixed
// ---------------------------------------------------------------------------

#[test]
fn test_schema_violation_finding_stale_and_cleared_on_fix() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert!(!entity_stale(&store, "E1S1"));

    // Make frontmatter schema-invalid (version is not an integer).
    let invalid = story_md("E1S1", "Story E1S1").replace("version: 1", "version: not-a-number");
    fs::write(story_path(root, "E1S1"), invalid).unwrap();

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 0, "invalid file is not upserted");
    assert_eq!(summary.findings, 1);

    let f: Vec<FindingRecord> = store
        .get_findings_for_path("docs/specs/stories/E1S1.md")
        .unwrap();
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].code, "schema_violation");
    assert!(f[0].message.as_deref().unwrap().contains("version"));
    assert!(
        entity_stale(&store, "E1S1"),
        "previous row retained and flagged stale"
    );

    // Fix the file and re-boot: finding cleared, stale cleared.
    fs::write(story_path(root, "E1S1"), story_md("E1S1", "Fixed Story")).unwrap();
    let summary2 = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary2.parsed, 1, "fixed file re-parsed");
    assert_eq!(summary2.findings, 0, "finding cleared once fixed");
    assert!(store
        .get_findings_for_path("docs/specs/stories/E1S1.md")
        .unwrap()
        .is_empty());
    assert!(
        !entity_stale(&store, "E1S1"),
        "stale flag cleared on successful parse"
    );
    assert_eq!(
        store.get_entity("E1S1").unwrap().unwrap().title.as_deref(),
        Some("Fixed Story")
    );
}

// ---------------------------------------------------------------------------
// qdev write (dirty) forces re-parse even when mtime/size match; row cleared
// ---------------------------------------------------------------------------

#[test]
fn test_dirty_row_forced_reparsed_and_cleared() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    let before_hash: String = store
        .get_entity("E1S1")
        .unwrap()
        .unwrap()
        .content_hash
        .clone();

    // Simulate `qdev update`: mark the entity dirty (sync_state left intact so mtime/size
    // match — the dirty flag alone must force a re-parse).
    store
        .mark_entity_dirty("E1S1", "2026-09-07T00:00:00Z")
        .unwrap();
    assert_eq!(store.get_dirty_entities().unwrap().len(), 1);

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(
        summary.parsed, 1,
        "dirty entity re-parsed despite unchanged mtime/size"
    );

    // Dirty row cleared after consumption.
    assert!(
        store.get_dirty_entities().unwrap().is_empty(),
        "consumed dirty row cleared"
    );

    // Content unchanged (same file), hash stable.
    assert_eq!(
        store.get_entity("E1S1").unwrap().unwrap().content_hash,
        before_hash
    );
}

#[test]
fn test_dirty_row_with_deleted_sync_state_restored() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    // Mimic `qdev update` exactly: mark the entity dirty AND delete its sync_state row
    // (the write path invalidates the row so the sweep must treat the file as new).
    store
        .mark_entity_dirty("E1S1", "2026-09-07T00:00:00Z")
        .unwrap();
    rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .execute(
            "DELETE FROM sync_state WHERE path = 'docs/specs/stories/E1S1.md';",
            [],
        )
        .unwrap();

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(
        summary.parsed, 1,
        "written entity re-parsed after write-path invalidation"
    );
    assert!(
        store.get_dirty_entities().unwrap().is_empty(),
        "consumed dirty row cleared"
    );

    // sync_state row restored with the file's current metadata and hash.
    let row: Option<(i64, i64, Option<String>)> = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row(
            "SELECT mtime, size, content_hash FROM sync_state WHERE path = 'docs/specs/stories/E1S1.md';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let (stored_mtime, stored_size, stored_hash) = row.expect("sync_state row restored");
    let md = fs::metadata(story_path(root, "E1S1")).unwrap();
    let actual_mtime = md
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert_eq!(stored_mtime, actual_mtime, "restored mtime matches file");
    assert_eq!(stored_size, md.len() as i64, "restored size matches file");
    assert!(stored_hash.is_some(), "restored hash present");
}

// ---------------------------------------------------------------------------
// Gates edited in qdev.toml are re-upserted on next boot
// ---------------------------------------------------------------------------

fn gate_count(root: &Path) -> i64 {
    rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row("SELECT count(*) FROM gates;", [], |r| r.get(0))
        .unwrap()
}

fn gate_command(root: &Path, id: &str) -> Option<String> {
    rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row(
            "SELECT command FROM gates WHERE id = ?1;",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok()
}

#[test]
fn test_gates_refresh_on_config_change() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();

    // Initial qdev.toml with one gate.
    write_qdev_toml(root, "[[gates]]\nid = \"lint\"\ncommand = \"cargo clippy\"");
    let store = ensure_cache(root, &storage).unwrap();

    assert_eq!(gate_count(root), 1, "initial gate hydrated");
    assert_eq!(gate_command(root, "lint"), Some("cargo clippy".to_string()));

    // Add a second gate and change the first.
    write_qdev_toml(
        root,
        "[[gates]]\nid = \"lint\"\ncommand = \"cargo clippy -- -D warnings\"\n\n[[gates]]\nid = \"test\"\ncommand = \"cargo test\"",
    );

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 1, "qdev.toml re-processed");

    assert_eq!(gate_count(root), 2, "both gates present after refresh");
    assert_eq!(
        gate_command(root, "lint"),
        Some("cargo clippy -- -D warnings".to_string()),
        "gate command updated"
    );
    assert_eq!(gate_command(root, "test"), Some("cargo test".to_string()));
}

// ---------------------------------------------------------------------------
// v1 cache auto-rebuilds losslessly to v2
// ---------------------------------------------------------------------------

#[test]
fn test_v1_cache_auto_rebuilds_to_current_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();

    // Pre-create a v1 cache (user_version = 1).
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
    }

    // Any boot detects the mismatch and rebuilds to the current schema version.
    let store = ensure_cache(root, &storage).unwrap();

    let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
    let user_ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        user_ver, CACHE_SCHEMA_VERSION,
        "user_version rebuilt to the current version"
    );

    // Every table present.
    let tables: Vec<String> = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();
    assert_eq!(
        tables.len(),
        ALL_TABLE_NAMES.len(),
        "rebuilt cache has {} tables",
        ALL_TABLE_NAMES.len()
    );

    // Lossless: entities rebuilt from files.
    assert!(store.get_entity("E1S1").unwrap().is_some());
    assert!(store.get_entity("E1S2").unwrap().is_some());
}

// ---------------------------------------------------------------------------
// Rebuild and sweep produce identical findings/stale state
// ---------------------------------------------------------------------------

#[test]
fn test_rebuild_and_sweep_findings_equal() {
    // The sweep must land exactly the cache state a full rebuild of the same tree produces -
    // including findings, stale flags and every child table - after real changes to the tree.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_at(
        root,
        "docs/specs/stories/E1S1.md",
        &story_with_children("E1S1", &["NG-1", "NG-2"], &["E1S0", "E1S9"]),
    );
    write_story(root, "E1S2", "Two");
    write_story(root, "E1S3", "Three");
    write_at(
        root,
        "docs/state/sprints/sprint-1.md",
        &sprint_md("sprint-1", &["E1S1", "E1S2"]),
    );
    // The three shapes the first version of this test deliberately avoided, and behind which
    // four convergence defects lived: a removed file with an edge pointing *into* it, a
    // duplicate-id pair whose row-owning file is the one removed, and an unreadable file.
    write_at(
        root,
        "docs/specs/stories/E1S5.md",
        &story_with_children("E1S5", &[], &["E1S6"]),
    );
    write_story(root, "E1S6", "Depended on");
    write_story(root, "E1S7", "Duplicate keeper");
    write_at(
        root,
        "docs/specs/stories/E1S7-copy.md",
        &story_md("E1S7", "Duplicate copy"),
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    // Sorted last of the two files declaring E1S7, so it is the one that owns the cache row -
    // and therefore the one whose removal used to erase the survivor.
    assert_eq!(
        store.get_entity("E1S7").unwrap().unwrap().source_path,
        "docs/specs/stories/E1S7.md"
    );

    // Mutate: drop a constraint, a relation and an assignment; add a file; remove a file; add a
    // conflicted file and a schema-invalid file (both new, so neither path has a previous row);
    // remove a file an unchanged file depends on; remove the duplicate that owned the row.
    fs::remove_file(story_path(root, "E1S6")).unwrap();
    fs::remove_file(story_path(root, "E1S7")).unwrap();
    write_at(
        root,
        "docs/specs/stories/E1S1.md",
        &story_with_children("E1S1", &["NG-1"], &["E1S0"]),
    );
    write_at(
        root,
        "docs/state/sprints/sprint-1.md",
        &sprint_md("sprint-1", &["E1S1"]),
    );
    write_story(root, "E1S4", "Added");
    fs::remove_file(story_path(root, "E1S3")).unwrap();
    fs::write(
        story_path(root, "E1S8"),
        story_md("E1S8", "Conflicted") + "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> b\n",
    )
    .unwrap();
    fs::write(
        story_path(root, "E1S9"),
        story_md("E1S9", "Invalid").replace("version: 1", "version: bogus"),
    )
    .unwrap();

    // An unreadable file, new so neither path has a previous row to retain (retention on a
    // *previously parsed* file is a deliberate divergence, asserted on its own below).
    let unreadable = make_unreadable(root, "docs/specs/stories/E1S9-unreadable.md");

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert!(summary.parsed >= 3, "changed/added files must re-parse");
    assert_eq!(summary.purged, 3, "every removed file must be purged");
    let swept = dump_tables(&cache_db(root));

    // A full rebuild of the very same tree must produce the same rows in every table.
    store.reset_and_rebuild(root, &storage).unwrap();
    let rebuilt = dump_tables(&cache_db(root));

    for &table in ALL_TABLE_NAMES {
        // `sync_meta` legitimately differs: it stamps *when* each pass ran, not the content it
        // describes, so it is expected to advance between the sweep and the rebuild that
        // follows. Its shape is still asserted below rather than skipped outright — a pass
        // that failed to stamp, or wrongly cleared the table, must not slip through here.
        if table == "sync_meta" {
            for (label, dump) in [("sweep", &swept), ("rebuild", &rebuilt)] {
                let rows = dump.get(table).expect("sync_meta table must exist");
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
            swept.get(table),
            rebuilt.get(table),
            "table '{table}' diverges between sweep and rebuild"
        );
    }

    // Sanity: the mutation really did exercise the paths under test.
    assert_eq!(
        swept["constraints"].len(),
        1,
        "the dropped constraint must be gone, not merged"
    );
    assert_eq!(
        swept["relations"].len(),
        2,
        "the dropped relation must be gone, not merged - E1S1 -> E1S0 and the surviving \
         E1S5 -> E1S6 edge into the removed file are all that is left"
    );
    assert_eq!(
        swept["sprint_assignments"].len(),
        1,
        "the dropped sprint assignment must be gone, not merged"
    );
    let codes: Vec<&str> = swept["findings"].iter().map(|r| r[1].as_str()).collect();
    assert!(codes.contains(&"merge_conflict"), "codes: {codes:?}");
    assert!(codes.contains(&"schema_violation"), "codes: {codes:?}");
    assert!(
        codes.contains(&"dangling_relation"),
        "the edge into the removed file must survive to be reported: {codes:?}"
    );
    if unreadable {
        assert!(codes.contains(&"read_error"), "codes: {codes:?}");
    }
    assert_eq!(
        swept["entities"]
            .iter()
            .filter(|r| r[0] == "E1S7")
            .map(|r| r[5].as_str())
            .collect::<Vec<_>>(),
        vec!["docs/specs/stories/E1S7-copy.md"],
        "the surviving duplicate must be re-parsed, not left purged"
    );
}

/// `chmod 000` the file at `rel`, returning whether it really is unreadable now. Running as
/// root (or on a platform without POSIX modes) reads it anyway, in which case the caller skips
/// the unreadable half of its assertions rather than failing for the wrong reason.
fn make_unreadable(root: &Path, rel: &str) -> bool {
    make_unreadable_with(root, rel, &story_md("E1S9U", "Unreadable"))
}

/// `make_unreadable` for a file that is not an entity markdown story — the scratch (`.jsonl`) and
/// evidence (`.json`) read sites take their own branches in both hydration paths, and were
/// reachable by no test while the helper only ever wrote a story.
fn make_unreadable_with(root: &Path, rel: &str, content: &str) -> bool {
    write_at(root, rel, content);
    let path = root.join(rel);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    }
    if fs::read_to_string(&path).is_ok() {
        fs::remove_file(&path).unwrap();
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// A purge deletes only what the removed file owned
// ---------------------------------------------------------------------------

#[test]
fn test_purge_keeps_inbound_edges_and_reports_them_dangling() {
    // Deleting E1S1.md must not delete the `depends_on` edge E1S2.md declares: E1S2.md is
    // unchanged, nothing re-parses it, and the edge is the fact the user needs reported.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    write_at(
        root,
        "docs/specs/stories/E1S2.md",
        &story_with_children("E1S2", &[], &["E1S1"]),
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(count_rows(root, "SELECT COUNT(*) FROM relations;"), 1);

    fs::remove_file(story_path(root, "E1S1")).unwrap();
    store.sweep_workspace(root, &storage).unwrap();

    assert!(
        store.get_entity("E1S1").unwrap().is_none(),
        "the removed file's own entity must be purged"
    );
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM relations WHERE source_id = 'E1S2' AND target_id = 'E1S1';"
        ),
        1,
        "the edge declared by the unchanged file must survive the purge"
    );
    let dangling: Vec<FindingRecord> = store
        .list_findings()
        .unwrap()
        .into_iter()
        .filter(|f| f.code == "dangling_relation")
        .collect();
    assert_eq!(
        dangling.len(),
        1,
        "the surviving edge must be reported dangling: {:?}",
        store.list_findings().unwrap()
    );
    assert_eq!(dangling[0].path, "docs/specs/stories/E1S2.md");
    assert_eq!(dangling[0].severity, "error");
}

// ---------------------------------------------------------------------------
// Every known, readable file is accounted for
// ---------------------------------------------------------------------------

#[test]
fn test_deleting_the_duplicate_that_owned_the_row_reparses_the_survivor() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "Keeper");
    write_at(
        root,
        "docs/specs/stories/E1S1-copy.md",
        &story_md("E1S1", "Copy"),
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(
        store.get_entity("E1S1").unwrap().unwrap().source_path,
        "docs/specs/stories/E1S1.md",
        "the last path in sorted order owns the row"
    );

    // The natural response to `duplicate_planning_id`: delete the copy. Here it is the file the
    // cache points at that goes, which used to purge the entity wholesale.
    fs::remove_file(story_path(root, "E1S1")).unwrap();
    store.sweep_workspace(root, &storage).unwrap();

    let survivor = store
        .get_entity("E1S1")
        .unwrap()
        .expect("the entity must still resolve from the file still on disk");
    assert_eq!(survivor.source_path, "docs/specs/stories/E1S1-copy.md");
    assert!(!survivor.stale);
}

#[test]
fn test_deleting_both_duplicates_leaves_no_entity_and_no_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "Keeper");
    write_at(
        root,
        "docs/specs/stories/E1S1-copy.md",
        &story_md("E1S1", "Copy"),
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    fs::remove_file(story_path(root, "E1S1")).unwrap();
    fs::remove_file(root.join("docs/specs/stories/E1S1-copy.md")).unwrap();
    store.sweep_workspace(root, &storage).unwrap();

    assert!(store.get_entity("E1S1").unwrap().is_none());
    assert_eq!(
        store.list_findings().unwrap(),
        Vec::new(),
        "nothing is left to report once both files are gone"
    );

    // And a rebuild of the same tree agrees.
    let swept = dump_tables(&cache_db(root));
    store.reset_and_rebuild(root, &storage).unwrap();
    let rebuilt = dump_tables(&cache_db(root));
    assert_eq!(swept.get("entities"), rebuilt.get("entities"));
    assert_eq!(swept.get("findings"), rebuilt.get("findings"));
}

#[test]
fn test_unchanged_file_whose_row_is_intact_is_still_skipped() {
    // The accounted-for check must not turn every warm boot into a re-parse: a file with a row,
    // and a file with a finding explaining why it has none, are both left alone.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    fs::write(
        story_path(root, "E1S8"),
        story_md("E1S8", "Conflicted") + "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> b\n",
    )
    .unwrap();
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(
        summary.parsed, 0,
        "a warm sweep must not re-parse a file that is accounted for"
    );
    assert_eq!(summary.purged, 0);
}

// ---------------------------------------------------------------------------
// One answer to an unreadable file
// ---------------------------------------------------------------------------

#[test]
fn test_unreadable_file_records_read_error_on_both_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    if !make_unreadable(root, "docs/specs/stories/E1S9U.md") {
        eprintln!("skipping: files are readable regardless of mode (running as root?)");
        return;
    }

    // Sweep path: a new unreadable file is reported, never silently swept.
    store.sweep_workspace(root, &storage).unwrap();
    let swept_codes: Vec<String> = store
        .list_findings()
        .unwrap()
        .into_iter()
        .map(|f| f.code)
        .collect();
    assert!(
        swept_codes.contains(&"read_error".to_string()),
        "sweep codes: {swept_codes:?}"
    );

    // Rebuild path: the same answer, rather than the silent `continue` that left the findings
    // table emptied and unrepopulated for the command that triggered the rebuild.
    store.reset_and_rebuild(root, &storage).unwrap();
    let rebuilt: Vec<FindingRecord> = store
        .list_findings()
        .unwrap()
        .into_iter()
        .filter(|f| f.code == "read_error")
        .collect();
    assert_eq!(
        rebuilt.len(),
        1,
        "a rebuild must report the unreadable file too"
    );
    assert_eq!(rebuilt[0].path, "docs/specs/stories/E1S9U.md");
    assert_eq!(rebuilt[0].severity, "error");
}

#[test]
fn test_unreadable_previously_parsed_file_retains_its_rows_stale() {
    // The one deliberate divergence: a file that parsed before and cannot be read now keeps its
    // previous rows, flagged stale, where a rebuild has no rows for it at all. The *findings*
    // still converge, and `qdev sync --rebuild` remains the repair for the rest.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    // Rewrite (so mtime/size move and the sweep treats it as a candidate) then make it
    // unreadable.
    write_story(root, "E1S1", "One edited");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(story_path(root, "E1S1"), fs::Permissions::from_mode(0o000)).unwrap();
    }
    if fs::read_to_string(story_path(root, "E1S1")).is_ok() {
        eprintln!("skipping: files are readable regardless of mode (running as root?)");
        return;
    }

    store.sweep_workspace(root, &storage).unwrap();
    assert!(
        entity_stale(&store, "E1S1"),
        "the retained row must be flagged stale"
    );
    let swept_findings = store.list_findings().unwrap();
    assert_eq!(swept_findings.len(), 1);
    assert_eq!(swept_findings[0].code, "read_error");

    store.reset_and_rebuild(root, &storage).unwrap();
    let rebuilt_findings = store.list_findings().unwrap();
    assert_eq!(
        rebuilt_findings
            .iter()
            .map(|f| (f.path.as_str(), f.code.as_str()))
            .collect::<Vec<_>>(),
        swept_findings
            .iter()
            .map(|f| (f.path.as_str(), f.code.as_str()))
            .collect::<Vec<_>>(),
        "both paths report the same read_error"
    );
    assert!(
        store.get_entity("E1S1").unwrap().is_none(),
        "a rebuild has no previous row to retain - the documented exception to convergence"
    );
}

// ---------------------------------------------------------------------------
// Re-parse replaces child rows
// ---------------------------------------------------------------------------

#[test]
fn test_reparse_replaces_child_rows() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_at(
        root,
        "docs/specs/stories/E1S1.md",
        &story_with_children("E1S1", &["NG-1", "NG-2"], &["E1S0", "E1S9"]),
    );
    write_at(
        root,
        "docs/state/sprints/sprint-1.md",
        &sprint_md("sprint-1", &["E1S1", "E1S2"]),
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(count_rows(root, "SELECT COUNT(*) FROM constraints;"), 2);
    assert_eq!(count_rows(root, "SELECT COUNT(*) FROM relations;"), 2);
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM sprint_assignments;"),
        2
    );

    // Drop one of each from the files.
    write_at(
        root,
        "docs/specs/stories/E1S1.md",
        &story_with_children("E1S1", &["NG-2"], &["E1S9"]),
    );
    write_at(
        root,
        "docs/state/sprints/sprint-1.md",
        &sprint_md("sprint-1", &["E1S2"]),
    );
    store.sweep_workspace(root, &storage).unwrap();

    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM constraints;"),
        1,
        "a constraint removed from the file must not survive the sweep"
    );
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM constraints WHERE id = 'E1S1/NG-2';"
        ),
        1,
        "the surviving constraint must be the one still declared"
    );
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM relations;"),
        1,
        "a relation removed from the file must not survive the sweep"
    );
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM relations WHERE target_id = 'E1S9';"
        ),
        1
    );
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM sprint_assignments;"),
        1,
        "an assignment removed from the sprint file must not survive the sweep"
    );
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM sprint_assignments WHERE story_id = 'E1S2';"
        ),
        1
    );
}

// ---------------------------------------------------------------------------
// Removal purge covers every kind and file role
// ---------------------------------------------------------------------------

#[test]
fn test_purge_removes_child_rows_for_every_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "[[gates]]\nid = \"fmt\"\ncommand = \"cargo fmt\"\n");
    write_at(
        root,
        "docs/state/sprints/sprint-1.md",
        &sprint_md("sprint-1", &["E1S1"]),
    );
    write_at(
        root,
        "docs/state/dw/DW-1111.md",
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
    );
    write_at(
        root,
        "docs/state/decisions/DEC-2222.md",
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
    );
    write_at(
        root,
        "docs/state/soup/rusqlite@0.31.0.md",
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
    );
    write_at(
        root,
        "docs/state/scratch/E1S1.jsonl",
        "{\"seq\":1,\"at\":\"2026-09-07T00:00:00Z\",\"author_type\":\"human\",\"author_id\":\"alice\",\"kind\":\"note\",\"text\":\"Spike\"}\n",
    );
    write_at(
        root,
        "docs/state/evidence/E1S1/abc-fmt.json",
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
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    for (table, sql) in [
        ("sprints", "SELECT COUNT(*) FROM sprints;"),
        (
            "sprint_assignments",
            "SELECT COUNT(*) FROM sprint_assignments;",
        ),
        ("deferred_work", "SELECT COUNT(*) FROM deferred_work;"),
        ("decisions", "SELECT COUNT(*) FROM decisions;"),
        (
            "soup_dependencies",
            "SELECT COUNT(*) FROM soup_dependencies;",
        ),
        (
            "scratchpad_entries",
            "SELECT COUNT(*) FROM scratchpad_entries;",
        ),
        ("gate_runs", "SELECT COUNT(*) FROM gate_runs;"),
        ("gates", "SELECT COUNT(*) FROM gates;"),
    ] {
        assert!(
            count_rows(root, sql) > 0,
            "fixture must populate '{table}' before the removal"
        );
    }

    // Remove every file, including qdev.toml, then sweep.
    for rel in [
        "docs/state/sprints/sprint-1.md",
        "docs/state/dw/DW-1111.md",
        "docs/state/decisions/DEC-2222.md",
        "docs/state/soup/rusqlite@0.31.0.md",
        "docs/state/scratch/E1S1.jsonl",
        "docs/state/evidence/E1S1/abc-fmt.json",
        "qdev.toml",
    ] {
        fs::remove_file(root.join(rel)).unwrap();
    }
    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.purged, 7, "every removed file must be purged");

    for (table, sql) in [
        ("entities", "SELECT COUNT(*) FROM entities;"),
        ("sprints", "SELECT COUNT(*) FROM sprints;"),
        (
            "sprint_assignments",
            "SELECT COUNT(*) FROM sprint_assignments;",
        ),
        ("deferred_work", "SELECT COUNT(*) FROM deferred_work;"),
        ("decisions", "SELECT COUNT(*) FROM decisions;"),
        (
            "soup_dependencies",
            "SELECT COUNT(*) FROM soup_dependencies;",
        ),
        (
            "scratchpad_entries",
            "SELECT COUNT(*) FROM scratchpad_entries;",
        ),
        ("gate_runs", "SELECT COUNT(*) FROM gate_runs;"),
        ("gates", "SELECT COUNT(*) FROM gates;"),
        ("sync_state", "SELECT COUNT(*) FROM sync_state;"),
    ] {
        assert_eq!(
            count_rows(root, sql),
            0,
            "'{table}' must be empty after every file was removed"
        );
    }
}

// ---------------------------------------------------------------------------
// Scratch (.jsonl) and evidence (.json) files are swept
// ---------------------------------------------------------------------------

#[test]
fn test_scratch_and_evidence_files_swept() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    write_at(
        root,
        "docs/state/scratch/E1S1.jsonl",
        "{\"seq\":1,\"at\":\"2026-09-07T00:00:00Z\",\"author_type\":\"human\",\"author_id\":\"alice\",\"kind\":\"note\",\"text\":\"One\"}\n",
    );
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM scratchpad_entries;"),
        1
    );

    // An unchanged scratch file must not be re-parsed and must not be purged.
    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 0);
    assert_eq!(summary.purged, 0);
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM scratchpad_entries;"),
        1
    );

    // Appending a line and adding an evidence file must reach the cache on the next sweep.
    write_at(
        root,
        "docs/state/scratch/E1S1.jsonl",
        "{\"seq\":1,\"at\":\"2026-09-07T00:00:00Z\",\"author_type\":\"human\",\"author_id\":\"alice\",\"kind\":\"note\",\"text\":\"One\"}\n{\"seq\":2,\"at\":\"2026-09-07T00:01:00Z\",\"author_type\":\"human\",\"author_id\":\"alice\",\"kind\":\"note\",\"text\":\"Two\"}\n",
    );
    write_at(
        root,
        "docs/state/evidence/E1S1/abc-fmt.json",
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
    );
    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 2, "the scratch and evidence files re-parse");
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM scratchpad_entries;"),
        2,
        "the appended scratchpad line must reach the cache"
    );
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM gate_runs;"),
        1,
        "the new evidence file must reach the cache"
    );
}

// ---------------------------------------------------------------------------
// Unreadable file: recorded as a finding, dirty row retained
// ---------------------------------------------------------------------------

#[test]
fn test_unreadable_file_records_finding_and_keeps_dirty() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    store
        .mark_entity_dirty("E1S1", "2026-09-07T00:00:00Z")
        .unwrap();

    // Non-UTF-8 bytes make `read_to_string` fail.
    fs::write(story_path(root, "E1S1"), [0xff, 0xfe, 0xfd]).unwrap();
    let summary = store.sweep_workspace(root, &storage).unwrap();

    let findings = store
        .get_findings_for_path("docs/specs/stories/E1S1.md")
        .unwrap();
    assert_eq!(findings.len(), 1, "one finding for the unreadable file");
    assert_eq!(findings[0].code, "read_error");
    assert_eq!(findings[0].severity, "error");
    assert!(summary.findings >= 1);

    assert!(
        entity_stale(&store, "E1S1"),
        "the retained row must be flagged stale"
    );
    assert_eq!(
        count_rows(root, "SELECT COUNT(*) FROM dirty_entities;"),
        1,
        "a file that never parsed must not consume its dirty row"
    );

    // Restoring readable content clears the finding and the flags.
    write_story(root, "E1S1", "Recovered");
    store.sweep_workspace(root, &storage).unwrap();
    assert!(store
        .get_findings_for_path("docs/specs/stories/E1S1.md")
        .unwrap()
        .is_empty());
    assert!(!entity_stale(&store, "E1S1"));
    assert_eq!(count_rows(root, "SELECT COUNT(*) FROM dirty_entities;"), 0);
}

// ---------------------------------------------------------------------------
// A real v1 cache (14 tables, no entities.stale)
// ---------------------------------------------------------------------------

/// Turns the v2 cache at `root` into a genuine v1 shape: 14 tables, `entities` without
/// `stale`, `user_version` at 1.
fn downgrade_cache_to_v1(root: &Path) {
    let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
    conn.execute_batch(
        "DROP TABLE findings;
         ALTER TABLE entities DROP COLUMN stale;
         PRAGMA user_version = 1;",
    )
    .unwrap();
}

#[test]
fn test_real_v1_cache_rebuilds_to_current_schema_and_matches_a_fresh_sweep() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_at(
        root,
        "docs/specs/stories/E1S1.md",
        &story_with_children("E1S1", &["NG-1"], &["E1S0"]),
    );
    write_story(root, "E1S2", "Two");
    write_at(
        root,
        "docs/state/sprints/sprint-1.md",
        &sprint_md("sprint-1", &["E1S1"]),
    );
    let storage = storage();
    drop(ensure_cache(root, &storage).unwrap());
    downgrade_cache_to_v1(root);

    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Mismatch,
        "a v1 cache must be detected as a mismatch"
    );
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM pragma_table_info('entities') WHERE name = 'stale';"
        ),
        0,
        "the downgraded cache really lacks entities.stale"
    );

    // Any boot rebuilds it losslessly to the current schema version.
    let store = ensure_cache(root, &storage).unwrap();
    let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
    let user_ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(user_ver, CACHE_SCHEMA_VERSION);
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        tables,
        ALL_TABLE_NAMES.len() as i64,
        "rebuilt cache has {} tables",
        ALL_TABLE_NAMES.len()
    );
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM pragma_table_info('entities') WHERE name = 'stale';"
        ),
        1,
        "entities.stale is restored"
    );
    assert!(store.get_entity("E1S1").unwrap().is_some());
    assert!(store.get_entity("sprint-1").unwrap().is_some());

    // AC 6: the rebuilt state equals a fresh sweep of the same tree.
    let rebuilt = dump_tables(&cache_db(root));
    let summary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 0, "the rebuilt cache is already current");
    let swept = dump_tables(&cache_db(root));
    for &table in ALL_TABLE_NAMES {
        assert_eq!(
            rebuilt.get(table),
            swept.get(table),
            "table '{table}' differs between the v1 rebuild and a fresh sweep"
        );
    }
}

#[test]
fn test_half_migrated_v1_cache_is_reported_as_mismatch() {
    // Running the v2 DDL over a v1 database creates `findings` and stamps the pragmas, but
    // `CREATE TABLE IF NOT EXISTS` cannot add `entities.stale`. Such a cache must never be
    // reported Valid, or every later sweep would fail on the missing column.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    drop(ensure_cache(root, &storage).unwrap());
    downgrade_cache_to_v1(root);

    let store = SqliteStore::open(cache_db(root)).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::store::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();
    drop(store);

    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM pragma_table_info('entities') WHERE name = 'stale';"
        ),
        0,
        "the half-migrated cache still lacks entities.stale"
    );
    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Mismatch,
        "a half-migrated cache must be rebuilt, not trusted"
    );

    // And a boot heals it.
    let store = ensure_cache(root, &storage).unwrap();
    assert!(store.get_entity("E1S1").unwrap().is_some());
    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Valid
    );
}

// ---------------------------------------------------------------------------
// Benchmark: median warm sweep <= 30 ms, every run <= 500 ms (AD-6)
// ---------------------------------------------------------------------------

#[test]
fn test_benchmark_warm_sweep_bound() {
    const N: usize = 1000;
    const WARM_SWEEPS: usize = 25;
    const MEDIAN_BUDGET_MS: u128 = 30;
    const CEILING_MS: u128 = 500;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    for i in 0..N {
        let id = format!("E1S{i}");
        write_story(root, &id, &format!("Story {id}"));
    }
    let storage = storage();

    // Warm-up: initial full rebuild.
    let store = ensure_cache(root, &storage).unwrap();

    // Repeated warm sweeps; time each.
    let mut durations_ms: Vec<u128> = Vec::with_capacity(WARM_SWEEPS);
    for _ in 0..WARM_SWEEPS {
        let start = Instant::now();
        let summary = store.sweep_workspace(root, &storage).unwrap();
        durations_ms.push(start.elapsed().as_millis());
        assert_eq!(summary.parsed, 0, "warm sweep must not re-parse");
        assert_eq!(summary.purged, 0, "warm sweep must not purge");
    }

    let mut sorted = durations_ms.clone();
    sorted.sort();
    let median = sorted[sorted.len() / 2];
    let max = *sorted.last().unwrap();

    eprintln!(
        "warm sweep benchmark (N={N}): median={median}ms max={max}ms runs={:?}",
        durations_ms
    );
    assert!(
        median <= MEDIAN_BUDGET_MS,
        "median warm sweep {median}ms exceeds {MEDIAN_BUDGET_MS}ms bound"
    );
    assert!(
        max <= CEILING_MS,
        "a warm sweep {max}ms exceeds the {CEILING_MS}ms regression ceiling"
    );
}

// ---------------------------------------------------------------------------
// Benchmark: 1,000 fixtures with one modified file (AC 1)
// ---------------------------------------------------------------------------

#[test]
fn test_benchmark_one_modified_sweep_bound() {
    const N: usize = 1000;
    const RUNS: usize = 5;
    const CEILING_MS: u128 = 500;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    for i in 0..N {
        let id = format!("E1S{i}");
        write_story(root, &id, &format!("Story {id}"));
    }
    let storage = storage();

    // Warm-up: initial full rebuild, then one warm sweep so only the edit is outstanding.
    let store = ensure_cache(root, &storage).unwrap();
    store.sweep_workspace(root, &storage).unwrap();

    let mut durations_ms: Vec<u128> = Vec::with_capacity(RUNS);
    for run in 0..RUNS {
        // Each run edits exactly one file, so exactly one file is hashed *and* re-parsed. The
        // title grows by a character per run: successive runs land in the same wall-clock
        // second, and mtime alone has 1-second granularity (spec Design Notes).
        write_story(
            root,
            "E1S500",
            &format!("Story E1S500 revision{}", "x".repeat(run + 1)),
        );

        let start = Instant::now();
        let summary = store.sweep_workspace(root, &storage).unwrap();
        durations_ms.push(start.elapsed().as_millis());

        assert_eq!(
            summary.parsed, 1,
            "exactly the modified file must be re-parsed"
        );
        assert_eq!(summary.purged, 0);
        assert_eq!(summary.unchanged, N, "every other file must be untouched");
    }

    let mut sorted = durations_ms.clone();
    sorted.sort();
    let median = sorted[sorted.len() / 2];
    let max = *sorted.last().unwrap();
    eprintln!(
        "one-modified sweep benchmark (N={N}): median={median}ms max={max}ms runs={durations_ms:?}"
    );

    assert_eq!(
        store.get_entity("E1S500").unwrap().unwrap().title.unwrap(),
        format!("Story E1S500 revision{}", "x".repeat(RUNS)),
        "the edit must be visible in the cache"
    );
    assert!(
        max <= CEILING_MS,
        "a one-modified sweep {max}ms exceeds the {CEILING_MS}ms regression ceiling"
    );
}

// ---------------------------------------------------------------------------
// sync_meta / last_synced_at (spec-1-12)
// ---------------------------------------------------------------------------

#[test]
fn test_get_last_synced_at_none_on_freshly_created_schema() {
    // A freshly created in-memory schema never goes through `sweep_workspace` or
    // `rebuild_from_workspace`, so `sync_meta` must be empty and reported as `None`, not an error.
    let store = SqliteStore::open_in_memory().unwrap();
    assert_eq!(store.get_last_synced_at().unwrap(), None);
}

#[test]
fn test_sync_meta_stamped_after_sweep() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    let storage = storage();

    let store = ensure_cache(root, &storage).unwrap();
    // `ensure_cache` already performed the initial rebuild/sweep, which must have stamped it.
    let first_stamp = store
        .get_last_synced_at()
        .unwrap()
        .expect("sync_meta must be stamped after the boot-time rebuild");

    // An explicit sweep (even a no-op one) refreshes the stamp.
    store.sweep_workspace(root, &storage).unwrap();
    let second_stamp = store
        .get_last_synced_at()
        .unwrap()
        .expect("sync_meta must remain stamped after an explicit sweep");
    assert!(
        second_stamp >= first_stamp,
        "the stamp must not go backwards across sweeps"
    );
}

#[test]
fn test_sync_meta_stamped_after_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    let storage = storage();

    let store = ensure_cache(root, &storage).unwrap();
    assert!(store.get_last_synced_at().unwrap().is_some());

    store.reset_and_rebuild(root, &storage).unwrap();
    assert!(
        store.get_last_synced_at().unwrap().is_some(),
        "sync_meta must be stamped inside reset_and_rebuild's own rebuild transaction"
    );
}

#[test]
fn test_sync_reports_summary_counts_matching_sweep_summary_shape() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    write_story(root, "E1S2", "Two");
    let storage = storage();

    let store = ensure_cache(root, &storage).unwrap();

    // Modify one file, leave the other untouched: a plain sync should report one parsed file and
    // the rest unchanged (the untouched story plus qdev.toml itself), matching the `SweepSummary`
    // shape used by `qdev sync`.
    write_story(root, "E1S1", "One modified");
    let summary: SweepSummary = store.sweep_workspace(root, &storage).unwrap();
    assert_eq!(summary.parsed, 1);
    assert_eq!(summary.unchanged, 2);
    assert_eq!(summary.purged, 0);

    // `--rebuild` drops and repopulates the cache from Markdown; counts must reflect the full
    // re-parse of both entity files plus qdev.toml itself, matching the same "successfully
    // parsed and upserted this pass" meaning `sweep_workspace` gives `parsed` (qdev.toml counts
    // there too, via `SweepFileRole::Config`).
    let rebuild_summary = store.reset_and_rebuild(root, &storage).unwrap();
    assert_eq!(rebuild_summary.parsed, 3);
    assert_eq!(rebuild_summary.unchanged, 0);
    assert_eq!(rebuild_summary.purged, 0);
}

/// Removes `sync_meta` from an otherwise-current cache, leaving every other table, the
/// `entities.stale` column, and both version pragmas exactly as a healthy cache has them. This
/// is the shape a cache written by a pre-1.12 binary has, and the *only* thing that
/// distinguishes it is the missing table.
fn drop_sync_meta(root: &Path) {
    let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
    conn.execute_batch("DROP TABLE IF EXISTS sync_meta;")
        .unwrap();
    // `user_version` is restamped to the current version so the stamp check passes and the
    // table-presence check is the only thing left that can catch the missing table — which is
    // exactly what this fixture exists to pin.
    conn.execute_batch(&format!("PRAGMA user_version = {};", CACHE_SCHEMA_VERSION))
        .unwrap();
}

/// A cache missing only `sync_meta` must be reported `Mismatch`, not `Valid`. The pragmas
/// cannot catch this on their own — the older binary stamped them to its own current version
/// after every rebuild, and sweeps issue DML only — so the table-presence check is the sole
/// detector. If it stopped covering `sync_meta`, `ensure_cache` would take its healthy-boot
/// branch and the sweep's `INSERT INTO sync_meta` would fail with `no such table`, leaving
/// every qdev command in that workspace broken until the cache was deleted by hand.
#[test]
fn test_cache_missing_only_sync_meta_is_reported_as_mismatch_and_healed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();
    drop(ensure_cache(root, &storage).unwrap());

    drop_sync_meta(root);
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='sync_meta';"
        ),
        0,
        "fixture must actually remove the table"
    );
    {
        let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
        let user_ver: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            user_ver, CACHE_SCHEMA_VERSION,
            "the fixture must look healthy to the version stamp, so that only the \
             table-presence check can catch this cache"
        );
    }
    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Mismatch,
        "a cache with no sync_meta table must never be reported Valid"
    );

    // Booting heals it: the table is back, stamped, and the entities survive the rebuild.
    let store = ensure_cache(root, &storage).unwrap();
    assert!(
        store.get_last_synced_at().unwrap().is_some(),
        "the healed cache must carry a sync stamp"
    );
    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Valid
    );
    assert_eq!(store.list_entities(&Default::default()).unwrap().len(), 2);
}

/// `qdev doctor` reads the schema version off the database rather than echoing the binary's
/// own constant, so it can actually tell a stale cache from a current one.
#[test]
fn test_cache_schema_introspection_reports_observed_state() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();

    assert_eq!(store.cache_schema_version().unwrap(), CACHE_SCHEMA_VERSION);
    assert!(store.cache_missing_tables().unwrap().is_empty());
    drop(store);

    // Stamp an older version and drop a table behind the store's back; both must be observed.
    let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
    conn.execute_batch("PRAGMA user_version = 1; DROP TABLE IF EXISTS sync_meta;")
        .unwrap();
    drop(conn);

    let store = SqliteStore::open(cache_db(root)).unwrap();
    assert_eq!(store.cache_schema_version().unwrap(), 1);
    assert_eq!(
        store.cache_missing_tables().unwrap(),
        vec!["sync_meta".to_string()]
    );
}

/// Opening a cache to write one entity must never mark an older database as current. The write
/// path calls `create_schema` on whatever cache the workspace has; `CREATE TABLE IF NOT EXISTS`
/// leaves an existing older table alone, so if that call also stamped the version pragmas, an
/// unmigrated cache would start reporting Valid — `ensure_cache` would skip the rebuild and
/// every later sweep would die on the missing column, with no recovery but deleting the file.
#[test]
fn test_writing_to_an_older_cache_does_not_stamp_it_as_current() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    drop(ensure_cache(root, &storage).unwrap());

    // An older cache: the current tables minus the column v3 added, stamped with the old version.
    {
        let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
        conn.execute_batch(
            "DROP TABLE findings;
             CREATE TABLE findings (
                 path TEXT NOT NULL,
                 code TEXT NOT NULL,
                 severity TEXT NOT NULL,
                 message TEXT,
                 found_at TEXT NOT NULL,
                 PRIMARY KEY (path, code)
             );
             PRAGMA user_version = 2;",
        )
        .unwrap();
    }

    let record = EntityRecord {
        id: "E1S1".to_string(),
        kind: qdev_core::EntityKind::Story,
        title: Some("Story".to_string()),
        status: Some("draft".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E1S1.md".to_string(),
        content_hash: "hash".to_string(),
        version: 2,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-09T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E1".to_string()),
        seq: Some(1),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    qdev_core::upsert_cache_and_mark_dirty(&cache_db(root), &record).unwrap();

    assert_ne!(
        rusqlite::Connection::open(cache_db(root))
            .unwrap()
            .query_row("PRAGMA user_version;", [], |r| r.get::<_, u32>(0))
            .unwrap(),
        CACHE_SCHEMA_VERSION,
        "a write must not stamp the current version onto tables it did not migrate"
    );
    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Mismatch,
        "the older cache must still be seen as needing a rebuild"
    );

    // And booting still heals it rather than dying on the missing column.
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(
        inspect_cache_schema(&cache_db(root)).unwrap(),
        CacheSchemaStatus::Valid
    );
    assert!(store.sweep_workspace(root, &storage).is_ok());
}

/// The doctor cache section reports what the database says, not what the binary was compiled
/// with. Exercised here rather than through the CLI: `ensure_cache` heals a mismatched cache at
/// boot, so no CLI-level test can ever reach the section with a stale one — which means no CLI
/// test can tell the observed read from an echo of the constant.
#[test]
fn test_doctor_cache_section_reports_a_stale_database() {
    use qdev_core::doctor::{CacheDoctorSection, DoctorSection};

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1"]);
    let storage = storage();
    drop(ensure_cache(root, &storage).unwrap());

    // Healthy first.
    let store = SqliteStore::open(cache_db(root)).unwrap();
    let fields = CacheDoctorSection.run(&store).unwrap().fields;
    let field = |name: &str| {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .unwrap_or_else(|| panic!("cache section must report '{name}'"))
            .1
            .clone()
    };
    assert_eq!(field("schema_status"), "ok");
    assert_eq!(field("cache_schema_version"), CACHE_SCHEMA_VERSION);
    assert_eq!(field("expected_cache_schema_version"), CACHE_SCHEMA_VERSION);
    drop(store);

    // Now a genuinely stale database: an older stamp and a missing table.
    {
        let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
        conn.execute_batch("PRAGMA user_version = 1; DROP TABLE IF EXISTS sync_meta;")
            .unwrap();
    }

    let store = SqliteStore::open(cache_db(root)).unwrap();
    let fields = CacheDoctorSection.run(&store).unwrap().fields;
    let field = |name: &str| {
        fields
            .iter()
            .find(|(k, _)| k == name)
            .unwrap_or_else(|| panic!("cache section must report '{name}'"))
            .1
            .clone()
    };
    assert_eq!(
        field("cache_schema_version"),
        1,
        "the section must report the version the database carries"
    );
    assert_eq!(field("expected_cache_schema_version"), CACHE_SCHEMA_VERSION);
    assert_eq!(field("schema_status"), "mismatch");
    assert_eq!(
        field("missing_tables"),
        serde_json::json!(["sync_meta"]),
        "a missing table must be named, not just counted"
    );
}

/// A file that was written through the write path and then deleted must still be purged.
///
/// The write path deletes the entity's `sync_state` row to invalidate it, and purge used to
/// discover removed files only through that table — so a write-then-delete left the entity in
/// the cache permanently, still queryable, with nothing on disk behind it. Deleting a file that
/// was *not* written first was purged correctly, which is why no existing test caught this.
#[test]
fn test_entity_written_then_deleted_is_still_purged() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    make_workspace(root, &["E1S1", "E1S2"]);
    let storage = storage();
    let store = ensure_cache(root, &storage).unwrap();
    assert_eq!(store.list_entities(&Default::default()).unwrap().len(), 2);

    // Write through the write path, which drops the file's sync_state row.
    let record = EntityRecord {
        id: "E1S2".to_string(),
        kind: qdev_core::EntityKind::Story,
        title: Some("Story".to_string()),
        status: Some("ready".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E1S2.md".to_string(),
        content_hash: "rewritten".to_string(),
        version: 2,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-09T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E1".to_string()),
        seq: Some(2),
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    qdev_core::upsert_cache_and_mark_dirty(&cache_db(root), &record).unwrap();
    assert_eq!(
        count_rows(
            root,
            "SELECT COUNT(*) FROM sync_state WHERE path = 'docs/specs/stories/E1S2.md';"
        ),
        0,
        "the write path must have invalidated the sync_state row (fixture precondition)"
    );

    std::fs::remove_file(story_path(root, "E1S2")).unwrap();
    let summary = store.sweep_workspace(root, &storage).unwrap();

    assert_eq!(summary.purged, 1, "the deleted file must be purged");
    let remaining: Vec<String> = store
        .list_entities(&Default::default())
        .unwrap()
        .into_iter()
        .map(|e| e.id)
        .collect();
    assert_eq!(
        remaining,
        vec!["E1S1".to_string()],
        "a written-then-deleted entity must not survive as a ghost"
    );
    assert!(store.get_entity("E1S2").unwrap().is_none());
}

/// The rebuild grew a `read_error` for four read sites, but only the entity-markdown one was
/// reachable by any test: `make_unreadable` always wrote a story. Reverting the scratch or
/// evidence arm to `Err(_) => continue` left the suite green, so an unreadable scratchpad or
/// evidence file could still be swallowed by a rebuild — the divergence this story removes.
#[test]
fn test_unreadable_scratch_and_evidence_files_record_read_error_on_both_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root, "");
    write_story(root, "E1S1", "One");
    let storage = storage();

    let scratch_rel = "docs/state/scratch/E1S1.jsonl";
    let evidence_rel = "docs/state/evidence/run-1.json";
    if !make_unreadable_with(root, scratch_rel, "{\"seq\":1,\"text\":\"note\"}\n")
        || !make_unreadable_with(root, evidence_rel, "{\"id\":\"run-1\"}\n")
    {
        eprintln!("skipping: this environment can read a 0o000 file");
        return;
    }

    // Rebuild path: the rebuild truncates `findings` first, so a swallowed read leaves nothing
    // behind even though the cache this starts from was built by a sweep.
    let store = ensure_cache(root, &storage).unwrap();
    store.rebuild_from_workspace(root, &storage).unwrap();
    let rebuild_paths: Vec<String> = store
        .list_findings()
        .unwrap()
        .into_iter()
        .filter(|f| f.code == "read_error")
        .map(|f| f.path)
        .collect();
    assert!(
        rebuild_paths.iter().any(|p| p == scratch_rel),
        "rebuild must record a read_error for the unreadable scratch file: {rebuild_paths:?}"
    );
    assert!(
        rebuild_paths.iter().any(|p| p == evidence_rel),
        "rebuild must record a read_error for the unreadable evidence file: {rebuild_paths:?}"
    );

    // Sweep path: the same two findings after a plain sweep over the same tree.
    let swept = ensure_cache(root, &storage).unwrap();
    swept.sweep_workspace(root, &storage).unwrap();
    let sweep_paths: Vec<String> = swept
        .list_findings()
        .unwrap()
        .into_iter()
        .filter(|f| f.code == "read_error")
        .map(|f| f.path)
        .collect();
    for rel in [scratch_rel, evidence_rel] {
        assert!(
            sweep_paths.iter().any(|p| p == rel),
            "sweep must record a read_error for {rel}: {sweep_paths:?}"
        );
    }
}
