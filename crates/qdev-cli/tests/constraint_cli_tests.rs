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

fn create_story(root: &Path, id: &str, status: &str, version: u64, constraints_yaml: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    let constraints_block = if constraints_yaml.is_empty() {
        String::new()
    } else {
        format!("constraints:\n{constraints_yaml}")
    };
    fs::write(
        dir.join(format!("{id}.md")),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: {status}
version: {version}
owners:
  - simon
epic_id: E12
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{constraints_block}---

## Acceptance Criteria
- Verify constraint behavior.
"#
        ),
    )
    .unwrap();
}

fn create_epic(root: &Path, id: &str, status: &str, version: u64, constraints_yaml: &str) {
    let dir = root.join("docs/specs/epics");
    fs::create_dir_all(&dir).unwrap();
    let constraints_block = if constraints_yaml.is_empty() {
        String::new()
    } else {
        format!("constraints:\n{constraints_yaml}")
    };
    fs::write(
        dir.join(format!("{id}.md")),
        format!(
            r#"---
id: {id}
title: "Epic {id}"
status: {status}
version: {version}
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{constraints_block}---

## Summary
- Epic summary.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_cli_constraint_add_happy_path() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 1, "");

    // 1. Add first constraint
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "no_go",
            "--",
            "Do not touch frame buffers",
        ])
        .assert()
        .success();

    let content1 = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content1.contains("version: 2"));
    assert!(content1.contains("id: NG-1"));
    assert!(content1.contains("kind: no_go"));

    // 2. Add second constraint of same kind: should allocate NG-2
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "no_go",
            "--",
            "Do not use unsafe",
        ])
        .assert()
        .success();

    let content2 = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content2.contains("version: 3"));
    assert!(content2.contains("id: NG-2"));

    // 3. Add appetite constraint: should allocate APP-1
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    cmd3.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "appetite",
            "--",
            "Maximum 2 days",
        ])
        .assert()
        .success();

    let content3 = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content3.contains("version: 4"));
    assert!(content3.contains("id: APP-1"));
    assert!(content3.contains("kind: appetite"));
}

#[test]
fn test_cli_constraint_add_json_envelope() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 1, "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "rabbit_hole",
            "--json",
            "--",
            "len == 0 does not mean empty",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert_eq!(json["schema_version"], "1");
    assert_eq!(json["id"], "E12S4/RH-1");
    assert_eq!(json["owner_id"], "E12S4");
    assert_eq!(json["kind"], "rabbit_hole");
    assert_eq!(json["text"], "len == 0 does not mean empty");
    assert_eq!(json["version"], 2);
}

#[test]
fn test_cli_constraint_add_to_epic() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_epic(root, "E12", "planning", 1, "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12",
            "--kind",
            "rabbit_hole",
            "--",
            "len == 0 does not mean empty",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(content.contains("version: 2"));
    assert!(content.contains("id: RH-1"));
    assert!(content.contains("kind: rabbit_hole"));
}

#[test]
fn test_cli_constraint_add_invalid_kind() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 1, "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "invalid_kind",
            "--",
            "text",
        ])
        .assert()
        .code(2);
}

#[test]
fn test_cli_constraint_add_empty_text() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 1, "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "no_go",
            "--",
            "   ",
        ])
        .assert()
        .code(2);
}

#[test]
fn test_cli_constraint_add_if_version_mismatch() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 2, "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "no_go",
            "--if-version",
            "1",
            "--",
            "text",
        ])
        .assert()
        .code(5);
}

#[test]
fn test_cli_constraint_remove_draft() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    create_story(root, "E12S4", "draft", 1, constraints_yaml);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/NG-1",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("version: 2"));
    assert!(!content.contains("NG-1"));
}

#[test]
fn test_cli_constraint_remove_non_draft_without_justification_refused() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    create_story(root, "E12S4", "ready", 1, constraints_yaml);

    // Refused with exit code 3 (PolicyRefusal)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/NG-1",
        ])
        .assert()
        .code(3);

    // File untouched
    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("version: 1"));
    assert!(content.contains("NG-1"));
}

