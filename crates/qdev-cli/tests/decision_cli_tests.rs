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

fn write_story(root: &Path, id: &str, title: &str) {
    let path = root.join(format!("docs/specs/stories/{}.md", id));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content = format!(
        r#"---
id: {id}
title: "{title}"
status: ready
version: 1
appetite: small
target_modules: ["core"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- [x] AC-1: Ready.
"#
    );
    fs::write(path, content).unwrap();

    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync"]).assert().success();
}

fn write_adr(root: &Path, id: &str, title: &str) {
    let path = root.join(format!("docs/specs/adrs/{}.md", id));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let content = format!(
        r#"---
id: {id}
title: "{title}"
status: active
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# {title}
"#
    );
    fs::write(path, content).unwrap();

    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync"]).assert().success();
}

#[test]
fn test_happy_path_manual_decision_log() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Buffer sizing story");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "human_ruling",
            "--topic",
            "Buffer sizing",
            "--ruling",
            "Fixed 4 MB pool",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("Logged decision DEC-"));
    assert!(stdout.contains("E12S4"));
    assert!(stdout.contains("human_ruling"));
    assert!(stdout.contains("Buffer sizing"));

    // Verify DEC file on disk
    let decs_dir = root.join("docs/state/decisions");
    let entries: Vec<_> = fs::read_dir(decs_dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
    let dec_path = entries[0].as_ref().unwrap().path();
    let file_content = fs::read_to_string(&dec_path).unwrap();

    assert!(file_content.contains("decision_type: human_ruling"));
    assert!(file_content.contains("subject_id: E12S4"));
    assert!(file_content.contains("topic: Buffer sizing"));
    assert!(file_content.contains("ruling: Fixed 4 MB pool"));
    assert!(file_content.contains("context: Decision on E12S4"));
    assert!(
        file_content.contains("# Buffer sizing\n\nContext: Decision on E12S4\n\nFixed 4 MB pool\n")
    );
}

#[test]
fn test_decision_log_with_context_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_adr(root, "AD-43", "Serialization Architecture");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "AD-43",
            "--type",
            "agent_assumption",
            "--topic",
            "Serialization",
            "--ruling",
            "Use rmp-serde",
            "--context",
            "Benchmarked 30% faster",
            "--json",
        ])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert!(val["id"].as_str().unwrap().starts_with("DEC-"));
    assert_eq!(val["subject_id"], "AD-43");
    assert_eq!(val["decision_type"], "agent_assumption");
    assert_eq!(val["topic"], "Serialization");
    assert_eq!(val["ruling"], "Use rmp-serde");
    assert_eq!(val["context"], "Benchmarked 30% faster");
    assert_eq!(val["author"]["id"], "simon");
    assert!(val["path"]
        .as_str()
        .unwrap()
        .starts_with("docs/state/decisions/DEC-"));
}

#[test]
fn test_list_decisions_subject_and_type_filtering() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4");
    write_story(root, "E12S5", "Story 5");

    // Decision 1: E12S4 human_ruling
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "human_ruling",
            "--topic",
            "Pool size",
            "--ruling",
            "4 MB",
        ])
        .assert()
        .success();

    // Decision 2: E12S4 cross_team_override
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "cross_team_override",
            "--topic",
            "API override",
            "--ruling",
            "Approved cross-team",
        ])
        .assert()
        .success();

    // Decision 3: E12S5 human_ruling
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S5",
            "--type",
            "human_ruling",
            "--topic",
            "Thread count",
            "--ruling",
            "8 threads",
        ])
        .assert()
        .success();

    // 1. Filter by subject E12S4 -> returns 2 items
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "decisions", "--subject", "E12S4", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(val["kind"], "decision");

    // 2. Filter by subject E12S4 AND type cross_team_override -> returns 1 item
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "list",
            "decisions",
            "--subject",
            "E12S4",
            "--type",
            "cross_team_override",
            "--json",
        ])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "API override");

    // 3. Filter with no matches -> returns empty list
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "list",
            "decisions",
            "--subject",
            "E12S4",
            "--type",
            "pivot",
            "--json",
        ])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    assert_eq!(items.len(), 0);

    // Text output returns "(no matching entities)"
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "decisions", "--subject", "E12S4", "--type", "pivot"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("(no matching entities)"));

    // 4. Filter by type human_ruling alone (no --subject) -> returns 2 items across E12S4 and E12S5
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "decisions", "--type", "human_ruling", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(val["kind"], "decision");
}

#[test]
fn test_decision_log_error_cases() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4");

    // 1. Missing required flags (--type, --topic, --ruling) -> Clap exit 2
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["decision", "log", "--subject", "E12S4"])
        .assert()
        .failure()
        .code(2);

    // 2. Non-existent subject entity -> Exit 2 usage_error / entity_not_found
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "NONEXISTENT",
            "--type",
            "human_ruling",
            "--topic",
            "T",
            "--ruling",
            "R",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");

    // 3. Invalid decision type -> Exit 2 usage_error
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "invalid_type",
            "--topic",
            "T",
            "--ruling",
            "R",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");

    // 4. Empty ruling -> Exit 2 usage_error
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "human_ruling",
            "--topic",
            "T",
            "--ruling",
            "   ",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");

    // 5. Empty or whitespace topic -> Exit 2 usage_error
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "human_ruling",
            "--topic",
            "   ",
            "--ruling",
            "Some ruling",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_system_generated_decisions_parity_with_list_filtering() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4");

    // Claim a lease on E12S4
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Release with --force --justification records a lease_override decision
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "release",
            "story",
            "E12S4",
            "--force",
            "--justification",
            "Force released for priority task",
        ])
        .assert()
        .success();

    // Verify it is queryable via `qdev list decisions --subject E12S4 --type lease_override`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "list",
            "decisions",
            "--subject",
            "E12S4",
            "--type",
            "lease_override",
            "--json",
        ])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Lease override on story E12S4");
}

#[test]
fn test_decision_log_with_author_override() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "decision",
            "log",
            "--subject",
            "E12S4",
            "--type",
            "agent_assumption",
            "--topic",
            "Cache buffer size",
            "--ruling",
            "Auto-sized to 16MB",
            "--author-type",
            "agent",
            "--author-id",
            "bot-42",
            "--json",
        ])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["author"]["type"], "agent");
    assert_eq!(val["author"]["id"], "bot-42");

    let file_path = root.join(val["path"].as_str().unwrap());
    let content = fs::read_to_string(&file_path).unwrap();
    let fm = qdev_core::extract_frontmatter(&content).unwrap();
    assert_eq!(fm["created_by"]["type"], "agent");
    assert_eq!(fm["created_by"]["id"], "bot-42");
    assert_eq!(fm["updated_by"]["type"], "agent");
    assert_eq!(fm["updated_by"]["id"], "bot-42");
}
