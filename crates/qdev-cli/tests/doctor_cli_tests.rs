//! `qdev doctor` CLI tests (spec-1-12): cache section fields present, `--json` shape, and the
//! uninitialized-workspace usage error.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "TestProject",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();
}

fn write_story(dir: &Path, id: &str, title: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_doctor_json_shape_and_exit_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["schema_version"], "1");

    let sections = val["sections"].as_array().unwrap();
    let cache = sections
        .iter()
        .find(|s| s["name"] == "cache")
        .expect("a 'cache' doctor section must be present");

    assert!(cache["cache_schema_version"].is_number());
    assert!(cache["entity_count"].is_number());
    assert!(cache["finding_count"].is_number());
    assert!(cache.get("last_synced_at").is_some());
}

#[test]
fn test_doctor_healthy_cache_reports_entity_and_finding_counts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_story(&stories_dir, "E1S2", "Two");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let cache = sections.iter().find(|s| s["name"] == "cache").unwrap();

    // Boot-time hydration (spec-1-6/1-7) has already parsed both new story files by the time
    // `doctor` reads the cache.
    assert_eq!(cache["entity_count"], 2);
    assert_eq!(cache["finding_count"], 0);
    assert!(
        cache["last_synced_at"].is_string(),
        "a synced cache must report a non-null last_synced_at: {}",
        cache
    );
}

#[test]
fn test_doctor_text_output_contains_cache_section() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["doctor"]).assert().success();

    let output = assert.get_output();
    let text = std::str::from_utf8(&output.stdout).unwrap();
    assert!(text.contains("[cache]"), "text output: {}", text);
    assert!(text.contains("schema_version"), "text output: {}", text);
    assert!(text.contains("entity_count"), "text output: {}", text);
    assert!(text.contains("last_synced_at"), "text output: {}", text);
    assert!(text.contains("finding_count"), "text output: {}", text);
}

#[test]
fn test_doctor_uninitialized_workspace_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_doctor_uninitialized_workspace_text_mode_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(
        stderr.contains("usage_error"),
        "text-mode error output should be usage-error-shaped: {}",
        stderr
    );
}

#[test]
fn test_doctor_reports_nonzero_finding_count_for_a_seeded_validation_finding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // A merge-conflict marker is never parsed and records a `merge_conflict` finding for the
    // file's path (see `hydrate_markdown_file` in qdev-core's sqlite store).
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("E1S1.md"),
        r#"---
id: E1S1
title: "One"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

<<<<<<< HEAD
## Acceptance Criteria
- AC.
=======
## Acceptance Criteria
- Other AC.
>>>>>>> branch
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let cache = sections.iter().find(|s| s["name"] == "cache").unwrap();

    assert!(
        cache["finding_count"].as_u64().unwrap_or(0) > 0,
        "expected a non-zero finding_count for a seeded merge-conflict finding: {}",
        cache
    );
}

/// The cache section reports the version the database actually carries alongside the one this
/// binary expects. Echoing the compiled-in constant back — as it did — made the one diagnostic
/// `qdev doctor` ships structurally incapable of detecting the stale or half-migrated cache it
/// exists to find.
#[test]
fn test_doctor_reports_observed_schema_state() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(&root.join("docs/specs/stories"), "E1S1", "Story one");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let cache = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "cache")
        .expect("a cache section");

    assert_eq!(cache["schema_status"], "ok");
    assert_eq!(cache["missing_tables"].as_array().unwrap().len(), 0);
    assert_eq!(
        cache["cache_schema_version"], cache["expected_cache_schema_version"],
        "a healthy cache reports the version it was built for"
    );
    assert!(
        cache["schema_version"].is_null(),
        "the section must not shadow the envelope's string schema_version"
    );
}

/// A cache stamped with an older version is healed by the boot-time `ensure_cache` rebuild
/// before any command's handler runs, so `qdev doctor` reports it as `ok` — the rebuild is what
/// makes that true, and `schema_status` is what would catch a rebuild that failed to stamp.
/// (The store-level observation itself is covered by `sweep_tests`.)
#[test]
fn test_doctor_reports_ok_after_a_stale_cache_is_healed_at_boot() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(&root.join("docs/specs/stories"), "E1S1", "Story one");

    let mut prime = Command::cargo_bin("qdev").unwrap();
    prime
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    {
        let conn =
            qdev_core::rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 1;").unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let cache = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "cache")
        .expect("a cache section");

    assert_eq!(
        cache["schema_status"], "ok",
        "the boot rebuild must have restamped the cache before doctor read it"
    );
    assert_eq!(
        cache["cache_schema_version"],
        cache["expected_cache_schema_version"]
    );
    assert_eq!(
        cache["entity_count"], 1,
        "the rebuild must not have lost the workspace's entities"
    );
}
