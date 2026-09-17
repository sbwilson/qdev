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

    let mut qdev_toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    qdev_toml.push_str(
        r#"
[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]
"#,
    );
    fs::write(root.join("qdev.toml"), qdev_toml).unwrap();
}

fn write_story(root: &Path, id: &str, title: &str, status: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: {status}
version: 1
owners:
  - simon
target_modules:
  - bridge
appetite: small
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# {id}: {title}
"#
        ),
    )
    .unwrap();

    // Sync to cache
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["sync"]).assert().success();
}

#[test]
fn test_cli_sprint_open_success_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "sprint",
            "open",
            "6",
            "--title",
            "The Rust Core Port",
            "--release",
            "0.1.0",
            "--json",
        ])
        .assert()
        .success();

    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["schema_version"], "1");
    assert_eq!(v["sprint_id"], 6);
    assert_eq!(v["entity_id"], "sprint-6");
    assert_eq!(v["title"], "The Rust Core Port");
    assert_eq!(v["status"], "active");
    assert_eq!(v["release"], "0.1.0");

    let md_path = root.join("docs/state/sprints/sprint-6.md");
    assert!(md_path.is_file());
    let content = fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("title: The Rust Core Port"));
    assert!(content.contains("status: active"));
    assert!(content.contains("release: 0.1.0"));
}

#[test]
fn test_cli_sprint_open_errors() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // 1. Open sprint 6
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "6", "--title", "Port"])
        .assert()
        .success();

    // 2. Duplicate open sprint 6 -> exit 1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "open", "6", "--title", "Duplicate", "--json"])
        .assert()
        .failure()
        .code(1);
    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "sprint_already_exists");

    // 3. Empty title -> exit 1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "open", "7", "--title", "  ", "--json"])
        .assert()
        .failure()
        .code(1);
    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "empty_title");
}

#[test]
fn test_cli_sprint_assign_and_get() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E12S4", "Buffer Layout", "ready");
    write_story(root, "E11S9", "Transport Bridge", "in_progress");

    // Open sprint 6
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "6", "--title", "Sprint 6"])
        .assert()
        .success();

    let e12s4_before = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e11s9_before = fs::read_to_string(root.join("docs/specs/stories/E11S9.md")).unwrap();

    // Assign stories to sprint 6
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "assign", "6", "E12S4", "E11S9", "--json"])
        .assert()
        .success();

    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["sprint_id"], 6);
    assert_eq!(v["assigned_stories"], serde_json::json!(["E12S4", "E11S9"]));

    // Story markdown files and IDs must remain immutable!
    let e12s4_after = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e11s9_after = fs::read_to_string(root.join("docs/specs/stories/E11S9.md")).unwrap();
    assert_eq!(e12s4_before, e12s4_after);
    assert_eq!(e11s9_before, e11s9_after);

    // Get sprint projection via `qdev get sprint 6 --json`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["get", "sprint", "6", "--json"])
        .assert()
        .success();

    let proj: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(proj["id"], "sprint-6");
    assert_eq!(proj["kind"], "sprint");
    let assignments = proj["assignments"].as_array().unwrap();
    assert_eq!(assignments.len(), 2);
    let assigned_ids: Vec<&str> = assignments
        .iter()
        .map(|a| a["story"].as_str().unwrap())
        .collect();
    assert!(assigned_ids.contains(&"E12S4"));
    assert!(assigned_ids.contains(&"E11S9"));
    assert_eq!(proj["status_counts"]["ready"], 1);
    assert_eq!(proj["status_counts"]["in_progress"], 1);

    // Also verify get via numeric ID: `qdev get 6 --json`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["get", "6", "--json"])
        .assert()
        .success();

    // List stories filtered by sprint: `qdev list stories --sprint 6`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--sprint", "6", "--json"])
        .assert()
        .success();
    let list_val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let rows = list_val["items"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn test_cli_sprint_assign_errors() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E12S4", "Buffer Layout", "ready");

    // Assign to nonexistent sprint -> exit 2
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "assign", "999", "E12S4", "--json"])
        .assert()
        .failure()
        .code(2);
    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "sprint_not_found");

    // Open sprint 6
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "6", "--title", "Sprint 6"])
        .assert()
        .success();

    // Assign nonexistent story -> exit 2
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "assign", "6", "NONEXISTENT", "--json"])
        .assert()
        .failure()
        .code(2);
    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["error"]["code"], "story_not_found");
}