#[test]
fn test_cli_constraint_remove_non_draft_with_justification_succeeds() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    create_story(root, "E12S4", "ready", 1, constraints_yaml);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/NG-1",
            "--justification",
            "Design changed to allow direct buffers",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("version: 2"));
    assert!(!content.contains("NG-1"));
}

#[test]
fn test_cli_constraint_remove_not_found() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 1, "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/NG-99",
        ])
        .assert()
        .code(2);
}

#[test]
fn test_cli_constraint_remove_json_envelope() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    create_story(root, "E12S4", "draft", 1, constraints_yaml);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/NG-1",
            "--json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let json: Value = serde_json::from_str(&stdout).expect("Valid JSON");
    assert_eq!(json["schema_version"], "1");
    assert_eq!(json["id"], "E12S4/NG-1");
    assert_eq!(json["owner_id"], "E12S4");
    assert_eq!(json["version"], 2);
}

#[test]
fn test_cli_constraint_get_resolution_and_inheritance() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);

    // Create epic E12 with RH-1
    let epic_constraints = "  - id: RH-1\n    kind: rabbit_hole\n    text: len == 0 does not mean empty\n";
    create_epic(root, "E12", "active", 1, epic_constraints);

    // Create story E12S4 with NG-1
    let story_constraints = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    create_story(root, "E12S4", "draft", 1, story_constraints);

    // Populate cache by syncing
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync", "--rebuild"]).assert().success();

    // 1. qdev get E12S4/NG-1 resolves bare constraint
    let mut get_cmd1 = Command::cargo_bin("qdev").unwrap();
    let assert1 = get_cmd1
        .current_dir(root)
        .args(["get", "E12S4/NG-1"])
        .assert()
        .success();
    let stdout1 = String::from_utf8(assert1.get_output().stdout.clone()).unwrap();
    assert!(stdout1.contains("id: E12S4/NG-1"));
    assert!(stdout1.contains("owner: E12S4"));
    assert!(stdout1.contains("kind: no_go"));

    // 2. qdev get story E12S4 --json projects both own and inherited constraints
    let mut get_cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = get_cmd2
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success();
    let stdout2 = String::from_utf8(assert2.get_output().stdout.clone()).unwrap();
    let json2: Value = serde_json::from_str(&stdout2).expect("Valid JSON");
    let constraints = json2["constraints"].as_array().expect("constraints array");
    assert_eq!(constraints.len(), 2);

    let own = constraints.iter().find(|c| c["id"] == "E12S4/NG-1").expect("own constraint");
    assert_eq!(own["kind"], "no_go");
    assert_eq!(own["inherited_from"], Value::Null);

    let inherited = constraints.iter().find(|c| c["id"] == "E12/RH-1").expect("inherited constraint");
    assert_eq!(inherited["kind"], "rabbit_hole");
    assert_eq!(inherited["inherited_from"], "E12");
}

#[test]
fn test_cli_constraint_remove_if_version_mismatch() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    create_story(root, "E12S4", "draft", 2, constraints_yaml);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/NG-1",
            "--if-version",
            "1",
        ])
        .assert()
        .code(5);

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("version: 2"));
    assert!(content.contains("NG-1"));
}

#[test]
fn test_cli_appetite_constraint_remove_and_get() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "draft", 1, "");

    // 1. Add appetite constraint
    let mut add_cmd = Command::cargo_bin("qdev").unwrap();
    add_cmd
        .current_dir(root)
        .args([
            "constraint",
            "add",
            "E12S4",
            "--kind",
            "appetite",
            "--",
            "2 days maximum",
        ])
        .assert()
        .success();

    let content1 = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content1.contains("id: APP-1"));
    assert!(content1.contains("kind: appetite"));

    // 2. Bare get
    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_get = get_cmd
        .current_dir(root)
        .args(["get", "E12S4/APP-1"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert_get.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("id: E12S4/APP-1"));
    assert!(stdout.contains("kind: appetite"));
    assert!(stdout.contains("text: 2 days maximum"));

    // 3. Remove appetite constraint
    let mut rm_cmd = Command::cargo_bin("qdev").unwrap();
    rm_cmd
        .current_dir(root)
        .args([
            "constraint",
            "remove",
            "E12S4/APP-1",
        ])
        .assert()
        .success();

    let content2 = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(!content2.contains("APP-1"));
}
