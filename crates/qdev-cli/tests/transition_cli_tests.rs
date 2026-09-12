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

fn write_story_file(root: &Path, id: &str, content: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{}.md", id)), content).unwrap();
}

fn write_dw_file(root: &Path, id: &str, content: &str) {
    let dir = root.join("docs/state/dw");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{}.md", id)), content).unwrap();
}

#[test]
fn test_transition_happy_path_text_and_json() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "CoreResponse Buffer Layout"
status: in-progress
version: 3
owners: ["simon"]
epic_id: E12
appetite: small
target_modules: ["bridge"]
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

    // 1. Text mode transition to review
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args(["transition", "story", "E12S4", "review"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Transitioned story E12S4 in-progress -> review (version 4)",
        ));

    // Verify file content updated
    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("status: review"));
    assert!(content.contains("version: 4"));

    // 2. JSON mode transition to done
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S4", "done", "--json"])
        .assert()
        .success();

    let stdout_bytes = assert_json.get_output().stdout.clone();
    let json_val: Value = serde_json::from_slice(&stdout_bytes).expect("must parse envelope JSON");
    assert_eq!(json_val["schema_version"], "1");
    assert_eq!(json_val["id"], "E12S4");
    assert_eq!(json_val["from_status"], "review");
    assert_eq!(json_val["to_status"], "done");
    assert_eq!(json_val["version"], 5);
    assert_eq!(json_val["closed_dw"], serde_json::json!([]));
}

#[test]
fn test_transition_draft_to_ready_readiness_failure_exit_1() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    // 1. Missing AC section
    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: "Draft Story"
status: draft
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Summary
Missing acceptance criteria.
"#,
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "ready"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("readiness_criteria_unmet"));

    // JSON mode error envelope
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "ready", "--json"])
        .assert()
        .code(1);

    let json_val: Value = serde_json::from_slice(&assert_json.get_output().stdout)
        .expect("must parse error envelope");
    assert_eq!(json_val["error"]["code"], "readiness_criteria_unmet");

    // 2. Missing appetite
    write_story_file(
        tmp.path(),
        "E12S2",
        r#"---
id: E12S2
title: "Draft Story 2"
status: draft
version: 1
target_modules: ["bridge"]
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

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(tmp.path())
        .args(["transition", "story", "E12S2", "ready", "--json"])
        .assert()
        .code(1);

    // 3. Missing target_modules
    write_story_file(
        tmp.path(),
        "E12S3",
        r#"---
id: E12S3
title: "Draft Story 3"
status: draft
version: 1
appetite: small
target_modules: []
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

    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    cmd3.current_dir(tmp.path())
        .args(["transition", "story", "E12S3", "ready", "--json"])
        .assert()
        .code(1);
}

#[test]
fn test_transition_ready_to_in_progress_blocked_refusal_exit_3() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    // Story E12S1 is still in draft / ready
    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: "Prerequisite Story"
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
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

    write_story_file(
        tmp.path(),
        "E12S2",
        r#"---
id: E12S2
title: "Dependent Story"
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
relations:
  depends_on:
    - E12S1
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

    // Sync cache so dependencies are indexed
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd
        .current_dir(tmp.path())
        .args(["sync"])
        .assert()
        .success();

    // Transition E12S2 to in-progress -> must be blocked
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args(["transition", "story", "E12S2", "in-progress"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("story_blocked"))
        .stderr(predicate::str::contains("E12S1"));

    // In JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S2", "in-progress", "--json"])
        .assert()
        .code(3);

    let json_val: Value = serde_json::from_slice(&assert_json.get_output().stdout)
        .expect("must parse error envelope");
    assert_eq!(json_val["error"]["code"], "story_blocked");
    assert_eq!(
        json_val["error"]["details"]["blocking_ids"],
        serde_json::json!(["E12S1"])
    );

    // Now transition E12S1 to in-progress, review, then done
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "in-progress"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "review"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "done"])
        .assert()
        .success();

    // Now E12S2 can transition to in-progress!
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S2", "in-progress"])
        .assert()
        .success();
}

