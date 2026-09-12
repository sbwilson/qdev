use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use assert_cmd::Command;
use predicates::prelude::*;
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

fn create_sample_story(root: &Path, id: &str, content: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{}.md", id)), content).unwrap();
}

fn relation_story(id: &str, relations: &str) -> String {
    format!(
        r#"---
id: {id}
title: Story {id}
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{relations}---

## Acceptance Criteria
- AC.
"#
    )
}

fn assert_update_refusal_is_non_mutating(
    root: &Path,
    field: &str,
    expected_code: &str,
    expected_exit: i32,
    original: &str,
) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "E1S2", "--field", field, "--json"])
        .assert()
        .failure()
        .code(expected_exit);
    let output: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(output["error"]["code"], expected_code);
    assert_eq!(
        fs::read_to_string(root.join("docs/specs/stories/E1S2.md")).unwrap(),
        original,
        "a refused relation update must not change frontmatter bytes"
    );
    let connection = rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
    let version: u64 = connection
        .query_row(
            "SELECT version FROM entities WHERE id = 'E1S2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let dirty_rows: u64 = connection
        .query_row(
            "SELECT COUNT(*) FROM dirty_entities WHERE id = 'E1S2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(version, 1);
    assert_eq!(dirty_rows, 0);
}

#[test]
fn test_update_relations_uses_the_same_prewrite_graph_gate_as_relate() {
    // Dangling target.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E1S2", &relation_story("E1S2", ""));
    let original = fs::read_to_string(root.join("docs/specs/stories/E1S2.md")).unwrap();
    assert_update_refusal_is_non_mutating(
        root,
        "relations={depends_on: [E9S9]}",
        "dangling_relation",
        1,
        &original,
    );

    // Invalid source/target pair.
    // An epic belongs in the epics directory; use a separate workspace to keep the fixture's
    // filename-to-kind rule explicit.
    let pair_temp = TempDir::new().unwrap();
    let pair_root = pair_temp.path();
    setup_workspace(pair_root);
    create_sample_story(pair_root, "E1S2", &relation_story("E1S2", ""));
    let epic_dir = pair_root.join("docs/specs/epics");
    fs::create_dir_all(&epic_dir).unwrap();
    fs::write(epic_dir.join("E1.md"), relation_story("E1", "")).unwrap();
    let pair_original = fs::read_to_string(pair_root.join("docs/specs/stories/E1S2.md")).unwrap();
    assert_update_refusal_is_non_mutating(
        pair_root,
        "relations={traces_to: [E1]}",
        "invalid_relation_kind",
        1,
        &pair_original,
    );

    // A cycle in the replacement's final graph.
    let cycle_temp = TempDir::new().unwrap();
    let cycle_root = cycle_temp.path();
    setup_workspace(cycle_root);
    create_sample_story(
        cycle_root,
        "E1S1",
        &relation_story("E1S1", "relations:\n  depends_on: [E1S2]\n"),
    );
    create_sample_story(cycle_root, "E1S2", &relation_story("E1S2", ""));
    let cycle_original = fs::read_to_string(cycle_root.join("docs/specs/stories/E1S2.md")).unwrap();
    assert_update_refusal_is_non_mutating(
        cycle_root,
        "relations={depends_on: [E1S1]}",
        "dependency_cycle",
        1,
        &cycle_original,
    );

    // Unknown names are usage errors through this surface too, before schema validation could
    // turn them into a different logical-failure contract.
    assert_update_refusal_is_non_mutating(
        root,
        "relations={unknown_relation: [E1S2]}",
        "usage_error",
        2,
        &original,
    );

    // A malformed map is intentionally not interpreted by the graph gate; the existing generic
    // writer's schema failure remains responsible, and still cannot mutate any write state.
    assert_update_refusal_is_non_mutating(
        root,
        "relations=not-a-map",
        "schema_validation_failed",
        1,
        &original,
    );
}

#[test]
fn test_update_relations_replaces_and_removes_edges_in_the_final_graph() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E1S1", &relation_story("E1S1", ""));
    create_sample_story(root, "E1S3", &relation_story("E1S3", ""));
    create_sample_story(
        root,
        "E1S2",
        &relation_story("E1S2", "relations:\n  depends_on: [E1S1]\n"),
    );
    // The existing source map forms a cycle. Replacing it must assess the final map rather than
    // treating the new entry as an addition to the old one.
    create_sample_story(
        root,
        "E1S1",
        &relation_story("E1S1", "relations:\n  depends_on: [E1S2]\n"),
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "E1S2",
            "--field",
            "relations={depends_on: [E1S3]}",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(root.join("docs/specs/stories/E1S2.md")).unwrap();
    assert!(content.contains("depends_on:"));
    assert!(content.contains("E1S3"));
    assert!(!content.contains("E1S1"));

    // A new process hydrates exactly the replacement map into cache rows.
    let mut status = Command::cargo_bin("qdev").unwrap();
    status.current_dir(root).args(["status"]).assert().success();
    let connection = rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
    let rows: Vec<(String, String)> = {
        let mut statement = connection
            .prepare("SELECT relation, target_id FROM relations WHERE source_id = 'E1S2'")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    assert_eq!(rows, vec![("depends_on".to_string(), "E1S3".to_string())]);
}

#[test]
fn test_update_rejects_case_variant_of_canonical_field_key() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E1S2", &relation_story("E1S2", ""));
    let original = fs::read_to_string(root.join("docs/specs/stories/E1S2.md")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "E1S2", "--field", "Status=ready", "--json"])
        .assert()
        .failure()
        .code(2);
    let output: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(output["error"]["code"], "usage_error");
    assert_eq!(
        fs::read_to_string(root.join("docs/specs/stories/E1S2.md")).unwrap(),
        original
    );

    let mut kind = Command::cargo_bin("qdev").unwrap();
    kind.current_dir(root)
        .args(["update", "E1S2", "--field", "Kind=story", "--json"])
        .assert()
        .failure()
        .code(2);
    assert_eq!(
        fs::read_to_string(root.join("docs/specs/stories/E1S2.md")).unwrap(),
        original
    );
}

