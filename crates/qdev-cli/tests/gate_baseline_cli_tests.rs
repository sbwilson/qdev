use std::fs;
use std::path::Path;

use assert_cmd::Command;
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

fn validate_against_schema(schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("schema must compile");
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "instance failed schema validation: {:?}\nschema: {}\ninstance: {}",
        errors,
        schema,
        instance
    );
}

// ---------------------------------------------------------------------------
// 1. Rejection tests (Unknown gate, non-ratchet gate, invalid value)
// ---------------------------------------------------------------------------

#[test]
fn test_baseline_unknown_gate_rejected_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "nonexistent"])
        .assert()
        .failure()
        .code(2);
    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("nonexistent"));
}

#[test]
fn test_baseline_non_ratchet_gate_rejected_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "lint"
command = "echo pass"
kind = "check"
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "lint", "--set"])
        .assert()
        .failure()
        .code(2);
    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("not a ratchet gate"));
}

// ---------------------------------------------------------------------------
// 2. Inspect missing baseline (text and JSON)
// ---------------------------------------------------------------------------

#[test]
fn test_baseline_inspect_missing_text_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 10"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    let text_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count"])
        .assert()
        .success()
        .code(0);
    let stdout = std::str::from_utf8(&text_assert.get_output().stdout).unwrap();
    assert!(stdout.contains("No baseline recorded for gate 'warning-count' on branch 'main'"));

    // JSON inspection when absent
    let json_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&json_assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["gate"], "warning-count");
    assert_eq!(val["branch"], "main");
    assert_eq!(val["exists"], false);

    // Validate against schema
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "gate_baseline", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    validate_against_schema(&schema, &val);
}

// ---------------------------------------------------------------------------
// 3. Set baseline via --value and inspect (text and JSON)
// ---------------------------------------------------------------------------

#[test]
fn test_baseline_set_via_value_and_inspect() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 10"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    // Set baseline: `qdev gate baseline warning-count --set --value 10`
    let set_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "gate",
            "baseline",
            "warning-count",
            "--set",
            "--value",
            "10",
            "--author-type",
            "human",
            "--author-id",
            "simon",
        ])
        .assert()
        .success()
        .code(0);
    let set_stdout = std::str::from_utf8(&set_assert.get_output().stdout).unwrap();
    assert!(set_stdout.contains("Recorded baseline for gate 'warning-count' on branch 'main': value 10"));

    // Verify baseline file was written
    let baseline_file = root.join("docs/state/baselines/main/warning-count.json");
    assert!(baseline_file.exists());
    let content = fs::read_to_string(&baseline_file).unwrap();
    let file_json: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(file_json["gate"], "warning-count");
    assert_eq!(file_json["metric"], "warnings");
    assert_eq!(file_json["direction"], "must_not_increase");
    assert_eq!(file_json["value"], 10.0);
    assert_eq!(file_json["author"]["type"], "human");
    assert_eq!(file_json["author"]["id"], "simon");

    // Inspect text
    let inspect_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count"])
        .assert()
        .success()
        .code(0);
    let inspect_stdout = std::str::from_utf8(&inspect_assert.get_output().stdout).unwrap();
    assert!(inspect_stdout.contains("Baseline for gate 'warning-count' (branch 'main'):"));
    assert!(inspect_stdout.contains("metric: warnings"));
    assert!(inspect_stdout.contains("direction: must_not_increase"));
    assert!(inspect_stdout.contains("value: 10"));
    assert!(inspect_stdout.contains("author: human:simon"));

    // Inspect JSON
    let json_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&json_assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["gate"], "warning-count");
    assert_eq!(val["branch"], "main");
    assert_eq!(val["exists"], true);
    assert_eq!(val["metric"], "warnings");
    assert_eq!(val["direction"], "must_not_increase");
    assert_eq!(val["value"], 10.0);
    assert_eq!(val["author"]["id"], "simon");

    // Schema validation
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "gate_baseline", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    validate_against_schema(&schema, &val);
}

// ---------------------------------------------------------------------------
// 4. Set baseline via running the gate
// ---------------------------------------------------------------------------

#[test]
fn test_baseline_set_via_run() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 42"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "gate",
            "baseline",
            "warning-count",
            "--set",
            "--json",
            "--author-type",
            "human",
            "--author-id",
            "simon",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["exists"], true);
    assert_eq!(val["value"], 42.0);

    let baseline_file = root.join("docs/state/baselines/main/warning-count.json");
    assert!(baseline_file.exists());
}

