use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_config_show_json_empty_dir() {
    let temp = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(temp.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert!(val.get("config").is_some(), "Must contain config field");
    assert!(val.get("sources").is_some(), "Must contain sources field");
    assert_eq!(val["config"]["git"]["remote"], "origin");
    assert_eq!(val["sources"]["git.remote"], "default");
}

#[test]
fn test_config_show_json_dual_configs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "DualProject"
default_sprint = 7

[git]
remote = "origin"
integration_branch = "develop"

[[modules]]
id = "proj-mod1"
paths = ["crates/p1/**"]

[[modules]]
id = "proj-mod2"
paths = ["crates/p2/**"]

[[gates]]
id = "gate1"
command = "echo 1"

[[gates]]
id = "gate2"
command = "echo 2"
"#;

    let local_toml = r#"
[project]
name = "LocalOverriddenProject"

[identity]
developer_id = "amelia"

[[gates]]
id = "gate-local"
command = "echo local"
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");

    // Scalar override
    assert_eq!(val["config"]["project"]["name"], "LocalOverriddenProject");
    assert_eq!(val["sources"]["project.name"], ".qdev.local.toml");

    // Project key preserved
    assert_eq!(val["config"]["project"]["default_sprint"], 7);
    assert_eq!(val["sources"]["project.default_sprint"], "qdev.toml");

    // Identity from local
    assert_eq!(val["config"]["identity"]["developer_id"], "amelia");
    assert_eq!(val["sources"]["identity.developer_id"], ".qdev.local.toml");

    // Array wholesale replacement: gates has exactly 1 gate from local
    let gates = val["config"]["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0]["id"], "gate-local");
    assert_eq!(val["sources"]["gates"], ".qdev.local.toml");

    // Modules preserved from project since not in local
    let modules = val["config"]["modules"].as_array().unwrap();
    assert_eq!(modules.len(), 2);
    assert_eq!(val["sources"]["modules"], "qdev.toml");
}

#[test]
fn test_config_show_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "TextProject"
default_sprint = 2
"#;
    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["config", "show"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("TextProject"))
        .stdout(predicate::str::contains("qdev.toml"));
}

#[test]
fn test_schema_violation_project_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let bad_project = r#"
[project]
name = "BadProject"
illegal_field = "bad_value"
"#;
    fs::write(root.join("qdev.toml"), bad_project).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["config", "show", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    let msg = val["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("illegal_field"),
        "Error message must name illegal_field: {}",
        msg
    );
    assert!(
        msg.contains("qdev.toml"),
        "Error message must name qdev.toml: {}",
        msg
    );
}

#[test]
fn test_schema_violation_project_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let bad_project = r#"
[project]
name = "BadProject"
bogus_key = 123
"#;
    fs::write(root.join("qdev.toml"), bad_project).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["config", "show"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("bogus_key"))
        .stderr(predicate::str::contains("qdev.toml"));
}

#[test]
fn test_schema_violation_local_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let valid_project = r#"
[project]
name = "GoodProject"
"#;
    let bad_local = r#"
[preferences]
color = 99999
"#;
    fs::write(root.join("qdev.toml"), valid_project).unwrap();
    fs::write(root.join(".qdev.local.toml"), bad_local).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["config", "show", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    let msg = val["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("color"),
        "Error message must name color: {}",
        msg
    );
    assert!(
        msg.contains(".qdev.local.toml"),
        "Error message must name .qdev.local.toml: {}",
        msg
    );
}

#[test]
fn test_schema_violation_local_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let bad_local = r#"
[identity]
invalid_key = "nope"
"#;
    fs::write(root.join(".qdev.local.toml"), bad_local).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["config", "show"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("invalid_key"))
        .stderr(predicate::str::contains(".qdev.local.toml"));
}

#[test]
fn test_schema_violation_blocks_pulse_command() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let bad_project = r#"
[project]
unknown_field = "fail"
"#;
    fs::write(root.join("qdev.toml"), bad_project).unwrap();

    // Testing that `qdev status --json` fails boot with exit 2 naming key and file
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["status", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    let msg = val["error"]["message"].as_str().unwrap();
    assert!(msg.contains("unknown_field"));
    assert!(msg.contains("qdev.toml"));
}
