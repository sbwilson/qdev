//! `qdev relate` / `qdev unrelate` CLI tests (spec-1-10), covering the I/O & edge-case matrix.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use qdev_core::rusqlite;
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

fn create_story(root: &Path, id: &str, relations_yaml: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    let relations_block = if relations_yaml.is_empty() {
        String::new()
    } else {
        format!("relations:\n{relations_yaml}")
    };
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{relations_block}---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

fn create_epic(root: &Path, id: &str) {
    let dir = root.join("docs/specs/epics");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Epic {id}"
status: planning
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Summary
- Summary.
"#
        ),
    )
    .unwrap();
}

fn read_story(root: &Path, id: &str) -> String {
    fs::read_to_string(root.join("docs/specs/stories").join(format!("{}.md", id))).unwrap()
}

fn relation_row_count(root: &Path, source: &str, relation: &str, target: &str) -> i64 {
    let conn = rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM relations WHERE source_id = ?1 AND relation = ?2 AND target_id = ?3;",
        rusqlite::params![source, relation, target],
        |row| row.get(0),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn test_relate_happy_path_patches_frontmatter_and_updates_cache() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("valid JSON on stdout");
    assert_eq!(val["id"], "E1S2");
    assert_eq!(val["relation"], "depends_on");
    assert_eq!(val["target_id"], "E1S1");
    assert_eq!(val["changed"], true);

    let content = read_story(root, "E1S2");
    assert!(content.contains("relations:"));
    assert!(content.contains("depends_on:"));
    assert!(content.contains("- E1S1") || content.contains("E1S1"));
    assert!(content.contains("version: 2\n"));

    // A second CLI invocation boots via ensure_cache and re-syncs the relations table.
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root).args(["status"]).assert().success();
    assert_eq!(relation_row_count(root, "E1S2", "depends_on", "E1S1"), 1);
}

#[test]
fn test_relate_merges_into_existing_relations_map_without_dropping_other_entries() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S9", "");
    create_story(root, "E1S2", "  depends_on: [\"E1S9\"]\n");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1"])
        .assert()
        .success()
        .code(0);

    let content = read_story(root, "E1S2");
    assert!(content.contains("E1S9"), "existing target must survive");
    assert!(content.contains("E1S1"), "new target must be added");
}

// ---------------------------------------------------------------------------
// Refusals (exit 1, no write)
// ---------------------------------------------------------------------------

#[test]
fn test_relate_refuses_dangling_target() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S9", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["error"]["code"], "dangling_relation");

    let content = read_story(root, "E1S2");
    assert!(!content.contains("relations:"), "no write must occur");
}

#[test]
fn test_relate_refuses_wrong_kind_pair() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_epic(root, "E1");

    // traces_to must target a Requirement, not an Epic.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S1", "traces_to", "E1", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["error"]["code"], "invalid_relation_kind");

    let content = read_story(root, "E1S1");
    assert!(!content.contains("relations:"), "no write must occur");
}

#[test]
fn test_relate_refuses_would_be_cycle() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "  depends_on: [\"E1S2\"]\n");
    create_story(root, "E1S2", "");

    // E1S1 already depends_on E1S2; relating E1S2 depends_on E1S1 would close a cycle.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["error"]["code"], "dependency_cycle");

    let content = read_story(root, "E1S2");
    assert!(!content.contains("relations:"), "no write must occur");
}

// ---------------------------------------------------------------------------
// Unrelate
// ---------------------------------------------------------------------------

#[test]
fn test_unrelate_removes_present_entry() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(
        root,
        "E1S2",
        "  depends_on: [\"E1S1\"]\n  traces_to: [\"FR-1\"]\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["unrelate", "E1S2", "depends_on", "E1S1"])
        .assert()
        .success()
        .code(0);

    let content = read_story(root, "E1S2");
    assert!(
        !content.contains("depends_on"),
        "the emptied relation key must be dropped"
    );
    assert!(
        content.contains("traces_to"),
        "other relations must survive"
    );
    assert!(content.contains("version: 2\n"));
}

#[test]
fn test_unrelate_absent_is_idempotent_noop_exit_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["unrelate", "E1S2", "depends_on", "E1S1", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["changed"], false);

    let content = read_story(root, "E1S2");
    assert!(
        content.contains("version: 1\n"),
        "no-op must not bump the version"
    );
}

// ---------------------------------------------------------------------------
// compute_blocked integration (existing behavior, unchanged, exercised via relate)
// ---------------------------------------------------------------------------

