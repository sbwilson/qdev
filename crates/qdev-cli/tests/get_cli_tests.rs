use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
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

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Writes the cli-reference.md `get` JSON envelope example fixture: epic E12 (with an inherited
/// rabbit_hole constraint), story E12S4 (with an own no_go constraint and three relations), and
/// story E12S3 as the `depends_on` target.
fn write_reference_fixture(root: &Path, e12s3_status: &str) {
    write_file(
        root,
        "docs/specs/epics/E12.md",
        r#"---
id: E12
title: "Bridge Layer"
status: active
version: 1
constraints:
  - id: RH-2
    kind: rabbit_hole
    text: "len == 0 does not mean empty result"
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Goal
"#,
    );

    write_file(
        root,
        "docs/specs/stories/E12S4.md",
        r#"---
id: E12S4
title: "CoreResponse Buffer Layout"
status: ready
version: 3
owners: ["simon", "team:core-platform"]
epic_id: E12
appetite: small
safety_class: ClassB
target_modules: ["bridge", "foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: "Do not implement Swift decoding"
relations:
  depends_on: ["E12S3"]
  traces_to: ["FR-102"]
  governed_by: ["AD-43"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Buffers round-trip.
"#,
    );

    write_file(
        root,
        "docs/specs/stories/E12S3.md",
        &format!(
            r#"---
id: E12S3
title: "Prior Story"
status: {}
version: 1
epic_id: E12
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Done.
"#,
            e12s3_status
        ),
    );
}

#[test]
fn test_get_story_json_matches_reference_shape() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_slice(&output.stdout).unwrap();

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["id"], "E12S4");
    assert_eq!(val["kind"], "story");
    assert_eq!(val["epic_id"], "E12");
    assert_eq!(val["title"], "CoreResponse Buffer Layout");
    assert_eq!(val["status"], "ready");
    assert_eq!(val["blocked"], false);
    assert_eq!(val["appetite"], "small");
    assert_eq!(val["safety_class"], "ClassB");
    assert_eq!(
        val["owners"],
        serde_json::json!(["simon", "team:core-platform"])
    );
    assert_eq!(
        val["target_modules"],
        serde_json::json!(["bridge", "foundation"])
    );
    assert_eq!(val["version"], 3);

    let constraints = val["constraints"].as_array().unwrap();
    assert_eq!(constraints.len(), 2);
    assert_eq!(constraints[0]["id"], "E12S4/NG-1");
    assert_eq!(constraints[0]["kind"], "no_go");
    assert!(constraints[0].get("inherited_from").is_none());
    assert_eq!(constraints[1]["id"], "E12/RH-2");
    assert_eq!(constraints[1]["kind"], "rabbit_hole");
    assert_eq!(constraints[1]["inherited_from"], "E12");

    assert_eq!(val["relations"]["depends_on"], serde_json::json!(["E12S3"]));
    assert_eq!(val["relations"]["traces_to"], serde_json::json!(["FR-102"]));
    assert_eq!(
        val["relations"]["governed_by"],
        serde_json::json!(["AD-43"])
    );

    // Never fields per spec boundaries.
    assert!(val.get("gates").is_none(), "no gates field is modeled yet");
    assert!(
        val.get("scratch").is_none(),
        "scratch must be absent without --expand scratch"
    );
}

#[test]
fn test_get_story_json_byte_identical_across_runs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    let out1 = cmd1
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let out2 = cmd2
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(out1, out2);
}

#[test]
fn test_get_blocked_true_when_dependency_not_done() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "in-progress");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["blocked"], true);
}

#[test]
fn test_get_expand_scratch_adds_section_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");
    write_file(
        root,
        "docs/state/scratch/E12S4.jsonl",
        r#"{"seq": 1, "at": "2026-09-08T00:00:00Z", "author": {"type": "human", "id": "simon"}, "kind": "note", "text": "Investigated the buffer layout"}
"#,
    );

    let mut without_expand_cmd = Command::cargo_bin("qdev").unwrap();
    let without_expand = without_expand_cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success();
    let mut without_val: Value =
        serde_json::from_slice(&without_expand.get_output().stdout).unwrap();
    assert!(without_val.get("scratch").is_none());

    let mut with_expand_cmd = Command::cargo_bin("qdev").unwrap();
    let with_expand = with_expand_cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "scratch", "--json"])
        .assert()
        .success();
    let mut with_val: Value = serde_json::from_slice(&with_expand.get_output().stdout).unwrap();

    let scratch = with_val["scratch"].as_array().expect("scratch present");
    assert_eq!(scratch.len(), 1);
    assert_eq!(scratch[0]["text"], "Investigated the buffer layout");

    // Nothing else changes: strip `scratch` from both and compare.
    with_val.as_object_mut().unwrap().remove("scratch");
    without_val.as_object_mut().unwrap().remove("scratch");
    assert_eq!(with_val, without_val);
}

