use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("failed to run `git {}`: {}", args.join(" "), e));
    assert!(
        output.status.success(),
        "`git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "ImpactTest",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();

    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    let toml = r#"
[project]
name = "ImpactTest"

[identity]
developer_id = "simon"
teams = ["core-platform"]

[git]
remote = "origin"
integration_branch = "main"
branching_mode = "story-branch"
require_clean_tree_in_scope = true

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"

[[modules]]
id = "foundation"
paths = ["crates/foundation/**"]

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]

[[gates]]
id = "lint"
command = "cargo clippy"

[[gates]]
id = "c-abi-round-trip"
command = "./gate.sh"
verifies = ["FR-102"]
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    fs::create_dir_all(root.join("crates/foundation/src")).unwrap();
    fs::create_dir_all(root.join("crates/bridge/src")).unwrap();
    fs::write(root.join("crates/foundation/src/lib.rs"), "// foundation\n").unwrap();
    fs::write(root.join("crates/bridge/src/lib.rs"), "// bridge\n").unwrap();
    fs::write(root.join("README.md"), "# Readme\n").unwrap();

    // Create story E12S4
    let story_1 = r#"---
id: E12S4
title: Core Bridge Layout
status: ready
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
target_modules:
  - bridge
relations:
  traces_to:
    - FR-102
  mitigates:
    - HAZ-14
  governed_by:
    - AD-43
---
# E12S4
"#;
    fs::write(root.join("docs/specs/stories/E12S4.md"), story_1).unwrap();

    // Create story E12S5 (in-progress in bridge)
    let story_2 = r#"---
id: E12S5
title: Active Bridge Buffer
status: in-progress
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
target_modules:
  - bridge
relations:
  depends_on:
    - E12S4
---
# E12S5
"#;
    fs::write(root.join("docs/specs/stories/E12S5.md"), story_2).unwrap();

    // Sync cache
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd
        .current_dir(root)
        .args(["sync", "--rebuild"])
        .assert()
        .success();
}

#[test]
fn test_schema_payload_impact() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["schema", "payload", "impact", "--json"])
        .assert()
        .success();

    let schema: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(schema["title"], "Impact Payload Schema");
    assert!(schema["properties"]["modules"].is_object());
    assert!(schema["properties"]["stories"].is_object());
    assert!(schema["properties"]["dependents"].is_object());
    assert!(schema["properties"]["requirements"].is_object());
    assert!(schema["properties"]["hazards"].is_object());
    assert!(schema["properties"]["adrs"].is_object());
    assert!(schema["properties"]["gates"].is_object());
    assert!(schema["properties"]["cited_entities"].is_object());

    // Compiles with jsonschema validator
    let validator = jsonschema::validator_for(&schema);
    assert!(validator.is_ok(), "schema must compile as valid JSON schema");
}

#[test]
fn test_impact_cli_by_story_id_text_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // 1. Text mode
    let mut cmd_text = Command::cargo_bin("qdev").unwrap();
    cmd_text
        .current_dir(root)
        .args(["impact", "E12S4"])
        .assert()
        .success()
        .stdout(predicates::str::contains("Impact Analysis for Story: E12S4"))
        .stdout(predicates::str::contains("Modules: bridge"))
        .stdout(predicates::str::contains("E12S5 [in-progress]: Active Bridge Buffer"))
        .stdout(predicates::str::contains("E12S5 (depth 1, depends_on)"))
        .stdout(predicates::str::contains("Linked Requirements: FR-102"))
        .stdout(predicates::str::contains("Linked Hazards: HAZ-14"))
        .stdout(predicates::str::contains("Linked ADRs: AD-43"))
        .stdout(predicates::str::contains("Overlapping Gates: c-abi-round-trip"));

    // 2. JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let json_assert = cmd_json
        .current_dir(root)
        .args(["impact", "E12S4", "--json"])
        .assert()
        .success();

    let instance: Value = serde_json::from_slice(&json_assert.get_output().stdout).unwrap();
    assert_eq!(instance["schema_version"], "1");
    assert_eq!(instance["target_story"], "E12S4");
    assert_eq!(instance["modules"], serde_json::json!(["bridge"]));
    assert_eq!(instance["requirements"], serde_json::json!(["FR-102"]));
    assert_eq!(instance["hazards"], serde_json::json!(["HAZ-14"]));
    assert_eq!(instance["adrs"], serde_json::json!(["AD-43"]));
    assert_eq!(instance["gates"], serde_json::json!(["c-abi-round-trip"]));
    assert_eq!(instance["stories"][0]["id"], "E12S5");
    assert_eq!(instance["stories"][0]["status"], "in-progress");
    assert_eq!(instance["dependents"][0]["id"], "E12S5");
    assert_eq!(instance["dependents"][0]["depth"], 1);

    // Validate against schema
    let mut schema_cmd = Command::cargo_bin("qdev").unwrap();
    let schema_output = schema_cmd
        .args(["schema", "payload", "impact", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_output.get_output().stdout).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert!(
        validator.is_valid(&instance),
        "JSON output must validate against payload-impact.json schema"
    );
}

#[test]
fn test_impact_cli_by_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add inline citations to crates/bridge/src/lib.rs
    fs::write(
        root.join("crates/bridge/src/lib.rs"),
        "// Implements [FR-102] governed by [AD-43] with [DW-7f3a]\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["impact", "--paths", "crates/bridge/src/lib.rs", "--json"])
        .assert()
        .success();

    let instance: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(instance["schema_version"], "1");
    assert!(instance["target_story"].is_null());
    assert_eq!(
        instance["target_paths"],
        serde_json::json!(["crates/bridge/src/lib.rs"])
    );
    assert_eq!(instance["modules"], serde_json::json!(["bridge"]));
    assert_eq!(instance["stories"][0]["id"], "E12S5");

    let citations = instance["cited_entities"].as_array().unwrap();
    let cited_strings: Vec<&str> = citations.iter().filter_map(|v| v.as_str()).collect();
    assert!(cited_strings.contains(&"FR-102"));
    assert!(cited_strings.contains(&"AD-43"));
    assert!(cited_strings.contains(&"DW-7f3a"));
}

#[test]
fn test_impact_cli_lease_fallback() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Claim lease for E12S4
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Run impact with no arguments
    let mut impact_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = impact_cmd
        .current_dir(root)
        .args(["impact", "--json"])
        .assert()
        .success();

    let instance: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(instance["target_story"], "E12S4");
    assert_eq!(instance["modules"], serde_json::json!(["bridge"]));
}

#[test]
fn test_impact_cli_missing_target_no_lease_fails_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Run impact with no arguments and no lease
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["impact"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("No target story or paths specified"));

    // In JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert = cmd_json
        .current_dir(root)
        .args(["impact", "--json"])
        .assert()
        .code(2);

    let instance: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(instance["error"]["code"], "usage_error");
}

#[test]
fn test_impact_cli_nonexistent_story_fails_exit_1() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["impact", "NONEXISTENT"])
        .assert()
        .code(1)
        .stderr(predicates::str::contains("Story 'NONEXISTENT' not found"));

    // In JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert = cmd_json
        .current_dir(root)
        .args(["impact", "NONEXISTENT", "--json"])
        .assert()
        .code(1);

    let instance: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(instance["error"]["code"], "entity_not_found");
}