#[test]
fn test_update_relations_validates_against_the_proposed_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let epics = root.join("docs/specs/epics");
    fs::create_dir_all(&epics).unwrap();
    fs::write(epics.join("E1.md"), relation_story("E1", "")).unwrap();
    create_sample_story(root, "E1S1", &relation_story("E1S1", ""));

    // `depends_on` is invalid for the cached Epic, but valid for the Story the same update
    // creates. Preflight must use the latter just as the writer's schema validation does.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "E1",
            "--field",
            "kind=story",
            "--field",
            "relations={depends_on: [E1S1]}",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(epics.join("E1.md")).unwrap();
    assert!(content.contains("kind: story"));
    assert!(content.contains("depends_on:"));
}

#[test]
fn test_update_single_field_happy_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
status: draft
version: 1
owners: ["simon"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Original criteria.
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "story", "E12S4", "--status", "in-progress"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Updated story E12S4 (version 2)"));

    let file_path = root.join("docs/specs/stories/E12S4.md");
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("status: in-progress\n"));
    assert!(content.contains("version: 2\n"));
    assert!(content.contains("updated_by:\n  type: human\n  id: simon\n"));

    // Verify cache was upserted and marked dirty
    let db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let (status, version): (String, u64) = conn
        .query_row(
            "SELECT status, version FROM entities WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "in-progress");
    assert_eq!(version, 2);

    let dirty_count: u32 = conn
        .query_row(
            "SELECT count(*) FROM dirty_entities WHERE id = 'E12S4';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(dirty_count, 1);

    // Verify stories table
    let (epic_id, seq): (String, u32) = conn
        .query_row(
            "SELECT epic_id, seq FROM stories WHERE id = 'E12S4';",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(epic_id, "E12");
    assert_eq!(seq, 4);
}

#[test]
fn test_update_via_universal_id_without_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "E12S4", "--status", "ready"])
        .assert()
        .success()
        .code(0);

    let file_path = root.join("docs/specs/stories/E12S4.md");
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("status: ready\n"));
    assert!(content.contains("version: 2\n"));
}