#[test]
fn test_blocked_true_for_undone_depends_on_target_after_relate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1"])
        .assert()
        .success();

    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = get_cmd
        .current_dir(root)
        .args(["get", "story", "E1S2", "--json"])
        .assert()
        .success();

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["blocked"], true);
}

// ---------------------------------------------------------------------------
// Duplicate relate (idempotency of the add path)
// ---------------------------------------------------------------------------

#[test]
fn test_relate_same_edge_twice_is_idempotent_noop() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1"])
        .assert()
        .success();

    // Second, identical relate call: must be a no-op (changed: false, version unchanged, and no
    // duplicate target written to the `depends_on` array).
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2
        .current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["changed"], false);
    assert_eq!(val["version"], 2);

    let content = read_story(root, "E1S2");
    assert!(
        content.contains("version: 2\n"),
        "no-op second relate must not bump the version"
    );
    let occurrences = content.matches("E1S1").count();
    assert_eq!(
        occurrences, 1,
        "target must appear exactly once, not duplicated: {}",
        content
    );
}

// ---------------------------------------------------------------------------
// Relation key order preservation (relate/unrelate rewrite the whole `relations:` block)
// ---------------------------------------------------------------------------

#[test]
fn test_relate_preserves_existing_relation_key_order() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S9", "");
    create_story(root, "E1S1", "");
    // Deliberately out of alphabetical order: traces_to, then depends_on.
    create_story(
        root,
        "E1S2",
        "  traces_to: [\"FR-1\"]\n  depends_on: [\"E1S9\"]\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["relate", "E1S2", "extends", "E1S1"])
        .assert()
        .success();

    let content = read_story(root, "E1S2");
    let traces_pos = content.find("traces_to:").expect("traces_to key present");
    let depends_pos = content.find("depends_on:").expect("depends_on key present");
    assert!(
        traces_pos < depends_pos,
        "original relation key order (traces_to before depends_on) must survive a relate call on a third relation:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// --if-version, and the relations table on the write path
// ---------------------------------------------------------------------------

/// `--if-version` is optimistic concurrency control: a stale version must refuse the write.
/// The check lives in `patch_frontmatter` and is exercised through `qdev update`, so nothing
/// observed whether `qdev relate` actually plumbed the flag through to it.
#[test]
fn test_relate_with_stale_if_version_is_refused_and_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let before = fs::read_to_string(root.join("docs/specs/stories/E1S1.md")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "relate",
            "E1S1",
            "depends_on",
            "E1S2",
            "--if-version",
            "99",
            "--json",
        ])
        .assert()
        .failure()
        .code(5);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "version_mismatch");

    assert_eq!(
        fs::read_to_string(root.join("docs/specs/stories/E1S1.md")).unwrap(),
        before,
        "a version conflict must leave the source file untouched"
    );
}

#[test]
fn test_relate_with_matching_if_version_succeeds() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2", "--if-version", "1"])
        .assert()
        .success();

    assert!(fs::read_to_string(root.join("docs/specs/stories/E1S1.md"))
        .unwrap()
        .contains("depends_on"));
}

/// A `relate` that would close a cycle created by an earlier `relate` is refused. This runs the
/// two commands as separate processes, so the second one's boot sweep re-hydrates the graph from
/// frontmatter either way — the in-transaction relation write is covered at the library level by
/// `write_tests::test_upsert_cache_with_relation_writes_the_relation_row`, which is the only
/// place a second operation can observe the first without an intervening sweep.
#[test]
fn test_relate_refuses_a_cycle_closed_by_an_earlier_relate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut first = Command::cargo_bin("qdev").unwrap();
    first
        .current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .success();

    // The reverse edge closes the cycle the first call created.
    let mut second = Command::cargo_bin("qdev").unwrap();
    second
        .current_dir(root)
        .args(["relate", "E1S2", "depends_on", "E1S1"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("dependency_cycle"));

    assert!(
        !fs::read_to_string(root.join("docs/specs/stories/E1S2.md"))
            .unwrap()
            .contains("depends_on"),
        "the refused edge must not have been written"
    );
}

/// A malformed `relations:` block is refused rather than silently rewritten away. The write
/// path reconstructs the whole block from the parsed value, so defaulting an unexpected shape
/// to an empty map deletes every existing edge and still reports success.
#[test]
fn test_relate_refuses_a_relations_block_that_is_not_a_mapping() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S2", "");

    let dir = root.join("docs/specs/stories");
    fs::write(
        dir.join("E1S1.md"),
        r#"---
id: E1S1
title: "Story E1S1"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
relations:
  - depends_on
---

## Acceptance Criteria
- AC.
"#,
    )
    .unwrap();
    let before = fs::read_to_string(dir.join("E1S1.md")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .failure();

    assert_eq!(
        fs::read_to_string(dir.join("E1S1.md")).unwrap(),
        before,
        "an unsupported relations shape must not be rewritten away"
    );
}

