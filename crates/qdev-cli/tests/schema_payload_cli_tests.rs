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

    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push_str(
        "\n[[modules]]\nid = \"bridge\"\npaths = [\"crates/bridge/**\"]\n\n[[modules]]\nid = \"foundation\"\npaths = [\"crates/foundation/**\"]\n",
    );
    fs::write(toml_path, toml).unwrap();
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

// ---------------------------------------------------------------------------
// Round-trips for the payloads added alongside their commands
// ---------------------------------------------------------------------------

/// `qdev validate --fix-ids --json` emits a renumber report, not a finding list — a different
/// shape from the same command with the same flag. It gets its own schema, and this is the
/// branch nothing exercised.
#[test]
fn test_round_trip_fix_ids_payload_against_fix_ids_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    // A second file declaring E12S4's id, so the renumber branch has work to do.
    let stories_dir = root.join("docs/specs/stories");
    fs::write(
        stories_dir.join("E12S4-dup.md"),
        fs::read_to_string(stories_dir.join("E12S4.md")).unwrap(),
    )
    .unwrap();

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "fix_ids", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let fix_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert();
    let instance: Value = serde_json::from_slice(&fix_assert.get_output().stdout).unwrap();

    assert!(
        !instance["renumbered"].as_array().unwrap().is_empty(),
        "the fixture must actually renumber something, or the items subschema is vacuous"
    );
    assert!(
        instance.get("findings").is_none(),
        "clean runs must omit findings: {instance}"
    );
    validate_against_schema(&schema, &instance);
}

#[test]
fn test_round_trip_fix_ids_payload_with_surviving_findings_against_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    // Duplicate E12S4 so renumber runs.
    let stories_dir = root.join("docs/specs/stories");
    fs::write(
        stories_dir.join("E12S4-dup.md"),
        fs::read_to_string(stories_dir.join("E12S4.md")).unwrap(),
    )
    .unwrap();

    // Add a file with an unregistered target module so an error finding survives.
    write_file(
        root,
        "docs/specs/stories/E12S9.md",
        r#"---
id: E12S9
title: "Unregistered Module Story"
status: draft
version: 1
target_modules: ["unregistered_mod"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#,
    );

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "fix_ids", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let fix_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .failure()
        .code(1);
    let instance: Value = serde_json::from_slice(&fix_assert.get_output().stdout).unwrap();

    assert!(
        !instance["findings"].as_array().unwrap().is_empty(),
        "the payload must include surviving findings: {instance}"
    );
    validate_against_schema(&schema, &instance);
}

#[test]
fn test_round_trip_fix_ids_payload_with_error_against_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    write_file(
        root,
        "docs/specs/stories/E1S1.md",
        r#"---
id: E1S1
title: "Story E1S1"
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
"#,
    );
    let story_content = fs::read_to_string(stories_dir.join("E1S1.md")).unwrap();
    write_file(root, "docs/specs/stories/E1S1-dup.md", &story_content);
    // Occupy the rename target E1S2.md as a directory so renumber fails with rename_target_exists.
    fs::create_dir_all(stories_dir.join("E1S2.md")).unwrap();

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "fix_ids", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let fix_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .failure();
    let instance: Value = serde_json::from_slice(&fix_assert.get_output().stdout).unwrap();

    assert!(
        instance["error"].is_object(),
        "the payload must include error when aborted: {instance}"
    );
    validate_against_schema(&schema, &instance);
}

#[test]
fn test_round_trip_list_payload_against_list_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "list", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let list_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["list", "stories", "--json"])
        .assert()
        .success();
    let instance: Value = serde_json::from_slice(&list_assert.get_output().stdout).unwrap();

    assert!(
        !instance["items"].as_array().unwrap().is_empty(),
        "the fixture must list at least one story"
    );
    validate_against_schema(&schema, &instance);
}

#[test]
fn test_round_trip_sync_payload_against_sync_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "sync", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    for args in [vec!["sync", "--json"], vec!["sync", "--rebuild", "--json"]] {
        let sync_assert = Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args(&args)
            .assert()
            .success();
        let instance: Value = serde_json::from_slice(&sync_assert.get_output().stdout).unwrap();
        validate_against_schema(&schema, &instance);
    }
}

#[test]
fn test_round_trip_doctor_payload_against_doctor_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "doctor", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    let doctor_assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();
    let instance: Value = serde_json::from_slice(&doctor_assert.get_output().stdout).unwrap();

    assert!(
        !instance["sections"].as_array().unwrap().is_empty(),
        "doctor must report at least the cache section"
    );
    validate_against_schema(&schema, &instance);
}

/// Every payload kind the binary advertises must have a schema that compiles. This is the
/// invariant that keeps a newly shipped `--json` payload from being neither schematized nor
/// recorded as deferred.
#[test]
fn test_every_advertised_payload_name_has_a_compilable_schema() {
    for name in [
        "story", "error", "validate", "fix_ids", "list", "sync", "doctor",
    ] {
        let assert = Command::cargo_bin("qdev")
            .unwrap()
            .args(["schema", "payload", name, "--json"])
            .assert()
            .success();
        let schema: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap_or_else(|e| panic!("payload schema '{}' must compile: {}", name, e));
    }
}