#[test]
fn test_update_golden_comment_preservation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let original = r#"---
# Top frontmatter comment
# Another top comment
id: E12S4
title: "CoreResponse Buffer Layout" # Inline comment on title

# Comment above status
status: draft

# Comment above version
version: 1

owners:
  - simon
  - team:core-platform

# Comment before created_by
created_by:
  type: human
  id: simon

updated_by:
  type: human
  id: simon

# Bottom frontmatter comment
---

## Acceptance Criteria
- Original criteria line 1.
- Original criteria line 2.

## Implementation Notes
- Keep buffer aligned to 64 bytes.
"#;
    create_sample_story(root, "E12S4", original);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "in-progress",
            "--author-type",
            "agent",
            "--author-id",
            "claude-code",
        ])
        .assert()
        .success()
        .code(0);

    let expected = r#"---
# Top frontmatter comment
# Another top comment
id: E12S4
title: "CoreResponse Buffer Layout" # Inline comment on title

# Comment above status
status: in-progress

# Comment above version
version: 2

owners:
  - simon
  - team:core-platform

# Comment before created_by
created_by:
  type: human
  id: simon

updated_by:
  type: agent
  id: claude-code

# Bottom frontmatter comment
---

## Acceptance Criteria
- Original criteria line 1.
- Original criteria line 2.

## Implementation Notes
- Keep buffer aligned to 64 bytes.
"#;

    let file_path = root.join("docs/specs/stories/E12S4.md");
    let actual = fs::read_to_string(&file_path).unwrap();
    assert_eq!(
        actual, expected,
        "Comments, blank lines, and key order outside edited fields must survive byte-for-byte"
    );
}

#[test]
fn test_update_optimistic_concurrency_match() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "ready",
            "--if-version",
            "1",
        ])
        .assert()
        .success()
        .code(0);

    let file_path = root.join("docs/specs/stories/E12S4.md");
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("version: 2\n"));
    assert!(content.contains("status: ready\n"));
}

#[test]
fn test_update_optimistic_concurrency_mismatch_exits_5() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "ready",
            "--if-version",
            "2", // Mismatch: file is version 1
            "--json",
        ])
        .assert()
        .failure()
        .code(5);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["error"]["code"], "version_mismatch");

    // Verify file was not modified
    let file_path = root.join("docs/specs/stories/E12S4.md");
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("status: draft\n"));
    assert!(content.contains("version: 1\n"));
}

#[test]
fn test_update_body_section_replacement_happy_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
- Old criterion 1.
- Old criterion 2.

## Implementation Directives
- Directive text untouched.
"#;
    create_sample_story(root, "E12S4", story_content);

    let ac_file = root.join("ac.md");
    fs::write(&ac_file, "- New criterion A.\n- New criterion B.\n\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--section",
            "Acceptance Criteria",
            "--file",
            "ac.md",
        ])
        .assert()
        .success()
        .code(0);

    let file_path = root.join("docs/specs/stories/E12S4.md");
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains("- New criterion A.\n- New criterion B.\n"));
    assert!(!content.contains("- Old criterion 1."));
    assert!(content.contains("## Implementation Directives\n- Directive text untouched."));
    assert!(content.contains("version: 2\n"));
}