#[test]
fn test_transition_terminal_and_backward_justification_exit_3() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: "Story"
status: review
version: 3
appetite: small
target_modules: ["bridge"]
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

    // 1. Abandoned without justification -> exit 3
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "abandoned"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("needs_justification"));

    // In JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "abandoned", "--json"])
        .assert()
        .code(3);
    let json_val: Value = serde_json::from_slice(&assert_json.get_output().stdout)
        .expect("must parse error envelope");
    assert_eq!(json_val["error"]["code"], "needs_justification");

    // 2. Abandoned with justification -> exit 0
    let mut cmd_ok = Command::cargo_bin("qdev").unwrap();
    cmd_ok
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S1",
            "abandoned",
            "--justification",
            "Superseded by bet",
        ])
        .assert()
        .success();

    // 3. Out of terminal state -> exit 1 invalid_transition
    let mut cmd_term = Command::cargo_bin("qdev").unwrap();
    cmd_term
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S1",
            "ready",
            "--justification",
            "Reopening",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("invalid_transition"));
}

#[test]
fn test_backward_transition_with_and_without_justification() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: "Story"
status: review
version: 3
appetite: small
target_modules: ["bridge"]
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

    // review -> in-progress without justification -> exit 3
    let mut cmd_no_just = Command::cargo_bin("qdev").unwrap();
    cmd_no_just
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "in-progress"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("needs_justification"));

    // review -> in-progress with justification -> exit 0
    let mut cmd_with_just = Command::cargo_bin("qdev").unwrap();
    cmd_with_just
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S1",
            "in-progress",
            "--justification",
            "Review rejected: missing test coverage",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(content.contains("status: in-progress"));
    assert!(content.contains("version: 4"));
}

#[test]
fn test_transition_to_done_closes_dw() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_dw_file(
        tmp.path(),
        "DW-7f3a",
        r#"---
id: DW-7f3a
title: "Technical debt item"
status: open
version: 1
origin_story_id: E12S1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Description
Debt description.
"#,
    );

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "Closing Story"
status: review
version: 2
appetite: small
target_modules: ["bridge"]
relations:
  closes_dw:
    - DW-7f3a
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

    // 1. Text mode output mentions closed DW
    let mut cmd_text = Command::cargo_bin("qdev").unwrap();
    cmd_text
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S4", "done"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Transitioned story E12S4 review -> done (version 3)",
        ))
        .stdout(predicate::str::contains("Closed deferred work: DW-7f3a"));

    // Verify DW file was updated
    let dw_content = fs::read_to_string(tmp.path().join("docs/state/dw/DW-7f3a.md")).unwrap();
    assert!(dw_content.contains("status: done"));
    assert!(dw_content.contains("resolution: E12S4"));
}

#[test]
fn test_transition_to_done_closes_dw_json() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_dw_file(
        tmp.path(),
        "DW-7f3a",
        r#"---
id: DW-7f3a
title: "Technical debt item"
status: open
version: 1
origin_story_id: E12S1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Description
Debt description.
"#,
    );

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "Closing Story"
status: review
version: 2
appetite: small
target_modules: ["bridge"]
relations:
  closes_dw:
    - DW-7f3a
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

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert_cmd = cmd
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S4", "done", "--json"])
        .assert()
        .success();

    let json_val: Value =
        serde_json::from_slice(&assert_cmd.get_output().stdout).expect("must parse JSON envelope");
    assert_eq!(json_val["closed_dw"], serde_json::json!(["DW-7f3a"]));
    assert_eq!(json_val["version"], 3);

    let dw_content = fs::read_to_string(tmp.path().join("docs/state/dw/DW-7f3a.md")).unwrap();
    assert!(dw_content.contains("status: done"));
    assert!(dw_content.contains("resolution: E12S4"));
}

#[test]
fn test_transition_author_attribution_override() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: "Story 1"
status: in-progress
version: 1
appetite: small
target_modules: ["bridge"]
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

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S1",
            "review",
            "--author-type",
            "agent",
            "--author-id",
            "bot-1",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(content.contains("status: review"));
    assert!(content.contains("type: agent"));
    assert!(content.contains("id: bot-1"));
}

