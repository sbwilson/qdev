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