#[test]
fn test_update_body_section_replacement_non_existent_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
- AC text.
"#;
    create_sample_story(root, "E12S4", story_content);

    let ac_file = root.join("ac.md");
    fs::write(&ac_file, "New text\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--section",
            "NonExistent",
            "--file",
            "ac.md",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_update_advisory_lock_timeout_exits_5() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let lock_path = root.join(".qdev/cache/write.lock");
    let root_path_buf = root.to_path_buf();
    let lock_released = Arc::new(AtomicBool::new(false));
    let lock_released_clone = lock_released.clone();
    let lock_acquired = Arc::new(AtomicBool::new(false));
    let lock_acquired_clone = lock_acquired.clone();
    let lock_path_clone = lock_path.clone();

    // The `lock_released` flag below is what actually ends the hold, right after the assertion;
    // the elapsed-time ceiling is only a safety valve for a panicking assert. It has to clear the
    // command's own 5s timeout *plus* however long this runner takes to spawn the binary — at
    // 6 seconds a slow Windows agent released the lock while `qdev` was still retrying, and the
    // update then succeeded where the test demanded exit 5.
    let lock_holder_thread = thread::spawn(move || {
        let _guard =
            qdev_core::acquire_write_lock(&lock_path_clone, Duration::from_millis(5000)).unwrap();
        lock_acquired_clone.store(true, Ordering::SeqCst);
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(60_000)
            && !lock_released_clone.load(Ordering::SeqCst)
        {
            thread::sleep(Duration::from_millis(50));
        }
    });

    // Wait until thread has acquired the lock
    while !lock_acquired.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
    }

    // Run qdev update while lock is held; should timeout after 5s and exit with code 5
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(&root_path_buf)
        .args(["update", "story", "E12S4", "--status", "ready", "--json"])
        .timeout(Duration::from_secs(10))
        .assert()
        .failure()
        .code(5);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["error"]["code"], "lock_timeout");

    lock_released.store(true, Ordering::SeqCst);
    let _ = lock_holder_thread.join();
}

#[test]
fn test_update_concurrent_atomic_updates() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story1 = r#"---
id: E12S1
title: Story One
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
- AC 1.
"#;

    let story2 = r#"---
id: E12S2
title: Story Two
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
- AC 2.
"#;

    create_sample_story(root, "E12S1", story1);
    create_sample_story(root, "E12S2", story2);

    let root_path1 = root.to_path_buf();
    let handle1 = thread::spawn(move || {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        cmd.current_dir(root_path1)
            .args(["update", "story", "E12S1", "--status", "in-progress"])
            .assert()
            .success()
            .code(0);
    });

    let root_path2 = root.to_path_buf();
    let handle2 = thread::spawn(move || {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        cmd.current_dir(root_path2)
            .args(["update", "story", "E12S2", "--status", "ready"])
            .assert()
            .success()
            .code(0);
    });

    handle1.join().unwrap();
    handle2.join().unwrap();

    let c1 = fs::read_to_string(root.join("docs/specs/stories/E12S1.md")).unwrap();
    let c2 = fs::read_to_string(root.join("docs/specs/stories/E12S2.md")).unwrap();
    assert!(c1.contains("status: in-progress\n"));
    assert!(c1.contains("version: 2\n"));
    assert!(c2.contains("status: ready\n"));
    assert!(c2.contains("version: 2\n"));
}

#[test]
fn test_update_non_existent_entity_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "story", "E99S99", "--status", "ready"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_update_json_envelope_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "in-progress",
            "--json",
        ])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["id"], "E12S4");
    assert_eq!(val["status"], "in-progress");
    assert_eq!(val["version"], 2);
    assert_eq!(val["updated_by"]["type"], "human");
    assert_eq!(val["updated_by"]["id"], "simon");
}

#[test]
fn test_update_attribution_override_flags() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "ready",
            "--author-type",
            "agent",
            "--author-id",
            "subagent-dev-1",
        ])
        .assert()
        .success()
        .code(0);

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("updated_by:\n  type: agent\n  id: subagent-dev-1\n"));
}

#[test]
fn test_update_field_arbitrary_frontmatter_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "story", "E12S4", "--field", "appetite=medium"])
        .assert()
        .success()
        .code(0);

    let file_content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(file_content.contains("appetite: medium\n"));
    assert!(file_content.contains("version: 2\n"));

    // Verify cache has updated appetite
    let db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let appetite: String = conn
        .query_row(
            "SELECT appetite FROM stories WHERE id = 'E12S4';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(appetite, "medium");
}

#[test]
fn test_update_title_flag() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: Original Title
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "story", "E12S4", "--title", "Brand New Title"])
        .assert()
        .success()
        .code(0);

    let file_content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(
        file_content.contains("title: Brand New Title\n")
            || file_content.contains("title: \"Brand New Title\"\n")
    );

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let title: String = conn
        .query_row("SELECT title FROM entities WHERE id = 'E12S4';", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(title, "Brand New Title");
}

