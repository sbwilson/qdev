//! `qdev validate` CLI tests (spec-1-11): `--json` shape, exit codes, `--changed` filtering, and
//! `--fix-ids` non-interactive refusal / guided renumber.

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

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

fn write_story(dir: &Path, id: &str) {
    fs::create_dir_all(dir).unwrap();
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
---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

fn read_story(dir: &Path, id: &str) -> String {
    fs::read_to_string(dir.join(format!("{}.md", id))).unwrap()
}

fn git(root: &Path, args: &[&str]) {
    let mut full_args = vec!["-c", "commit.gpgsign=false"];
    full_args.extend_from_slice(args);
    let status = StdCommand::new("git")
        .args(&full_args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn test_validate_happy_path_clean_workspace_exit_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["findings"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Duplicate ids -> JSON shape {code, severity, path, message}, exit 1
// ---------------------------------------------------------------------------

#[test]
fn test_validate_reports_duplicate_planning_id_json_shape_and_exit_1() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    for f in findings {
        assert_eq!(f["code"], "duplicate_planning_id");
        assert_eq!(f["severity"], "error");
        assert!(f["path"].is_string());
        assert!(f["message"].is_string());
    }
}

// ---------------------------------------------------------------------------
// Unregistered target module (seeded via `qdev update --field`)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_reports_unregistered_target_module() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "E1S1", "--field", "target_modules=[\"ghost\"]"])
        .assert()
        .success();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|f| f["code"] == "target_module_not_registered"));
}

// ---------------------------------------------------------------------------
// --changed filtering
// ---------------------------------------------------------------------------

#[test]
fn test_validate_changed_excludes_untouched_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git(root, &["init", "-q", "-b", "develop"]);
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    fs::write(
        stories_dir.join("E1S1-dup.md"),
        fs::read_to_string(stories_dir.join("E1S1.md")).unwrap(),
    )
    .unwrap();

    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "baseline with duplicate ids"]);
    git(root, &["checkout", "-q", "-b", "feature"]);

    // Nothing has changed relative to develop yet: --changed must exclude every finding.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--changed", "--json"])
        .assert()
        .success()
        .code(0);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["findings"].as_array().unwrap().len(), 0);

    // Touch one of the two duplicate files on the feature branch: only its finding survives.
    // (Appending, not rewriting: `write_story` produces byte-identical content, which `git diff`
    // would not consider a change at all.)
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(stories_dir.join("E1S1.md"))
        .unwrap();
    use std::io::Write;
    writeln!(f, "- extra AC.").unwrap();
    drop(f);

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["validate", "--changed", "--json"])
        .assert()
        .failure()
        .code(1);
    let output2 = assert2.get_output();
    let val2: Value = serde_json::from_str(std::str::from_utf8(&output2.stdout).unwrap()).unwrap();
    let findings2 = val2["findings"].as_array().unwrap();
    assert_eq!(findings2.len(), 1);
    assert_eq!(findings2[0]["path"], "docs/specs/stories/E1S1.md");
}

// ---------------------------------------------------------------------------
// --fix-ids gating
// ---------------------------------------------------------------------------

#[test]
fn test_fix_ids_non_interactive_without_yes_refuses_exit_3_no_writes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();
    let before = read_story(&stories_dir, "E1S2-dup");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--fix-ids", "--non-interactive", "--json"])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["error"]["code"], "needs_confirmation");

    let after = read_story(&stories_dir, "E1S2-dup");
    assert_eq!(before, after, "no write must occur on refusal");
}

#[test]
fn test_fix_ids_with_yes_renumbers_duplicate_and_leaves_workspace_clean() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    fs::write(
        stories_dir.join("E1S1-dup.md"),
        fs::read_to_string(stories_dir.join("E1S1.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["validate", "--fix-ids", "--non-interactive", "--yes"])
        .assert()
        .success()
        .code(0);

    // The lexicographically-first path keeps the id ("E1S1-dup.md" sorts before "E1S1.md" because
    // '-' < '.'); the other duplicate is renumbered to the next available Story id for the same
    // epic, with its version bumped. Check both files rather than assuming which one wins.
    let a = read_story(&stories_dir, "E1S1");
    let b = read_story(&stories_dir, "E1S1-dup");
    let a_keeps_id = a.contains("id: E1S1\n");
    let b_keeps_id = b.contains("id: E1S1\n");
    assert_ne!(
        a_keeps_id, b_keeps_id,
        "exactly one file must keep id E1S1; a={:?} b={:?}",
        a, b
    );
    let renumbered = if a_keeps_id { &b } else { &a };
    assert!(!renumbered.contains("id: E1S1\n"));
    assert!(renumbered.contains("version: 2"));

    // A follow-up plain validate call must now be clean.
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);
    let output2 = assert2.get_output();
    let val2: Value = serde_json::from_str(std::str::from_utf8(&output2.stdout).unwrap()).unwrap();
    assert_eq!(val2["findings"].as_array().unwrap().len(), 0);
}

#[test]
fn test_fix_ids_rewrites_relations_pointing_at_renumbered_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut relate_cmd = Command::cargo_bin("qdev").unwrap();
    relate_cmd
        .current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .success();

    let mut fix_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = fix_cmd
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1);
    assert_eq!(renumbered[0]["old_id"], "E1S2");
    let new_id = renumbered[0]["new_id"].as_str().unwrap().to_string();

    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    let get_assert = get_cmd
        .current_dir(root)
        .args(["get", "E1S1", "--json"])
        .assert()
        .success();
    let get_output = get_assert.get_output();
    let get_val: Value =
        serde_json::from_str(std::str::from_utf8(&get_output.stdout).unwrap()).unwrap();
    let depends_on = get_val["relations"]["depends_on"].as_array().unwrap();
    assert_eq!(
        depends_on,
        &vec![Value::String(new_id)],
        "the depends_on edge must now target the renumbered id, not the stale old id"
    );
}

#[test]
fn test_fix_ids_rewrites_citations_under_configured_module_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Register a module whose paths cover a citation-bearing source file.
    let mut toml_contents = fs::read_to_string(root.join("qdev.toml")).unwrap();
    toml_contents.push_str("\n[[modules]]\nid = \"core\"\npaths = [\"src/**\"]\n");
    fs::write(root.join("qdev.toml"), toml_contents).unwrap();

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "// see [E1S2] for context\nfn noop() {}\n",
    )
    .unwrap();

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1);
    assert_eq!(renumbered[0]["old_id"], "E1S2");
    let new_id = renumbered[0]["new_id"].as_str().unwrap().to_string();
    assert_eq!(renumbered[0]["citations_rewritten"], 1);

    let src_content = fs::read_to_string(src_dir.join("lib.rs")).unwrap();
    assert!(src_content.contains(&format!("[{}]", new_id)));
    assert!(!src_content.contains("[E1S2]"));
}

#[test]
fn test_fix_ids_no_duplicates_is_a_clean_noop() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["validate", "--fix-ids", "--non-interactive", "--yes"])
        .assert()
        .success()
        .code(0);
}