#[test]
fn test_cli_sprint_close_carry_over() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E12S4", "Buffer Layout", "ready");
    write_story(root, "E12S5", "Memory Pool", "done");

    // Open sprint 5
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "5", "--title", "Sprint 5"])
        .assert()
        .success();

    // Assign both stories to sprint 5
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "assign", "5", "E12S4", "E12S5"])
        .assert()
        .success();

    // Open sprint 6
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "6", "--title", "Sprint 6"])
        .assert()
        .success();

    let e12s4_before = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e12s5_before = fs::read_to_string(root.join("docs/specs/stories/E12S5.md")).unwrap();

    // Close sprint 5 with carry-over to 6: `qdev sprint close 5 --carry-over 6`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "close", "5", "--carry-over", "6", "--json"])
        .assert()
        .success();

    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["sprint_id"], 5);
    assert_eq!(v["status"], "completed");
    assert_eq!(v["carried_stories"], serde_json::json!(["E12S4"]));
    assert_eq!(v["carry_over_target"], 6);

    // Verify sprint 5 frontmatter status is completed
    let s5_content = fs::read_to_string(root.join("docs/state/sprints/sprint-5.md")).unwrap();
    assert!(s5_content.contains("status: completed"));
    assert!(s5_content.contains("completed_at:"));

    // Verify sprint 6 contains E12S4 with carried_from: 5, but NOT E12S5
    let s6_content = fs::read_to_string(root.join("docs/state/sprints/sprint-6.md")).unwrap();
    assert!(s6_content.contains("story: E12S4"));
    assert!(s6_content.contains("carried_from: 5"));
    assert!(!s6_content.contains("story: E12S5"));

    // Verify story markdown files and IDs were NOT altered
    let e12s4_after = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e12s5_after = fs::read_to_string(root.join("docs/specs/stories/E12S5.md")).unwrap();
    assert_eq!(e12s4_before, e12s4_after);
    assert_eq!(e12s5_before, e12s5_after);
}

#[test]
fn test_cli_sprint_fallback_resolution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // 0 active sprints -> `qdev sprint close --carry-over 7` exits 2
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "close", "--carry-over", "7", "--json"])
        .assert()
        .failure()
        .code(2);

    // Open sprint 6 (single active sprint)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "6", "--title", "Sprint 6"])
        .assert()
        .success();

    // Open target sprint 7
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "7", "--title", "Sprint 7"])
        .assert()
        .success();

    // Now 2 active sprints (6 and 7). Close without explicit sprint -> exit 2 (ambiguous)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "close", "--carry-over", "8", "--json"])
        .assert()
        .failure()
        .code(2);

    // Set default_sprint = 6 in qdev.toml
    let mut qdev_toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    qdev_toml = qdev_toml.replace("[project]", "[project]\ndefault_sprint = 6");
    fs::write(root.join("qdev.toml"), qdev_toml).unwrap();

    // Open sprint 8 as carry-over target
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "8", "--title", "Sprint 8"])
        .assert()
        .success();

    // Now close without explicit sprint resolves default_sprint (6) -> marks 6 completed
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["sprint", "close", "--carry-over", "8", "--json"])
        .assert()
        .success();
    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["sprint_id"], 6);
    assert_eq!(v["status"], "completed");
}

#[test]
fn test_cli_duplicate_active_sprint_validation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E12S4", "Buffer Layout", "ready");

    // Open sprint 5 and 6
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "5", "--title", "Sprint 5"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "open", "6", "--title", "Sprint 6"])
        .assert()
        .success();

    // Assign E12S4 to sprint 5
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "assign", "5", "E12S4"])
        .assert()
        .success();

    // Assign E12S4 to sprint 6 as well
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["sprint", "assign", "6", "E12S4"])
        .assert()
        .success();

    // `qdev validate` must report duplicate_active_sprint_assignment and exit 1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);

    let v: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let findings = v["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|f| f["code"] == "duplicate_active_sprint_assignment" && f["severity"] == "error"));
}