#[test]
fn test_update_post_patch_schema_validation_rejection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--field",
            "appetite=invalid_appetite",
        ])
        .assert()
        .failure()
        .code(1); // ExitCode::LogicalFailure
}

#[test]
fn test_update_rejects_managed_fields() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "story", "E12S4", "--field", "id=E99S99"])
        .assert()
        .failure()
        .code(2); // ExitCode::UsageError

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["update", "story", "E12S4", "--field", "version=99"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_update_rejects_conflicting_arguments() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: CoreResponse Buffer Layout
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
"#;
    create_sample_story(root, "E12S4", story_content);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "ready",
            "--field",
            "status=in-progress",
        ])
        .assert()
        .failure()
        .code(2);

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--title",
            "Flag Title",
            "--field",
            "title=Field Title",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_update_no_arguments_rejected_usage_error() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: Sample Story
status: draft
version: 1
---

## Description
Initial description.
"#;
    create_sample_story(root, "E12S4", story_content);
    let story_path = root.join("docs/specs/stories/E12S4.md");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "story", "E12S4"])
        .assert()
        .failure()
        .code(2);

    // Assert that the file content and version were left completely unmodified
    let content_after = fs::read_to_string(&story_path).unwrap();
    assert_eq!(
        content_after, story_content,
        "Target file must not be modified when update has no modification flags"
    );
}

#[test]
fn test_update_section_file_pairing_validation() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: Sample Story
status: draft
version: 1
---

## Acceptance Criteria
- AC 1
"#;
    create_sample_story(root, "E12S4", story_content);

    // 1. --section without --file
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--section",
            "Acceptance Criteria",
        ])
        .assert()
        .failure()
        .code(2);

    // 2. --file without --section
    let ac_file = root.join("ac.md");
    fs::write(&ac_file, "- AC 2\n").unwrap();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--file",
            ac_file.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_update_field_invalid_key_characters_rejected() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);

    let story_content = r#"---
id: E12S4
title: Sample Story
status: draft
version: 1
---

## Description
Test.
"#;
    create_sample_story(root, "E12S4", story_content);

    // Key with whitespace
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["update", "story", "E12S4", "--field", "invalid key=foo"])
        .assert()
        .failure()
        .code(2);

    // Key with colon
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["update", "story", "E12S4", "--field", "invalid:key=foo"])
        .assert()
        .failure()
        .code(2);
}

/// Author attribution is audited (AD-12), so an unrecognized author type is refused wherever it
/// comes from — the flag *and* `QDEV_AUTHOR_TYPE`. The env var used to be silently rewritten to
/// "human", which wrote a wrong-but-plausible author into the record with nothing to tell the
/// user their environment was misconfigured.
#[test]
fn test_invalid_author_type_is_a_usage_error_from_flag_and_env() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let story = r#"---
id: E12S4
title: Attribution Story
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
- One.
"#;
    create_sample_story(root, "E12S4", story);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "update",
            "story",
            "E12S4",
            "--status",
            "in-progress",
            "--author-type",
            "robot",
        ])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--author-type"))
        .stderr(predicate::str::contains("must be 'human' or 'agent'"));

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("QDEV_AUTHOR_TYPE", "robot")
        .args(["update", "story", "E12S4", "--status", "in-progress"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("QDEV_AUTHOR_TYPE"))
        .stderr(predicate::str::contains("must be 'human' or 'agent'"));

    // The story is untouched by either refusal — both refuse before any write.
    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("status: draft"), "got: {}", content);
    assert!(content.contains("version: 1"));

    // A valid env value still resolves, and is recorded as the author.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("QDEV_AUTHOR_TYPE", "agent")
        .env("QDEV_AUTHOR_ID", "claude-code")
        .args(["update", "story", "E12S4", "--status", "in-progress"])
        .assert()
        .success();

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("type: agent"), "got: {}", content);
    assert!(content.contains("id: claude-code"), "got: {}", content);
}

