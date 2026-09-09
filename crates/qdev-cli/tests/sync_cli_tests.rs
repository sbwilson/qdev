//! `qdev sync` CLI tests (spec-1-12): `--json` shape and counts, `--rebuild`, and the
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
fn test_sync_json_shape_and_exit_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sync", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["schema_version"], "1");
    for field in ["parsed", "unchanged", "purged", "findings"] {
        assert!(
            val[field].is_u64(),
            "expected numeric field '{}' in sync payload: {}",
            field,
            val
        );
    }
}

#[test]
fn test_sync_settles_to_all_unchanged_once_the_boot_time_sweep_has_run() {
    // Every `qdev` invocation against an initialized workspace runs the boot-time hydration
    // sweep (spec-1-6/1-7) before any command dispatches, `sync` included. So by the time
    // `handle_sync`'s own `sweep_workspace` call runs, that same process's boot sweep has
    // already absorbed any on-disk changes; the explicit sync settles to `unchanged` for
    // everything and `parsed`/`purged` at 0. This is the plain-sync counterpart to
    // `test_sync_rebuild_reparses_everything`, which forces real work via `--rebuild` instead.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_story(&stories_dir, "E1S2", "Two");

    // First sync: the boot-time sweep for this very invocation parses both new files before
    // `sync`'s own sweep runs, which then finds nothing left outstanding.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sync", "--json"])
        .assert()
        .success();

    // Modify one file, then sync again: same story, the modification is caught by this
    // invocation's own boot sweep first.
    write_story(&stories_dir, "E1S1", "One modified");
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2
        .current_dir(root)
        .args(["sync", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["parsed"], 0);
    assert_eq!(val["purged"], 0);
    assert_eq!(
        val["unchanged"], 3,
        "both story files plus qdev.toml must all be settled unchanged: {}",
        val
    );
}

#[test]
fn test_sync_rebuild_reparses_everything() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1", "One");
    write_story(&stories_dir, "E1S2", "Two");

    // Warm the cache first so a plain sync afterwards would report everything unchanged.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sync", "--json"])
        .assert()
        .success();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2
        .current_dir(root)
        .args(["sync", "--rebuild", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    // A full rebuild re-parses both story files plus qdev.toml itself from scratch, unlike a
    // plain sync which would report them unchanged.
    assert_eq!(val["parsed"], 3);
    assert_eq!(val["unchanged"], 0);
    assert_eq!(val["purged"], 0);

    // The entity must still be queryable afterwards: rebuild is lossless.
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    cmd3.current_dir(root)
        .args(["get", "story", "E1S1", "--json"])
        .assert()
        .success();
}

#[test]
fn test_sync_text_output_contains_counts() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["sync"]).assert().success();

    let output = assert.get_output();
    let text = std::str::from_utf8(&output.stdout).unwrap();
    assert!(text.contains("parsed="), "text output: {}", text);
    assert!(text.contains("unchanged="), "text output: {}", text);
    assert!(text.contains("purged="), "text output: {}", text);
    assert!(text.contains("findings="), "text output: {}", text);
}

#[test]
fn test_sync_uninitialized_workspace_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sync", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_sync_uninitialized_workspace_text_mode_exits_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sync"])
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