/// `relate` adopts hydration's kind rule too, and that half was pinned by nothing: every fixture
/// in this file is written into the directory matching its kind, so the frontmatter kind and the
/// directory kind always agree and the rule cannot be observed. Deleting the `kind_for_write`
/// call from `apply_relation_change` left the whole suite green.
///
/// Reaching the write path with a kind disagreement takes two steps, because the ADR schema is
/// strictly more permissive than the story schema: the file must first hydrate while it is still
/// valid, so that the later story-invalid edit leaves the previous row in the cache (marked
/// stale) for `relate`'s precheck to find, instead of the entity being absent entirely.
#[test]
fn test_relate_validates_against_frontmatter_kind_not_the_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    create_story(root, "E1S5", "");

    // In the ADR directory, declaring `kind: story`, and valid under both schemas for now.
    let adrs = root.join("docs/specs/adrs");
    fs::create_dir_all(&adrs).unwrap();
    let adr_path = adrs.join("AD-9.md");
    let adr = |appetite: &str| {
        format!(
            r#"---
id: AD-9
kind: story
title: Buffer layout decision
status: draft
version: 1
{appetite}created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Decision
- Chosen.
"#
        )
    };
    fs::write(&adr_path, adr("")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["status"]).assert().success();

    // Now make it invalid under the story schema only. `appetite` is an enum there and a
    // free-form extra field under the ADR schema, so the two schemas disagree about this file.
    fs::write(&adr_path, adr("appetite: bogus\n")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "AD-9", "depends_on", "E1S5", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        val["error"]["code"], "schema_validation_failed",
        "relate must judge the file by its frontmatter kind, as the next sweep will: {val}"
    );

    // Refused before the write: the file is unchanged.
    let content = fs::read_to_string(&adr_path).unwrap();
    assert!(
        !content.contains("depends_on"),
        "a refused relate must write nothing: {content}"
    );
}

/// The fence has to hold on the path that reports no change, which is where it did not: the
/// write path returned `Ok(changed: false)` for an edge that was already there before
/// `--if-version` was ever compared, so exit 0 confirmed a version nothing had looked at.
#[test]
fn test_relate_with_stale_if_version_on_an_existing_edge_is_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut first = Command::cargo_bin("qdev").unwrap();
    first
        .current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .success();
    let before = read_story(root, "E1S1");
    assert!(before.contains("version: 2\n"));

    // The edge is already present, so this relate would be an idempotent no-op — but the
    // expected version is stale, and the fence is compared first.
    let mut second = Command::cargo_bin("qdev").unwrap();
    let assert = second
        .current_dir(root)
        .args([
            "relate",
            "E1S1",
            "depends_on",
            "E1S2",
            "--if-version",
            "99",
            "--json",
        ])
        .assert()
        .failure()
        .code(5);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "version_mismatch");
    assert_eq!(val["error"]["details"]["expected_version"], 99);
    assert_eq!(val["error"]["details"]["current_version"], 2);

    assert_eq!(
        read_story(root, "E1S1"),
        before,
        "a fenced no-op must leave the file untouched"
    );
}

/// `--if-version` on `unrelate` means what it means on `relate` and `update`, on the path that
/// removes an edge.
#[test]
fn test_unrelate_with_stale_if_version_is_refused_and_the_edge_survives() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "  depends_on: [\"E1S1\"]\n");
    let before = read_story(root, "E1S2");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "unrelate",
            "E1S2",
            "depends_on",
            "E1S1",
            "--if-version",
            "99",
            "--json",
        ])
        .assert()
        .failure()
        .code(5);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "version_mismatch");

    assert_eq!(
        read_story(root, "E1S2"),
        before,
        "a refused unrelate must leave the edge and the file alone"
    );
}

/// The other no-op path: `unrelate` of an absent edge. The removal would change nothing, so the
/// comparison is the only thing the caller can be told — and it must still happen.
#[test]
fn test_unrelate_absent_edge_with_stale_if_version_is_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "unrelate",
            "E1S2",
            "depends_on",
            "E1S1",
            "--if-version",
            "99",
            "--json",
        ])
        .assert()
        .failure()
        .code(5);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "version_mismatch");
}