#[test]
fn test_transition_optimistic_concurrency_if_version() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: "Story 1"
status: in-progress
version: 3
appetite: small
target_modules: ["bridge"]
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

    // Mismatched version -> exit 5
    let mut cmd_mismatch = Command::cargo_bin("qdev").unwrap();
    cmd_mismatch
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S1",
            "review",
            "--if-version",
            "99",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("version_mismatch"));

    // Matching version -> exit 0
    let mut cmd_match = Command::cargo_bin("qdev").unwrap();
    cmd_match
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S1",
            "review",
            "--if-version",
            "3",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(content.contains("status: review"));
    assert!(content.contains("version: 4"));
}

#[test]
fn test_transition_usage_errors_exit_2() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    // Non-story entity kind -> exit 2
    let mut cmd_kind = Command::cargo_bin("qdev").unwrap();
    cmd_kind
        .current_dir(tmp.path())
        .args(["transition", "epic", "E12", "ready"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("usage_error"));

    // Unknown target status -> exit 2
    let mut cmd_status = Command::cargo_bin("qdev").unwrap();
    cmd_status
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "unknown_state"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("usage_error"));
}

#[test]
fn test_transition_outside_workspace_exits_2() {
    let tmp = TempDir::new().unwrap();
    // Do NOT call setup_workspace -> uninitialized directory

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args(["transition", "story", "E12S1", "ready"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("Not a qdev workspace"));
}

#[test]
fn test_backward_transition_acceptance_review_rejection_and_pivot_cli() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "CoreResponse Buffer Layout"
status: review
version: 3
owners: ["simon"]
epic_id: E12
appetite: small
target_modules: ["bridge"]
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

    // AC: Given a story in review, when running qdev transition story E12S4 in-progress without --justification,
    // then the command exits 3 with error code needs_justification.
    let mut cmd_missing_just = Command::cargo_bin("qdev").unwrap();
    cmd_missing_just
        .current_dir(tmp.path())
        .args(["transition", "story", "E12S4", "in-progress"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("needs_justification"));

    // AC: Given a story in review, when running qdev transition story E12S4 in-progress --justification "Failed AC-3",
    // then the transition succeeds with exit 0, a scratchpad entry with kind: transition and seq: 1 is appended
    // to docs/state/scratch/E12S4.jsonl, a DEC- record with decision_type: review_rejection is created in docs/state/decisions/,
    // and both are synced to cache.
    let mut cmd_review_rejection = Command::cargo_bin("qdev").unwrap();
    let assert_rej = cmd_review_rejection
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "in-progress",
            "--justification",
            "Failed AC-3",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Transitioned story E12S4 review -> in-progress (version 4)",
        ))
        .stdout(predicate::str::contains("Recorded decision: DEC-"));

    let stdout = String::from_utf8(assert_rej.get_output().stdout.clone()).unwrap();
    let dec_line = stdout
        .lines()
        .find(|l| l.starts_with("Recorded decision: "))
        .expect("Recorded decision line must be present");
    let dec_id = dec_line
        .strip_prefix("Recorded decision: ")
        .unwrap()
        .trim();
    assert!(dec_id.starts_with("DEC-"));

    // Verify scratchpad file
    let scratch_path = tmp.path().join("docs/state/scratch/E12S4.jsonl");
    assert!(scratch_path.exists());
    let scratch_lines: Vec<String> = fs::read_to_string(&scratch_path)
        .unwrap()
        .lines()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(scratch_lines.len(), 1);
    let entry_val: Value = serde_json::from_str(&scratch_lines[0]).unwrap();
    assert_eq!(entry_val["seq"], 1);
    assert_eq!(entry_val["kind"], "transition");
    assert_eq!(entry_val["text"], "Failed AC-3");

    // Verify DEC- file
    let dec_path = tmp.path().join(format!("docs/state/decisions/{}.md", dec_id));
    assert!(dec_path.exists());
    let dec_content = fs::read_to_string(&dec_path).unwrap();
    let dec_fm = qdev_core::extract_frontmatter(&dec_content).unwrap();
    assert_eq!(dec_fm["id"], dec_id);
    assert_eq!(dec_fm["subject_id"], "E12S4");
    assert_eq!(dec_fm["decision_type"], "review_rejection");
    assert_eq!(dec_fm["ruling"], "Failed AC-3");
    assert_eq!(dec_fm["context"], "review -> in-progress");
    assert!(dec_content.contains("Transition: review -> in-progress"));
    assert!(dec_content.contains("Failed AC-3"));

    // AC: Given a story in in-progress, when running qdev transition story E12S4 ready --justification "Re-scoping",
    // then the transition succeeds with exit 0 and a DEC- record with decision_type: pivot is created.
    let mut cmd_pivot = Command::cargo_bin("qdev").unwrap();
    let assert_pivot = cmd_pivot
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "ready",
            "--justification",
            "Re-scoping",
            "--json",
        ])
        .assert()
        .success();

    let pivot_json: Value =
        serde_json::from_slice(&assert_pivot.get_output().stdout).expect("Valid JSON envelope");
    assert_eq!(pivot_json["schema_version"], "1");
    assert_eq!(pivot_json["id"], "E12S4");
    assert_eq!(pivot_json["from_status"], "in-progress");
    assert_eq!(pivot_json["to_status"], "ready");
    assert_eq!(pivot_json["version"], 5);
    let pivot_dec_id = pivot_json["decision_id"]
        .as_str()
        .expect("decision_id must be in json");
    assert!(pivot_dec_id.starts_with("DEC-"));

    // Verify pivot decision file
    let pivot_dec_path = tmp
        .path()
        .join(format!("docs/state/decisions/{}.md", pivot_dec_id));
    assert!(pivot_dec_path.exists());
    let pivot_dec_content = fs::read_to_string(&pivot_dec_path).unwrap();
    let pivot_fm = qdev_core::extract_frontmatter(&pivot_dec_content).unwrap();
    assert_eq!(pivot_fm["decision_type"], "pivot");
    assert_eq!(pivot_fm["ruling"], "Re-scoping");
    assert_eq!(pivot_fm["context"], "in-progress -> ready");
    assert!(pivot_dec_content.contains("Transition: in-progress -> ready"));
    assert!(pivot_dec_content.contains("Re-scoping"));

    // Validate payload against schema
    let schema_str = include_str!("../../../crates/qdev-core/schemas/payload-transition.json");
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&pivot_json),
        "Transition JSON envelope must validate against payload-transition schema"
    );
}

