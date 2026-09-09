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

    // Spawn thread to hold lock for 6 seconds (longer than the 5s timeout)
    let lock_holder_thread = thread::spawn(move || {
        let _guard =
            qdev_core::acquire_write_lock(&lock_path_clone, Duration::from_millis(5000)).unwrap();
        lock_acquired_clone.store(true, Ordering::SeqCst);
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(6000)
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