#[test]
fn test_get_expand_relations_and_constraints_are_accepted_no_ops() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "get",
            "story",
            "E12S4",
            "--expand",
            "relations,constraints",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_get_expand_unknown_value_is_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "bogus", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_get_bare_id_resolves_without_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_file(
        root,
        "docs/specs/adrs/AD-43.md",
        r#"---
id: AD-43
title: "Use C ABI"
status: accepted
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Decision
"#,
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "AD-43", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["id"], "AD-43");
    assert_eq!(val["kind"], "adr");
}

#[test]
fn test_get_constraint_slash_form_returns_constraint_object() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "E12S4/NG-1", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["id"], "E12S4/NG-1");
    assert_eq!(val["owner_id"], "E12S4");
    assert_eq!(val["kind"], "no_go");
    assert_eq!(val["text"], "Do not implement Swift decoding");
    // A constraint payload is not an entity projection: no `blocked`/`relations` fields.
    assert!(val.get("blocked").is_none());
    assert!(val.get("relations").is_none());
}

#[test]
fn test_get_unknown_id_exit_2_entity_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E99S1", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "entity_not_found");
}

#[test]
fn test_get_kind_mismatch_exit_2_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "epic", "E12S4", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_get_missing_id_for_kind_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["get", "story"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Missing entity ID"));
}

#[test]
fn test_get_text_mode_compact_block() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4"])
        .assert()
        .success()
        .code(0);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("id: E12S4"));
    assert!(stdout.contains("blocked: false"));
    assert!(stdout.contains("constraints:"));
    assert!(stdout.contains("E12S4/NG-1"));
    assert!(stdout.contains("relations:"));
    assert!(stdout.contains("depends_on: E12S3"));
}

#[test]
fn test_get_text_mode_scratch_with_embedded_newline_stays_one_line_per_entry() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");
    write_file(
        root,
        "docs/state/scratch/E12S4.jsonl",
        "{\"seq\": 1, \"at\": \"2026-09-08T00:00:00Z\", \"author\": {\"type\": \"human\", \"id\": \"simon\"}, \"kind\": \"note\", \"text\": \"Line one\\nLine two\"}\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "scratch"])
        .assert()
        .success()
        .code(0);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let entry_lines: Vec<&str> = stdout
        .lines()
        .filter(|l| l.trim_start().starts_with("[1]"))
        .collect();
    assert_eq!(
        entry_lines.len(),
        1,
        "the embedded newline must not split the scratch entry across two lines: {}",
        stdout
    );
    assert!(entry_lines[0].contains("Line one Line two"));
}

#[test]
fn test_get_outside_workspace_is_clean_usage_error_and_creates_no_cache() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // Deliberately skip setup_workspace: no `qdev init` has run here.

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E1S1", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(
        !root.join(".qdev/cache/cache.sqlite").exists(),
        "qdev get must not create a cache file outside an initialized workspace"
    );
}

#[test]
fn test_get_story_with_stale_row_surfaces_stale_true() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    // Trigger an initial hydration so the cache actually has a row for E12S4 before the raw
    // SQL UPDATE below (otherwise it would be a silent no-op on a nonexistent row).
    let mut hydrate_cmd = Command::cargo_bin("qdev").unwrap();
    hydrate_cmd
        .current_dir(root)
        .args(["status", "--json"])
        .assert()
        .success();

    // Force the story's cache row stale directly, simulating the state hydration leaves behind
    // when a source file currently fails to parse (story 1.7's stale-retention behavior).
    let db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = qdev_core::rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("UPDATE entities SET stale = 1 WHERE id = 'E12S4';", [])
        .unwrap();
    drop(conn);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["stale"], true);
}

#[test]
fn test_get_text_mode_constraint_block() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_reference_fixture(root, "done");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "E12S4/NG-1"])
        .assert()
        .success()
        .code(0);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("id: E12S4/NG-1"));
    assert!(stdout.contains("owner: E12S4"));
    assert!(stdout.contains("kind: no_go"));
}