#[test]
fn test_backward_transition_cli_whitespace_justification_refusal_exit_3() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "Story"
status: review
version: 1
appetite: small
target_modules: ["bridge"]
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

    // Whitespace justification in text mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "in-progress",
            "--justification",
            "   \t\n ",
        ])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("needs_justification"));

    // Whitespace justification in JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "in-progress",
            "--justification",
            "   ",
            "--json",
        ])
        .assert()
        .code(3);

    let err_val: Value =
        serde_json::from_slice(&assert_json.get_output().stdout).expect("Error JSON envelope");
    assert_eq!(err_val["error"]["code"], "needs_justification");

    // Ensure story file remains unchanged
    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("status: review"));
    assert!(content.contains("version: 1"));
}

#[test]
fn test_backward_transition_cli_preserves_leases_and_evidence() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    // Write lease file in .qdev/leases
    let lease_dir = tmp.path().join(".qdev/leases");
    fs::create_dir_all(&lease_dir).unwrap();
    let lease_path = lease_dir.join("E12S4.json");
    let lease_data = r#"{"holder": "simon", "story": "E12S4", "active": true}"#;
    fs::write(&lease_path, lease_data).unwrap();

    // Write evidence file in docs/state/evidence
    let evidence_dir = tmp.path().join("docs/state/evidence");
    fs::create_dir_all(&evidence_dir).unwrap();
    let evidence_path = evidence_dir.join("ev-test.json");
    let evidence_data = r#"{"story": "E12S4", "gate": "unit-tests", "status": "pass"}"#;
    fs::write(&evidence_path, evidence_data).unwrap();

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "Story"
status: review
version: 1
appetite: small
target_modules: ["bridge"]
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

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "in-progress",
            "--justification",
            "Failed review check",
        ])
        .assert()
        .success();

    // AC: Given a story with an active lease and existing evidence, when a backward transition occurs,
    // then no lease is released and all evidence records remain untouched.
    assert!(lease_path.exists());
    assert_eq!(fs::read_to_string(&lease_path).unwrap(), lease_data);

    assert!(evidence_path.exists());
    assert_eq!(fs::read_to_string(&evidence_path).unwrap(), evidence_data);
}

