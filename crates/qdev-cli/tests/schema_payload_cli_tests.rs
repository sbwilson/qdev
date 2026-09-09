//! `qdev schema payload <name>` CLI tests (spec-1-13): text/JSON output for the three live
//! payload kinds (`story`, `error`, `validate`), unknown/deferred name handling, and round-trip
//! validation of real command output against the printed schema.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
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

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Minimal story + epic fixture sufficient to exercise `qdev get story <id> --json`.
fn write_story_fixture(root: &Path) {
    write_file(
        root,
        "docs/specs/epics/E12.md",
        r#"---
id: E12
title: "Bridge Layer"
status: active
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Goal
"#,
    );

    write_file(
        root,
        "docs/specs/stories/E12S4.md",
        r#"---
id: E12S4
title: "CoreResponse Buffer Layout"
status: ready
version: 3
owners: ["simon", "team:core-platform"]
epic_id: E12
appetite: small
safety_class: ClassB
target_modules: ["bridge", "foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: "Do not implement Swift decoding"
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Buffers round-trip.
"#,
    );
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
// Happy path: text + JSON mode for each live payload kind
// ---------------------------------------------------------------------------

#[test]
fn test_schema_payload_story_text_and_json() {
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "story"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Story Payload Schema"))
        .stdout(predicate::str::contains("schema_version"));

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "story", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value =
        serde_json::from_slice(&assert.get_output().stdout).expect("stdout must be valid JSON");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["title"], "Story Payload Schema");
    assert!(val["description"]
        .as_str()
        .unwrap()
        .contains("schema_version"));
}

#[test]
fn test_schema_payload_error_text_and_json() {
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "error"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Error Payload Schema"));

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "error", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["title"], "Error Payload Schema");

    // Matches JsonErrorEnvelope: schema_version, error{code, message, optional details}.
    let required = val["required"].as_array().unwrap();
    assert!(required.contains(&Value::String("schema_version".to_string())));
    assert!(required.contains(&Value::String("error".to_string())));
    let error_props = &val["properties"]["error"]["properties"];
    assert!(error_props["code"].is_object());
    assert!(error_props["message"].is_object());
    assert!(error_props["details"].is_object());
    let error_required = val["properties"]["error"]["required"].as_array().unwrap();
    assert!(error_required.contains(&Value::String("code".to_string())));
    assert!(error_required.contains(&Value::String("message".to_string())));
    assert!(!error_required.contains(&Value::String("details".to_string())));
}

#[test]
fn test_schema_payload_validate_text_and_json() {
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "validate"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Validate Payload Schema"));

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "validate", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["title"], "Validate Payload Schema");
}

// ---------------------------------------------------------------------------
// Unknown / deferred payload names
// ---------------------------------------------------------------------------

#[test]
fn test_schema_payload_unknown_name_is_usage_error() {
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "bogus"])
        .assert()
        .failure()
        .code(2);
    let output = assert.get_output();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(stderr.contains("story"));
    assert!(stderr.contains("error"));
    assert!(stderr.contains("validate"));
}

#[test]
fn test_schema_payload_unknown_name_json_mode() {
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "bogus", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_schema_payload_deferred_names_are_usage_errors() {
    for name in ["context", "next", "gate_run"] {
        let assert = Command::cargo_bin("qdev")
            .unwrap()
            .args(["schema", "payload", name])
            .assert()
            .failure()
            .code(2);
        let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
        assert!(stderr.contains("story"));
        assert!(stderr.contains("error"));
        assert!(stderr.contains("validate"));
    }
}

#[test]
fn test_schema_payload_missing_name_is_usage_error() {
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload"])
        .assert()
        .failure()
        .code(2);
    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("story"));
    assert!(stderr.contains("error"));
    assert!(stderr.contains("validate"));
}

#[test]
fn test_schema_payload_name_is_case_insensitive() {
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "STORY"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Story Payload Schema"));
}

// ---------------------------------------------------------------------------
// Existing `qdev schema <entity-kind>` path is unaffected
// ---------------------------------------------------------------------------

#[test]
fn test_schema_entity_kind_path_unaffected() {
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "story"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Story Frontmatter Schema"));
}

#[test]
fn test_schema_entity_kind_rejects_extra_positional_argument() {
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "story", "extra_garbage"])
        .assert()
        .failure()
        .code(2);
}

// ---------------------------------------------------------------------------
// Round-trip: real command output validates cleanly against the printed schema
// ---------------------------------------------------------------------------

#[test]
fn test_round_trip_story_payload_against_get_story_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "story", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let get_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["get", "story", "E12S4", "--json"])
        .assert()
        .success();
    let instance: Value = serde_json::from_slice(&get_assert.get_output().stdout).unwrap();

    validate_against_schema(&schema, &instance);
}

#[test]
fn test_round_trip_validate_payload_against_validate_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "validate", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let validate_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert();
    let output = validate_assert.get_output();
    let instance: Value = serde_json::from_slice(&output.stdout).unwrap();

    validate_against_schema(&schema, &instance);
}

#[test]
fn test_round_trip_error_payload_against_real_error_output() {
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "error", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    // A real error envelope: unknown entity-schema kind, JSON mode.
    let error_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "invalid_kind", "--json"])
        .assert()
        .failure()
        .code(2);
    let instance: Value = serde_json::from_slice(&error_assert.get_output().stdout).unwrap();

    validate_against_schema(&schema, &instance);
}
