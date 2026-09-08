//! E2E contract for the incremental hydration sweep (spec-1-7): a normal `qdev` command boots
//! the sweep, so hand edits / additions / removals reach the cache, findings are recorded, and a
//! contended write lock fails boot with exit 5.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use assert_cmd::Command;
use tempfile::TempDir;

use qdev_core::rusqlite;

fn init_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "SweepCli",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success()
        .code(0);
}

fn story_path(root: &Path, id: &str) -> PathBuf {
    root.join("docs/specs/stories").join(format!("{id}.md"))
}

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

fn write_story(root: &Path, id: &str, title: &str) {
    let path = story_path(root, id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, story_md(id, title)).unwrap();
}

fn cache_db(root: &Path) -> PathBuf {
    root.join(".qdev/cache/cache.sqlite")
}

fn run_status(root: &Path) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["status"])
        .assert()
        .success()
        .code(0)
}

fn entity_title(root: &Path, id: &str) -> Option<String> {
    rusqlite::Connection::open(cache_db(root))
        .ok()?
        .query_row(
            "SELECT title FROM entities WHERE id = ?1;",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok()
}

fn entity_exists(root: &Path, id: &str) -> bool {
    let n: i64 = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM entities WHERE id = ?1;",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .unwrap();
    n > 0
}

fn findings_for(root: &Path, path: &str) -> Vec<(String, String)> {
    let conn = rusqlite::Connection::open(cache_db(root)).unwrap();
    let mut stmt = conn
        .prepare("SELECT code, severity FROM findings WHERE path = ?1 ORDER BY code;")
        .unwrap();
    let rows = stmt
        .query_map(rusqlite::params![path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    rows.filter_map(|r| r.ok()).collect()
}

/// Boots the workspace with two stories already visible in the cache.
fn seeded_workspace(root: &Path) {
    init_workspace(root);
    write_story(root, "E1S1", "Story E1S1");
    write_story(root, "E1S2", "Story E1S2");
    // First status boot sweeps the new files into the cache.
    run_status(root);
    assert!(entity_exists(root, "E1S1"));
    assert!(entity_exists(root, "E1S2"));
}

#[test]
fn test_sweep_picks_up_hand_edits_additions_and_removals() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    seeded_workspace(root);

    // Hand edit E1S1, add E1S3, remove E1S2.
    fs::write(
        story_path(root, "E1S1"),
        story_md("E1S1", "Hand Edited Title"),
    )
    .unwrap();
    write_story(root, "E1S3", "Newly Added");
    fs::remove_file(story_path(root, "E1S2")).unwrap();

    // A normal command boots the sweep and syncs the cache.
    run_status(root);

    assert_eq!(
        entity_title(root, "E1S1"),
        Some("Hand Edited Title".to_string()),
        "modified file visible in cache"
    );
    assert!(entity_exists(root, "E1S3"), "added file visible in cache");
    assert!(!entity_exists(root, "E1S2"), "removed file gone from cache");
}

#[test]
fn test_sweep_records_findings_without_failing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    seeded_workspace(root);

    // Make E1S1 a merge conflict and E1S2 schema-invalid.
    fs::write(
        story_path(root, "E1S1"),
        story_md("E1S1", "Story E1S1") + "<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> branch\n",
    )
    .unwrap();
    fs::write(
        story_path(root, "E1S2"),
        story_md("E1S2", "Story E1S2").replace("version: 1", "version: nope"),
    )
    .unwrap();

    // The command must still succeed (findings are non-fatal).
    run_status(root);

    assert_eq!(
        findings_for(root, "docs/specs/stories/E1S1.md"),
        vec![("merge_conflict".to_string(), "error".to_string())]
    );
    assert_eq!(
        findings_for(root, "docs/specs/stories/E1S2.md"),
        vec![("schema_violation".to_string(), "error".to_string())]
    );
    // Previous rows retained and flagged stale.
    let stale_e1: i64 = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row("SELECT stale FROM entities WHERE id = 'E1S1';", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stale_e1, 1, "conflicted entity retained stale");
    let stale_e2: i64 = rusqlite::Connection::open(cache_db(root))
        .unwrap()
        .query_row("SELECT stale FROM entities WHERE id = 'E1S2';", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(stale_e2, 1, "violating entity retained stale");
}

#[test]
fn test_lock_contention_fails_boot_with_exit_5() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    seeded_workspace(root);

    // Hold the advisory write lock in this process; the child `qdev` (separate process) must
    // block on it and time out with a Conflict (exit 5).
    let lock_path = root.join(".qdev/cache/write.lock");
    let guard = qdev_core::write::acquire_write_lock(&lock_path, Duration::from_secs(20)).unwrap();

    let output = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["status"])
        .output()
        .unwrap();

    // Release the lock (child has already timed out by now).
    drop(guard);

    assert_eq!(
        output.status.code(),
        Some(5),
        "contended boot must fail exit 5 (lock_timeout); stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("lock_timeout"),
        "stderr should carry the lock_timeout code"
    );
}