#[test]
fn test_backward_transition_cli_omitted_justification_json_refusal_exit_3() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "Story"
status: review
version: 1
appetite: small
target_modules: ["bridge"]
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

    // Omitted justification in JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "in-progress",
            "--json",
        ])
        .assert()
        .code(3);

    let err_val: Value =
        serde_json::from_slice(&assert_json.get_output().stdout).expect("Error JSON envelope");
    assert_eq!(err_val["schema_version"], "1");
    assert_eq!(err_val["error"]["code"], "needs_justification");
    assert!(err_val["error"]["message"].as_str().unwrap().contains("justification"));

    // Ensure story file remains unchanged
    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("status: review"));
    assert!(content.contains("version: 1"));

    // Ensure no scratchpad or decisions were created
    let scratch = tmp.path().join("docs/state/scratch/E12S4.jsonl");
    assert!(!scratch.exists());
    let decisions = tmp.path().join("docs/state/decisions");
    if decisions.exists() {
        assert_eq!(fs::read_dir(decisions).unwrap().count(), 0);
    }
}

#[test]
fn test_backward_transition_cli_ready_to_draft_and_multistep_review_to_ready() {
    let tmp = TempDir::new().unwrap();
    setup_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: "Story"
status: ready
version: 2
appetite: small
target_modules: ["bridge"]
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

    // 1. ready -> draft in JSON mode
    let mut cmd_draft = Command::cargo_bin("qdev").unwrap();
    let assert_draft = cmd_draft
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "draft",
            "--justification",
            "Revisiting requirements",
            "--json",
        ])
        .assert()
        .success();

    let draft_json: Value =
        serde_json::from_slice(&assert_draft.get_output().stdout).expect("Valid JSON envelope");
    assert_eq!(draft_json["schema_version"], "1");
    assert_eq!(draft_json["id"], "E12S4");
    assert_eq!(draft_json["from_status"], "ready");
    assert_eq!(draft_json["to_status"], "draft");
    assert_eq!(draft_json["version"], 3);
    let draft_dec_id = draft_json["decision_id"].as_str().unwrap();
    assert!(draft_dec_id.starts_with("DEC-"));

    let dec_file = tmp.path().join(format!("docs/state/decisions/{}.md", draft_dec_id));
    let dec_content = fs::read_to_string(&dec_file).unwrap();
    let dec_fm = qdev_core::extract_frontmatter(&dec_content).unwrap();
    assert_eq!(dec_fm["decision_type"], "pivot");
    assert_eq!(dec_fm["ruling"], "Revisiting requirements");
    assert_eq!(dec_fm["context"], "ready -> draft");
    assert!(dec_content.contains("Transition: ready -> draft"));

    // Advance story to review: simulate by updating file directly
    let updated_story = r#"---
id: E12S4
title: "Story"
status: review
version: 5
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#;
    fs::write(tmp.path().join("docs/specs/stories/E12S4.md"), updated_story).unwrap();

    // 2. Multistep backward transition: review -> ready in text mode
    let mut cmd_multistep = Command::cargo_bin("qdev").unwrap();
    let assert_multi = cmd_multistep
        .current_dir(tmp.path())
        .args([
            "transition",
            "story",
            "E12S4",
            "ready",
            "--justification",
            "Failed review completely; re-triaging scope",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Transitioned story E12S4 review -> ready (version 6)",
        ))
        .stdout(predicate::str::contains("Recorded decision: DEC-"));

    let stdout = String::from_utf8(assert_multi.get_output().stdout.clone()).unwrap();
    let dec_line = stdout
        .lines()
        .find(|l| l.starts_with("Recorded decision: "))
        .expect("Recorded decision line must be present");
    let multi_dec_id = dec_line.strip_prefix("Recorded decision: ").unwrap().trim();

    let multi_dec_file = tmp.path().join(format!("docs/state/decisions/{}.md", multi_dec_id));
    let multi_dec_content = fs::read_to_string(&multi_dec_file).unwrap();
    let multi_dec_fm = qdev_core::extract_frontmatter(&multi_dec_content).unwrap();
    assert_eq!(multi_dec_fm["decision_type"], "review_rejection");
    assert_eq!(multi_dec_fm["ruling"], "Failed review completely; re-triaging scope");
    assert_eq!(multi_dec_fm["context"], "review -> ready");
    assert!(multi_dec_content.contains("Transition: review -> ready"));
}