// ---------------------------------------------------------------------------
// Kind agreement between a write and the sweep that follows it (identity seam)
// ---------------------------------------------------------------------------

/// A file's frontmatter `kind:` is what hydration believes, so it is what a write must validate
/// against. Before this, writers took the kind from the directory: an update validated against
/// the ADR schema, exited 0, and the next boot sweep recorded an error-severity
/// `schema_violation` on the file it had just written.
#[test]
fn test_update_validates_against_frontmatter_kind_not_the_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // In the ADR directory, but declaring `kind: story`. `appetite` is a free-form extra field
    // under the ADR schema and an enum under the story schema, so the two schemas disagree
    // about this file.
    let adrs = root.join("docs/specs/adrs");
    fs::create_dir_all(&adrs).unwrap();
    fs::write(
        adrs.join("AD-9.md"),
        r#"---
id: AD-9
kind: story
title: Buffer layout decision
status: draft
version: 1
appetite: bogus
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Decision
- Chosen.
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "AD-9", "--status", "ready", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "schema_validation_failed");
    // Nothing was written: the file still says draft.
    let content = fs::read_to_string(adrs.join("AD-9.md")).unwrap();
    assert!(content.contains("status: draft\n"), "{content}");
}

/// The other side of the same rule: when the write does succeed on a file whose `kind:`
/// disagrees with its directory, the next boot sweep must record no new `schema_violation` for
/// it — the write and the sweep now validate against one schema.
#[test]
fn test_update_on_a_kind_disagreeing_file_leaves_no_schema_violation_behind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let adrs = root.join("docs/specs/adrs");
    fs::create_dir_all(&adrs).unwrap();
    fs::write(
        adrs.join("AD-9.md"),
        r#"---
id: AD-9
kind: story
title: Buffer layout decision
status: draft
version: 1
appetite: small
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Decision
- Chosen.
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "AD-9", "--status", "ready"])
        .assert()
        .success();

    // The next command boots, sweeps, and validates: no finding may have appeared for the file
    // the update just wrote.
    let mut validate = Command::cargo_bin("qdev").unwrap();
    let assert = validate
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert!(
        !findings
            .iter()
            .any(|f| f["path"] == "docs/specs/adrs/AD-9.md"),
        "the sweep must record nothing for the file the write just validated: {val}"
    );
}

/// The kind is resolved from the *patched* content, not the content that was read — so an update
/// that itself edits `kind:` is validated against the kind it is creating. That choice is the
/// only reason to prefer patched over existing content, and it was the one case no test covered:
/// both other kind tests use files that already declare `kind:` before the update, where the two
/// answers coincide.
#[test]
fn test_update_that_changes_kind_validates_against_the_new_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // A conventional ADR: no `kind:` field, so it reads as an ADR by directory. `appetite` is a
    // free-form extra field under the ADR schema and an enum under the story schema.
    let adrs = root.join("docs/specs/adrs");
    fs::create_dir_all(&adrs).unwrap();
    fs::write(
        adrs.join("AD-8.md"),
        r#"---
id: AD-8
title: Kind flip decision
status: draft
version: 1
appetite: bogus
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Decision
- Chosen.
"#,
    )
    .unwrap();

    // Flipping it to a story must be judged by the story schema, which rejects `appetite: bogus`.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "AD-8", "--field", "kind=story", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "schema_validation_failed");

    let content = fs::read_to_string(adrs.join("AD-8.md")).unwrap();
    assert!(
        !content.contains("kind: story"),
        "a refused update must leave the file alone: {content}"
    );

    // And the cache with it: the refusal happens before the upsert, so nothing of the old kind
    // is dropped and nothing of the new one is written. Read back through `get`, which is where
    // a wrongly-applied kind would surface.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "AD-8", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        val["kind"], "adr",
        "a refused update must leave the cache alone: {val}"
    );
}

// ---------------------------------------------------------------------------
// The identity rule answers "which file holds entity X", never the filesystem
// ---------------------------------------------------------------------------

