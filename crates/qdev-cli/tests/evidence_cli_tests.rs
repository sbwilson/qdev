use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let _ = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Simon"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "simon@example.com"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(root)
        .status();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
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

    append_to_qdev_toml(root, "[git]\nintegration_branch = \"main\"\n");

    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(root)
        .status();
}

fn append_to_qdev_toml(root: &Path, content: &str) {
    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push('\n');
    toml.push_str(content);
    fs::write(toml_path, toml).unwrap();
}

fn create_sample_story(root: &Path, story_id: &str) {
    let story_path = root.join(format!("docs/specs/stories/{}.md", story_id));
    fs::create_dir_all(story_path.parent().unwrap()).unwrap();
    fs::write(
        &story_path,
        format!(
            r#"---
id: {}
title: Test Story {}
status: in_progress
version: 1
safety_class: ClassB
owners: [simon]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
# Story {}
## Acceptance Criteria
- Done.
"#,
            story_id, story_id, story_id
        ),
    )
    .unwrap();

    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", &format!("Add story {}", story_id)])
        .current_dir(root)
        .status();
}

#[test]
fn test_gate_run_writes_evidence_and_receipt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "c-abi-round-trip"
command = "echo '18 tests passed'"
verifies = ["FR-102", "HAZ-01"]
"#,
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["gate", "run", "c-abi-round-trip"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("[PASS] c-abi-round-trip | 18 tests passed | "));
    assert!(stdout.contains("evidence docs/state/evidence/_workspace/"));

    // Extract the evidence path from the output
    let ev_marker = "evidence ";
    let ev_start = stdout.find(ev_marker).unwrap() + ev_marker.len();
    let ev_rel_path = stdout[ev_start..].trim_end();
    let ev_file = root.join(ev_rel_path);
    assert!(
        ev_file.is_file(),
        "Evidence file must exist at {}",
        ev_file.display()
    );

    let content = fs::read_to_string(&ev_file).unwrap();
    let val: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["gate"], "c-abi-round-trip");
    assert!(val.get("story").unwrap().is_null());
    assert_eq!(val["status"], "pass");
    assert_eq!(val["exit_code"], 0);
    assert_eq!(val["summary"], "18 tests passed");
    assert_eq!(val["verifies"], serde_json::json!(["FR-102", "HAZ-01"]));
    assert_eq!(val["skipped_locally"], false);
    assert_eq!(val["output_sha256"].as_str().unwrap().len(), 64);
    assert!(val["run_by"]["type"].is_string());
    assert!(val["run_by"]["id"].is_string());
}

#[test]
fn test_gate_run_with_story_and_collision_suffixing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E12S4");

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "lint"
command = "echo 'all good'"
verifies = ["FR-105"]
"#,
    );

    // First run on E12S4
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["gate", "run", "lint", "--story", "E12S4"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "evidence docs/state/evidence/E12S4/",
        ));

    let ev_dir = root.join("docs/state/evidence/E12S4");
    let files: Vec<_> = fs::read_dir(&ev_dir)
        .unwrap()
        .map(|r| r.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(files.len(), 1);
    let first_file = files[0].clone();
    assert!(first_file.ends_with("-lint.json"));
    assert!(!first_file.contains("-lint-2.json"));
    let first_content = fs::read_to_string(ev_dir.join(&first_file)).unwrap();

    // Second run on same commit -> creates -2.json
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["gate", "run", "lint", "--story", "E12S4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("-lint-2.json"));

    let files2: Vec<_> = fs::read_dir(&ev_dir)
        .unwrap()
        .map(|r| r.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(files2.len(), 2);
    assert!(files2.iter().any(|f| f.ends_with("-lint-2.json")));

    // First file must be byte-identical and unmodified
    let first_content_after = fs::read_to_string(ev_dir.join(&first_file)).unwrap();
    assert_eq!(first_content, first_content_after);

    // Third run on same commit -> creates -3.json
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    cmd3.current_dir(root)
        .args(["gate", "run", "lint", "--story", "E12S4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("-lint-3.json"));

    let files3: Vec<_> = fs::read_dir(&ev_dir)
        .unwrap()
        .map(|r| r.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(files3.len(), 3);
    assert!(files3.iter().any(|f| f.ends_with("-lint-3.json")));
}

#[test]
fn test_get_story_expand_evidence_text_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E12S4");

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "lint"
command = "echo 'lint pass'"
verifies = ["FR-105"]

[[gates]]
id = "fmt"
command = "echo 'fmt pass'"
"#,
    );

    // Run both gates for E12S4
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "lint", "--story", "E12S4"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "fmt", "--story", "E12S4"])
        .assert()
        .success();

    // Query in text mode with --expand evidence
    let mut cmd_text = Command::cargo_bin("qdev").unwrap();
    let assert_text = cmd_text
        .current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "evidence"])
        .assert()
        .success();

    let text_out = String::from_utf8(assert_text.get_output().stdout.clone()).unwrap();
    assert!(text_out.contains("evidence:\n"));
    assert!(text_out.contains("[PASS] fmt | fmt pass | "));
    assert!(text_out.contains("[PASS] lint | lint pass | "));

    // Query in JSON mode with --expand evidence
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "evidence", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    let ev_arr = val["evidence"].as_array().expect("evidence array expected");
    assert_eq!(ev_arr.len(), 2);
    assert_eq!(ev_arr[0]["gate"], "fmt");
    assert_eq!(ev_arr[1]["gate"], "lint");
    assert_eq!(ev_arr[0]["status"], "pass");
    assert_eq!(ev_arr[1]["status"], "pass");
    assert!(ev_arr[0]["evidence_path"]
        .as_str()
        .unwrap()
        .contains("-fmt.json"));
    assert!(ev_arr[1]["evidence_path"]
        .as_str()
        .unwrap()
        .contains("-lint.json"));
}

#[test]
fn test_get_story_expand_evidence_none_when_no_runs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E12S5");

    // Text mode: evidence: (none)
    let mut cmd_text = Command::cargo_bin("qdev").unwrap();
    cmd_text
        .current_dir(root)
        .args(["get", "story", "E12S5", "--expand", "evidence"])
        .assert()
        .success()
        .stdout(predicate::str::contains("evidence:\n  (none)\n"));

    // JSON mode: "evidence": []
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["get", "story", "E12S5", "--expand", "evidence", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    let ev_arr = val["evidence"].as_array().expect("evidence array expected");
    assert!(ev_arr.is_empty());
}

#[test]
fn test_get_story_expand_evidence_deduplicates_to_latest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E12S4");

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "lint"
command = "echo 'lint run'"
"#,
    );

    // Run lint twice
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "lint", "--story", "E12S4"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "lint", "--story", "E12S4"])
        .assert()
        .success();

    // Verify only 1 entry returned by get --expand evidence
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "evidence", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let ev_arr = val["evidence"].as_array().unwrap();
    assert_eq!(ev_arr.len(), 1);
    assert_eq!(ev_arr[0]["gate"], "lint");
    assert!(ev_arr[0]["evidence_path"]
        .as_str()
        .unwrap()
        .contains("-lint-2.json"));
}

#[test]
fn test_get_expand_bogus_usage_error_includes_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_sample_story(root, "E12S4");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["get", "story", "E12S4", "--expand", "bogus"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "expected one of: relations, constraints, scratch, evidence",
        ));
}
