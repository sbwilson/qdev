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

fn create_story(root: &Path, id: &str, status: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{id}.md")),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: {status}
version: 1
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Verify scratchpad behavior.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_scratch_append_with_held_lease_happy_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    // 1. Claim story lease
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // 2. Append tradeoff entry
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "tradeoff",
            "--",
            "AtomicBool over Mutex on frame drop latch",
        ])
        .assert()
        .success();

    // Verify JSONL file content
    let scratch_file = root.join("docs/state/scratch/E12S4.jsonl");
    assert!(scratch_file.is_file());
    let content = fs::read_to_string(&scratch_file).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);

    let val: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(val["seq"], 1);
    assert_eq!(val["kind"], "tradeoff");
    assert_eq!(val["text"], "AtomicBool over Mutex on frame drop latch");
    assert_eq!(val["author"]["id"], "simon");
    assert!(val["at"].as_str().is_some());
}

#[test]
fn test_scratch_append_second_entry_default_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    // Claim lease
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // First entry
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "scratch", "append", "E12S4", "--kind", "tradeoff", "--", "First",
        ])
        .assert()
        .success();

    // Second entry with default kind
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["scratch", "append", "E12S4", "--", "Checked frame latch"])
        .assert()
        .success();

    let scratch_file = root.join("docs/state/scratch/E12S4.jsonl");
    let content = fs::read_to_string(&scratch_file).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 2);

    let val2: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(val2["seq"], 2);
    assert_eq!(val2["kind"], "note");
    assert_eq!(val2["text"], "Checked frame latch");
}

#[test]
fn test_scratch_append_without_lease_non_interactive() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    // No lease held, non-interactive append should fail with exit code 3
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--non-interactive",
            "--json",
            "scratch",
            "append",
            "E12S4",
            "--",
            "Unleased note",
        ])
        .assert()
        .failure()
        .code(3);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "needs_confirmation");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--override --justification"));
}

#[test]
fn test_scratch_append_with_wrong_lease_non_interactive() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S3", "in-progress");
    create_story(root, "E12S4", "in-progress");

    // Claim lease on E12S3
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S3"])
        .assert()
        .success();

    // Try to append to E12S4
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--non-interactive",
            "--json",
            "scratch",
            "append",
            "E12S4",
            "--",
            "Wrong lease note",
        ])
        .assert()
        .failure()
        .code(3);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "needs_confirmation");
}

#[test]
fn test_scratch_append_with_override_and_justification() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    // No lease held, but --override and --justification provided
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "--override",
            "--justification",
            "Lead signoff for emergency note",
            "scratch",
            "append",
            "E12S4",
            "--",
            "Emergency note",
        ])
        .assert()
        .success();

    let scratch_file = root.join("docs/state/scratch/E12S4.jsonl");
    assert!(scratch_file.is_file());

    // Verify a DEC- record was logged in docs/state/decisions/
    let dec_dir = root.join("docs/state/decisions");
    let mut dec_files = Vec::new();
    if dec_dir.is_dir() {
        for entry in fs::read_dir(&dec_dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                dec_files.push(path);
            }
        }
    }
    assert_eq!(dec_files.len(), 1, "Expected one override decision record");
    let dec_content = fs::read_to_string(&dec_files[0]).unwrap();
    assert!(dec_content.contains("type: lease_override"));
    assert!(dec_content.contains("Lead signoff for emergency note"));
}

#[test]
fn test_scratch_append_with_override_missing_justification() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--override",
            "--json",
            "scratch",
            "append",
            "E12S4",
            "--",
            "Note",
        ])
        .assert()
        .failure()
        .code(3);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "needs_justification");
}

#[test]
fn test_scratch_append_invalid_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--json",
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "invalid_kind",
            "--",
            "Text",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid scratchpad kind"));
}