fn story_doc(id: &str) -> String {
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
---

## Acceptance Criteria
- AC.
"#
    )
}

/// A lone `e1s1.md` is one file, so it resolves once. The resolver used to probe
/// `dir.join("E1S1.md").is_file()` before listing the directory, and on a case-insensitive host
/// the OS answered for that spelling too — so the single file was found twice, under two
/// different strings, and every writer refused with "Multiple entity files match" naming
/// `E1S1.md`, a file that does not exist, for an entity `qdev get` returns at exit 0.
#[test]
fn test_a_lowercase_file_name_resolves_once_for_every_writer() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(stories.join("e1s1.md"), story_doc("E1S1")).unwrap();
    fs::write(stories.join("E1S2.md"), story_doc("E1S2")).unwrap();

    let mut get = Command::cargo_bin("qdev").unwrap();
    get.current_dir(root)
        .args(["get", "E1S1", "--json"])
        .assert()
        .success();

    let mut update = Command::cargo_bin("qdev").unwrap();
    update
        .current_dir(root)
        .args(["update", "E1S1", "--status", "ready"])
        .assert()
        .success()
        .code(0);
    assert!(fs::read_to_string(stories.join("e1s1.md"))
        .unwrap()
        .contains("status: ready"));

    let mut relate = Command::cargo_bin("qdev").unwrap();
    relate
        .current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .success()
        .code(0);

    let mut unrelate = Command::cargo_bin("qdev").unwrap();
    unrelate
        .current_dir(root)
        .args(["unrelate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .success()
        .code(0);
}

/// "Multiple entity files match" now means two directory entries, so every name it prints is a
/// name `ls` would print.
#[test]
fn test_two_real_files_are_named_and_both_exist() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(stories.join("E1S1.md"), story_doc("E1S1")).unwrap();
    fs::write(stories.join("E1S1-copy.md"), story_doc("E1S1")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "E1S1", "--status", "ready", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    let message = val["error"]["message"].as_str().unwrap().to_string();
    assert!(message.contains("Multiple entity files match"), "{message}");
    for named in ["E1S1.md", "E1S1-copy.md"] {
        assert!(message.contains(named), "{message}");
        assert!(
            stories.join(named).is_file(),
            "the refusal must name only files that exist: {named}"
        );
    }
}

/// Both spellings of one name are two files only on a case-sensitive host; where the host will
/// not hold both, there is nothing to assert and the test says so rather than pretending.
#[test]
fn test_both_extension_spellings_are_two_files_on_a_case_sensitive_host() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(stories.join("E1S1.md"), story_doc("E1S1")).unwrap();
    fs::write(stories.join("E1S1.MD"), story_doc("E1S1")).unwrap();
    let entries = fs::read_dir(&stories).unwrap().count();
    if entries < 2 {
        // Case-insensitive filesystem (macOS, Windows): the second write replaced the first, so
        // there is one file and this row has nothing to assert *here*. The rule itself is pinned
        // host-independently by
        // `write_tests::test_an_uppercase_extension_takes_part_in_the_multiple_match_refusal`,
        // which makes two files that differ in the stem as well as the extension. Say so out
        // loud: a row that silently does not run reads as a row that passed.
        eprintln!(
            "SKIPPED test_both_extension_spellings_are_two_files_on_a_case_sensitive_host: \
             this filesystem is case-insensitive, so `E1S1.md` and `E1S1.MD` are one file"
        );
        // What does hold on every host: the surviving file resolves, rather than the pair being
        // reported as a duplicate of itself.
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        cmd.current_dir(root)
            .args(["update", "E1S1", "--status", "ready"])
            .assert()
            .success()
            .code(0);
        return;
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "E1S1", "--status", "ready", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let message = val["error"]["message"].as_str().unwrap().to_string();
    assert!(message.contains("Multiple entity files match"), "{message}");
    assert!(
        message.contains("E1S1.md") && message.contains("E1S1.MD"),
        "{message}"
    );
}

/// "Entity file not found for 'E1S9'" is true and useless when `notes.md` plainly holds `E1S9`
/// and `qdev get E1S9` just returned it. The refusal names the file and the rename that fixes
/// it — in the words `qdev validate` uses for the same file.
#[test]
fn test_an_id_held_by_an_unresolvable_name_is_refused_with_the_rename_that_fixes_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(stories.join("notes.md"), story_doc("E1S9")).unwrap();

    let mut get = Command::cargo_bin("qdev").unwrap();
    get.current_dir(root)
        .args(["get", "E1S9", "--json"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "E1S9", "--status", "ready", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    let message = val["error"]["message"].as_str().unwrap().to_string();
    assert!(
        message.contains("Entity file not found for 'E1S9'"),
        "{message}"
    );
    assert!(message.contains("docs/specs/stories/notes.md"), "{message}");
    assert!(message.contains("docs/specs/stories/E1S9.md"), "{message}");

    // And `qdev validate` says the same thing about the same file.
    let mut validate = Command::cargo_bin("qdev").unwrap();
    let validate_assert = validate
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success();
    let validate_val: Value = serde_json::from_slice(&validate_assert.get_output().stdout).unwrap();
    let warning = validate_val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["code"] == "entity_file_off_convention")
        .unwrap_or_else(|| panic!("{validate_val}"))
        .clone();
    let warning_message = warning["message"].as_str().unwrap();
    assert!(
        message.ends_with(warning_message),
        "the refusal must quote the warning verbatim:\n  refusal: {message}\n  warning: {warning_message}"
    );
}

/// Nothing on disk stays a plain not-found: the refusal only grows a file name when a file
/// really holds the id.
#[test]
fn test_an_id_no_file_holds_is_still_a_bare_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["update", "E1S1", "--status", "ready", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        val["error"]["message"], "Entity file not found for 'E1S1'",
        "{val}"
    );
}

