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

#[test]
fn test_config_show_json_exposes_qdev_module_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "ModuleProject"

[[modules]]
id = "core"
paths = ["crates/core/**"]
layer = 1

[[modules]]
id = "cli"
paths = ["crates/cli/**", "crates/cli/src/**"]
layer = 2
may_depend_on = ["core"]
"#;
    fs::write(root.join("qdev.toml"), project_toml).unwrap();

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

    // Check uppercase QDEV_MODULE_PATHS
    let qdev_module_paths_upper = &val["QDEV_MODULE_PATHS"];
    assert!(
        qdev_module_paths_upper.is_object(),
        "QDEV_MODULE_PATHS must be a JSON object"
    );
    assert_eq!(
        qdev_module_paths_upper["core"],
        serde_json::json!(["crates/core/**"])
    );
    assert_eq!(
        qdev_module_paths_upper["cli"],
        serde_json::json!(["crates/cli/**", "crates/cli/src/**"])
    );

    // Check lowercase qdev_module_paths
    let qdev_module_paths = &val["qdev_module_paths"];
    assert!(
        qdev_module_paths.is_object(),
        "qdev_module_paths must be a JSON object"
    );
    assert_eq!(
        qdev_module_paths["core"],
        serde_json::json!(["crates/core/**"])
    );
    assert_eq!(
        qdev_module_paths["cli"],
        serde_json::json!(["crates/cli/**", "crates/cli/src/**"])
    );
}

#[test]
fn test_config_duplicate_module_id_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let bad_project = r#"
[project]
name = "BadProject"

[[modules]]
id = "core"
paths = ["crates/core1/**"]

[[modules]]
id = "core"
paths = ["crates/core2/**"]
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

    assert_eq!(val["error"]["code"], "usage_error");
    let msg = val["error"]["message"].as_str().unwrap();
    assert!(msg.contains("duplicate module id 'core'"));
    assert_eq!(val["error"]["details"]["key"], "modules.id");
    assert_eq!(val["error"]["details"]["file"], "qdev.toml");
}

#[test]
fn test_config_empty_paths_array_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let bad_project = r#"
[project]
name = "BadProject"

[[modules]]
id = "core"
paths = []
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

    assert_eq!(val["error"]["code"], "usage_error");
    assert_eq!(val["error"]["details"]["key"], "modules.paths");
    assert_eq!(val["error"]["details"]["file"], "qdev.toml");
}

#[test]
fn test_doctor_reports_module_glob_unmatched_warning_and_exits_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create a real file for mod-real
    fs::create_dir_all(root.join("crates/real/src")).unwrap();
    fs::write(root.join("crates/real/src/lib.rs"), "// real module").unwrap();

    // Append modules to qdev.toml: one matched, one unmatched
    let mut config_str = fs::read_to_string(root.join("qdev.toml")).unwrap();
    config_str.push_str(
        r#"
[[modules]]
id = "real"
paths = ["crates/real/**"]

[[modules]]
id = "phantom"
paths = ["nonexistent/**"]
"#,
    );
    fs::write(root.join("qdev.toml"), config_str).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    let sections = val["sections"].as_array().unwrap();
    let validation_section = sections
        .iter()
        .find(|s| s["name"] == "validation")
        .expect("validation section must exist");

    assert_eq!(validation_section["status"], "ok");
    assert_eq!(validation_section["finding_count"], 1);
    assert_eq!(
        validation_section["findings_by_code"]["module_glob_unmatched"],
        1
    );
}

#[test]
fn test_validate_reports_module_glob_unmatched_warning_and_exits_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create a real file for mod-real
    fs::create_dir_all(root.join("crates/real/src")).unwrap();
    fs::write(root.join("crates/real/src/lib.rs"), "// real module").unwrap();

    // Append modules to qdev.toml: one matched, one unmatched
    let mut config_str = fs::read_to_string(root.join("qdev.toml")).unwrap();
    config_str.push_str(
        r#"
[[modules]]
id = "real"
paths = ["crates/real/**"]

[[modules]]
id = "phantom"
paths = ["nonexistent/**"]
"#,
    );
    fs::write(root.join("qdev.toml"), config_str).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    let findings = val["findings"]
        .as_array()
        .expect("findings array must be present");
    let unmatched = findings
        .iter()
        .find(|f| f["code"] == "module_glob_unmatched")
        .expect("module_glob_unmatched finding must be emitted");

    assert_eq!(unmatched["severity"], "warning");
    assert_eq!(unmatched["path"], "qdev.toml");
    let msg = unmatched["message"].as_str().unwrap();
    assert!(msg.contains("phantom"));
    assert!(msg.contains("nonexistent/**"));
}

#[test]
fn test_validate_reports_target_module_not_registered_and_exits_1() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Register module "core" with real matching path
    fs::create_dir_all(root.join("crates/core")).unwrap();
    fs::write(root.join("crates/core/lib.rs"), "// core").unwrap();

    let mut config_str = fs::read_to_string(root.join("qdev.toml")).unwrap();
    config_str.push_str(
        r#"
[[modules]]
id = "core"
paths = ["crates/core/**"]
"#,
    );
    fs::write(root.join("qdev.toml"), config_str).unwrap();

    // Create story targeting unregistered module "ghost"
    let story_path = root.join("docs/specs/stories/E12S1.md");
    fs::create_dir_all(story_path.parent().unwrap()).unwrap();
    fs::write(
        &story_path,
        r#"---
id: E12S1
title: "Unregistered target story"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
target_modules: ["ghost"]
---

## Acceptance Criteria
- AC.
"#,
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
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    let findings = val["findings"].as_array().unwrap();
    let unregistered_finding = findings
        .iter()
        .find(|f| f["code"] == "target_module_not_registered")
        .expect("target_module_not_registered finding must be emitted");

    assert_eq!(unregistered_finding["severity"], "error");
    let msg = unregistered_finding["message"].as_str().unwrap();
    assert!(msg.contains("ghost"));
}