#[test]
fn test_baseline_set_via_run_fails_if_command_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "failing-gate"
command = "sh -c 'exit 1'"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "failing-gate", "--set"])
        .assert()
        .failure()
        .code(1);
}

// ---------------------------------------------------------------------------
// 5. Gate run regression and non-regression CLI execution
// ---------------------------------------------------------------------------

#[test]
fn test_gate_run_ratchet_regression_cli() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 15"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    // Record baseline at 10
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--set", "--value", "10"])
        .assert()
        .success();

    // Running gate with output 15 should regress and exit 1
    let run_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "warning-count"])
        .assert()
        .failure()
        .code(1);
    let stdout = std::str::from_utf8(&run_assert.get_output().stdout).unwrap();
    assert!(stdout.contains("[FAIL] warning-count (exit 1)"));
    assert!(stdout.contains("ratchet regression: warnings increased from 10 to 15 (delta: +5)"));
}

#[test]
fn test_gate_run_ratchet_no_regression_cli() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 10"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    // Record baseline at 12
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--set", "--value", "12"])
        .assert()
        .success();

    // Running gate with output 10 should pass and report delta -2
    let run_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "warning-count"])
        .assert()
        .success()
        .code(0);
    let stdout = std::str::from_utf8(&run_assert.get_output().stdout).unwrap();
    assert!(stdout.contains("[PASS] warning-count"));
    assert!(stdout.contains("ratchet passed: warnings is 10 (baseline: 12, delta: -2)"));
}

// ---------------------------------------------------------------------------
// 6. Sprint close ratchet snapshotting into release frontmatter
// ---------------------------------------------------------------------------

#[test]
fn test_sprint_close_snapshots_ratchet_baselines_into_release() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // 1. Add ratchet gate to config
    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 10"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    // 2. Set baseline value 10
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--set", "--value", "10"])
        .assert()
        .success();

    // 3. Create release file docs/state/releases/0.1.0.md
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::write(
        root.join("docs/state/releases/0.1.0.md"),
        r#"---
id: 0.1.0
title: "Release 0.1.0"
status: planned
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# Release 0.1.0
"#,
    )
    .unwrap();

    // 4. Open sprint 5 linked to release 0.1.0
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint",
            "open",
            "5",
            "--title",
            "Sprint 5",
            "--release",
            "0.1.0",
        ])
        .assert()
        .success();

    // 5. Close sprint 5
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "close", "5"])
        .assert()
        .success()
        .code(0);

    // 6. Verify release frontmatter contains baseline_snapshot
    let release_content = fs::read_to_string(root.join("docs/state/releases/0.1.0.md")).unwrap();
    assert!(release_content.contains("baseline_snapshot:"));
    assert!(release_content.contains("warning-count: 10.0") || release_content.contains("warning-count: 10"));

    // Verify release validates against schema
    let fm: Value = qdev_core::extract_frontmatter(&release_content).unwrap();
    assert!(fm["baseline_snapshot"].is_object());
    assert_eq!(fm["baseline_snapshot"]["warning-count"], 10.0);
    qdev_core::validate_value_detailed(qdev_core::EntityKind::Release, &fm)
        .expect("release frontmatter must pass schema validation");
}

#[test]
fn test_sprint_close_without_release_skips_snapshot() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 10"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--set", "--value", "10"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "open", "5", "--title", "Sprint 5"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "close", "5"])
        .assert()
        .success()
        .code(0);
}

#[test]
fn test_gate_baseline_set_fails_exit_1_when_no_numeric_metric() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "text-only-ratchet"
command = "echo not a number"
kind = "ratchet"
metric = "score"
direction = "must_not_increase"
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "text-only-ratchet", "--set"])
        .assert()
        .failure()
        .code(1);

    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(
        stderr.contains("did not produce a numeric metric") || stderr.contains("no_metric"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_gate_baseline_value_without_set_fails_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "warning-count"
command = "echo 10"
kind = "ratchet"
metric = "warnings"
direction = "must_not_increase"
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "baseline", "warning-count", "--value", "10"])
        .assert()
        .failure()
        .code(2);

    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(
        stderr.contains("--value requires --set flag"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_gate_run_fails_exit_1_when_ratchet_produces_no_numeric_metric() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    append_to_qdev_toml(
        root,
        r#"
[[gates]]
id = "text-only-ratchet"
command = "echo all tests passed"
kind = "ratchet"
metric = "coverage"
direction = "must_not_decrease"
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "text-only-ratchet"])
        .assert()
        .failure()
        .code(1);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(
        stdout.contains("did not produce a numeric metric"),
        "stdout: {}",
        stdout
    );
}