/// The user-visible symptom NEW-4 was raised for: `update --field kind=` left the previous kind's
/// detail row in the cache, and `get` / `list` both `LEFT JOIN stories`, so an *epic* answered
/// with an `epic_id` and `list --epic E1` still listed it — until `sync --rebuild`, and only a
/// rebuild, changed both answers.
///
/// The assertion is the invariant rather than the symptom: both commands must answer the same
/// either side of a rebuild.
#[test]
fn test_a_kind_flip_answers_the_same_before_and_after_a_rebuild() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    create_sample_story(
        root,
        "E1S1",
        r#"---
id: E1S1
title: Buffer layout
status: draft
version: 1
owners: ["simon"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Original criteria.
"#,
    );

    let run = |args: &[&str]| -> Value {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd.current_dir(root).args(args).assert().success();
        serde_json::from_slice(&assert.get_output().stdout).unwrap()
    };

    // Hydrate, then confirm the fixture really is a story with a `stories` row behind it.
    run(&["sync", "--json"]);
    assert_eq!(run(&["get", "E1S1", "--json"])["epic_id"], "E1");

    run(&["update", "E1S1", "--field", "kind=epic", "--json"]);

    let get_before = run(&["get", "E1S1", "--json"]);
    let list_before = run(&["list", "epic", "--epic", "E1", "--json"]);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sync", "--rebuild", "--json"])
        .assert()
        .success();

    let get_after = run(&["get", "E1S1", "--json"]);
    let list_after = run(&["list", "epic", "--epic", "E1", "--json"]);

    assert_eq!(
        get_before, get_after,
        "`qdev get` answered differently either side of a rebuild"
    );
    assert_eq!(
        list_before, list_after,
        "`qdev list --epic` answered differently either side of a rebuild"
    );

    // And the answer is the right one: an epic is not a story and carries no owning epic.
    assert_eq!(get_after["kind"], "epic");
    assert!(
        get_after.get("epic_id").is_none() || get_after["epic_id"].is_null(),
        "an epic must not report an epic_id: {get_after}"
    );
    assert_eq!(
        list_after["items"].as_array().unwrap().len(),
        0,
        "`list --epic E1` must not list the epic itself: {list_after}"
    );
}