#[test]
fn test_scratch_append_empty_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--json", "scratch", "append", "E12S4", "--kind", "note", "--", "   ",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_scratch_append_nonexistent_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "--override",
            "--justification",
            "Override",
            "scratch",
            "append",
            "E99S99",
            "--",
            "Note",
        ])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_scratch_read_all_entries_and_empty() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    // 1. Read empty scratchpad
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["scratch", "read", "E12S4"])
        .assert()
        .success();
    let out = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(out.contains("(empty)"));

    // Read empty scratchpad with JSON
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["--json", "scratch", "read", "E12S4"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["story_id"], "E12S4");
    assert_eq!(val["entries"].as_array().unwrap().len(), 0);

    // Claim and append 3 entries
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "note",
            "--",
            "First note",
        ])
        .assert()
        .success();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "decision",
            "--",
            "Design decision",
        ])
        .assert()
        .success();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "tradeoff",
            "--",
            "Tradeoff text",
        ])
        .assert()
        .success();

    // Read all entries
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["--json", "scratch", "read", "E12S4"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let entries = val["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["seq"], 1);
    assert_eq!(entries[0]["kind"], "note");
    assert_eq!(entries[1]["seq"], 2);
    assert_eq!(entries[1]["kind"], "decision");
    assert_eq!(entries[2]["seq"], 3);
    assert_eq!(entries[2]["kind"], "tradeoff");
}

#[test]
fn test_scratch_read_summary_and_budget() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Append 8 entries:
    // 1: note
    // 2: decision
    // 3: note
    // 4: note
    // 5: note
    // 6: transition
    // 7: note
    // 8: note
    for (k, t) in [
        ("note", "Note 1"),
        ("decision", "Crucial Decision 2"),
        ("note", "Note 3"),
        ("note", "Note 4"),
        ("note", "Note 5"),
        ("transition", "Transition 6"),
        ("note", "Note 7"),
        ("note", "Note 8"),
    ] {
        Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args(["scratch", "append", "E12S4", "--kind", k, "--", t])
            .assert()
            .success();
    }

    // Read summary (default last 5 entries [4, 5, 6, 7, 8] + decisions [2] + transitions [6])
    // Result should contain seqs: 2, 4, 5, 6, 7, 8
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["--json", "scratch", "read", "E12S4", "--summary"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let entries = val["entries"].as_array().unwrap();
    let seqs: Vec<u64> = entries.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, vec![2, 4, 5, 6, 7, 8]);

    // Read summary with budget limit
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--json",
            "scratch",
            "read",
            "E12S4",
            "--summary",
            "--budget",
            "12",
        ])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let budget_entries = val["entries"].as_array().unwrap();
    assert!(budget_entries.len() < seqs.len());
    // Key decision/transition are prioritized
    let budget_seqs: Vec<u64> = budget_entries
        .iter()
        .map(|e| e["seq"].as_u64().unwrap())
        .collect();
    assert!(budget_seqs.contains(&2) || budget_seqs.contains(&6));
}

#[test]
fn test_story_spec_file_is_never_modified() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    let story_path = root.join("docs/specs/stories/E12S4.md");
    let original_content = fs::read_to_string(&story_path).unwrap();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "tradeoff",
            "--",
            "Tradeoff text",
        ])
        .assert()
        .success();

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["scratch", "read", "E12S4", "--summary"])
        .assert()
        .success();

    let post_content = fs::read_to_string(&story_path).unwrap();
    assert_eq!(
        original_content, post_content,
        "Story spec file must never be modified by scratch operations"
    );
}

#[test]
fn test_scratch_append_success_json_envelope() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "--json",
            "scratch",
            "append",
            "E12S4",
            "--kind",
            "decision",
            "--",
            "Envelope test note",
        ])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["story_id"], "E12S4");
    assert_eq!(val["seq"], 1);
    assert_eq!(val["kind"], "decision");
    assert_eq!(val["text"], "Envelope test note");
    assert_eq!(val["author"]["type"], "human");
    assert_eq!(val["author"]["id"], "simon");
    assert!(val["at"].as_str().is_some());
}

#[test]
fn test_scratch_append_with_trailing_flags_after_separator() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    // Without lease, pass --override and --justification AFTER `-- <text>`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--",
            "Emergency note",
            "--override",
            "--justification",
            "Lead signoff",
        ])
        .assert()
        .success();

    // Verify entry was appended
    let scratch_file = root.join("docs/state/scratch/E12S4.jsonl");
    assert!(scratch_file.is_file());
    let content = fs::read_to_string(&scratch_file).unwrap();
    let lines: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let val: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(val["text"], "Emergency note");

    // Also verify note text containing literal flag-like string e.g. "--override" is not eaten
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args([
            "scratch",
            "append",
            "E12S4",
            "--",
            "--override in note text",
            "--override",
            "--justification",
            "Lead signoff 2",
        ])
        .assert()
        .success();

    let content2 = fs::read_to_string(&scratch_file).unwrap();
    let lines2: Vec<&str> = content2.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines2.len(), 2);
    let val2: Value = serde_json::from_str(lines2[1]).unwrap();
    assert_eq!(val2["text"], "--override in note text");
}

#[test]
fn test_scratch_read_last_without_summary_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    create_story(root, "E12S4", "in-progress");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["scratch", "read", "E12S4", "--last", "3"])
        .assert()
        .failure()
        .code(2);
}