#[test]
fn test_unrelate_with_matching_if_version_removes_the_edge() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "  depends_on: [\"E1S1\"]\n");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "unrelate",
            "E1S2",
            "depends_on",
            "E1S1",
            "--if-version",
            "1",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["changed"], true);
    assert_eq!(val["version"], 2);

    let content = read_story(root, "E1S2");
    assert!(!content.contains("depends_on"), "the edge must be gone");
    assert!(content.contains("version: 2\n"));
}

// ---------------------------------------------------------------------------
// Relation names: an unknown one is a usage error from both commands
// ---------------------------------------------------------------------------

/// A typo'd relation name used to be swallowed by the idempotent no-op: exit 0, `changed:
/// false`, and the real `depends_on` edge still in the file — indistinguishable from a genuine
/// "there was nothing to remove", so a cleanup script reported the edge removed.
#[test]
fn test_unrelate_unknown_relation_is_a_usage_error_and_the_real_edge_survives() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "  depends_on: [\"E1S1\"]\n");
    let before = read_story(root, "E1S2");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["unrelate", "E1S2", "dependson", "E1S1", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    let message = val["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("dependson"),
        "the message must name what was typed: {message}"
    );
    for relation in [
        "depends_on",
        "extends",
        "supersedes",
        "traces_to",
        "verifies",
        "mitigates",
        "closes_dw",
        "governed_by",
    ] {
        assert!(
            message.contains(relation),
            "the message must name every valid relation, missing '{relation}': {message}"
        );
    }

    assert_eq!(
        read_story(root, "E1S2"),
        before,
        "the real depends_on edge must survive a typo'd unrelate"
    );
}

/// The same input on `relate` was reported as `invalid_relation_kind` (exit 1) — factually
/// wrong: the name is not a relation at all, so there is no kind pair to disallow.
#[test]
fn test_relate_unknown_relation_is_a_usage_error_not_invalid_relation_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S1", "dependson", "E1S2", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    let message = val["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("depends_on") && message.contains("governed_by"),
        "the message must name the valid relations: {message}"
    );

    let content = read_story(root, "E1S1");
    assert!(!content.contains("relations:"), "no write must occur");
}

/// The name check runs before the entity lookups, so a typo is reported as a typo even when the
/// ids are unresolvable — the usage error the user can act on, not a dangling-target report.
#[test]
fn test_relate_unknown_relation_is_reported_before_entity_resolution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E9S9", "dependson", "E9S8", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("dependson"));
}

/// `verifies` is a known relation with no allowed kind pairs until Epic 3 models gates. It must
/// be refused as a disallowed pair (exit 1), never as an unknown relation: `allowed_kind_pairs`
/// returns an empty slice for both cases, which is why the name list exists separately.
#[test]
fn test_relate_verifies_is_a_disallowed_pair_not_an_unknown_relation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S1", "verifies", "E1S2", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "invalid_relation_kind");

    let content = read_story(root, "E1S1");
    assert!(!content.contains("relations:"), "no write must occur");
}

/// `unrelate` accepts every known relation name, `verifies` included: nothing about a name that
/// cannot yet be written stops an edge under it from being removed.
#[test]
fn test_unrelate_verifies_is_accepted_as_a_known_relation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["unrelate", "E1S1", "verifies", "FR-1", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["changed"], false);
}

/// The positive fence on the no-op path: `--if-version` matching, on an edge that already
/// exists, must still be exit 0 `changed: false`. Its stale twin is covered above, and without
/// this one an over-eager fence (comparing against the post-bump version, say) would look right.
#[test]
fn test_relate_no_op_with_a_satisfied_if_version_is_still_a_no_op() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E1S1", "");
    create_story(root, "E1S2", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let version = val["version"].as_u64().unwrap();
    assert_eq!(val["changed"], true);

    // The same edge again, fencing against the version the first call left behind.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "relate",
            "E1S1",
            "depends_on",
            "E1S2",
            "--if-version",
            &version.to_string(),
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["changed"], false, "the edge is already there: {val}");
    assert_eq!(
        val["version"], version,
        "a satisfied fence on a no-op must not bump the version: {val}"
    );
}

/// `GRAPH_EDGE_RELATIONS` is a second, hand-written list of relation names (the subset `graph
/// --dot` draws). It cannot be derived from the kind-pair table — it is a rendering choice, not
/// the whole set — but every member must still *be* a relation, or the graph silently filters on
/// a name nothing can hold.
#[test]
fn test_graph_edge_relations_are_all_known_relations() {
    for name in ["depends_on", "extends", "supersedes"] {
        assert!(
            qdev_core::is_known_relation(name),
            "`graph --dot` filters on '{name}', which is not a relation name"
        );
    }
}
