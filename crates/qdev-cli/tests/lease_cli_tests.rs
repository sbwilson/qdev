//! CLI integration tests for story leases (Story 2.3).

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

fn write_story(root: &Path, id: &str, title: &str, status: &str) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: {status}
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC1.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_cli_claim_unleased_story_text_mode() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let text = std::str::from_utf8(&output.stdout).unwrap();
    assert!(text.contains("Claimed lease on story E12S4"), "text: {}", text);
    assert!(text.contains("export QDEV_SESSION=qs_E12S4_"), "text: {}", text);

    // Verify local lease record exists
    let local_lease = root.join(".qdev/leases/E12S4.json");
    assert!(local_lease.is_file());
    let lease_json: Value = serde_json::from_str(&fs::read_to_string(&local_lease).unwrap()).unwrap();
    assert_eq!(lease_json["story_id"], "E12S4");
    assert_eq!(lease_json["holder"], "simon");
    assert!(lease_json["session_token"].as_str().unwrap().starts_with("qs_E12S4_"));
}

#[test]
fn test_cli_claim_unleased_story_json_mode() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["story_id"], "E12S4");
    assert_eq!(val["holder"], "simon");
    assert_eq!(val["author_type"], "human");
    assert!(val["session_token"].as_str().unwrap().starts_with("qs_E12S4_"));
}

#[test]
fn test_cli_claim_nonexistent_story_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["claim", "story", "E99S99", "--json"])
        .assert()
        .failure()
        .code(1);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "entity_not_found");
}

#[test]
fn test_cli_claim_already_leased_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    // Claim once
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Duplicate claim in text mode
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["claim", "story", "E12S4", "--author-id", "bob"])
        .assert()
        .failure()
        .code(5);

    let output = assert2.get_output();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(stderr.contains("already_leased"), "stderr: {}", stderr);
    assert!(stderr.contains("simon"), "stderr: {}", stderr);

    // Duplicate claim in JSON mode
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    let assert3 = cmd3
        .current_dir(root)
        .args(["claim", "story", "E12S4", "--json"])
        .assert()
        .failure()
        .code(5);

    let val: Value = serde_json::from_slice(&assert3.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "already_leased");
    let details = &val["error"]["details"];
    assert_eq!(details["story_id"], "E12S4");
    assert_eq!(details["holder"], "simon");
}

#[test]
fn test_cli_release_own_lease() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    // Claim
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Release with story ID in text mode
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["release", "story", "E12S4"])
        .assert()
        .success()
        .code(0);

    let text = std::str::from_utf8(&assert2.get_output().stdout).unwrap();
    assert!(text.contains("Released lease on story E12S4"), "text: {}", text);
    assert!(!root.join(".qdev/leases/E12S4.json").exists());
}

#[test]
fn test_cli_release_own_lease_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    // Claim
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Release in JSON mode
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["release", "E12S4", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert2.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["story_id"], "E12S4");
    assert_eq!(val["released"], true);
}

#[test]
fn test_cli_release_without_story_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    // Without active lease: exit 1 no_active_lease
    let mut cmd0 = Command::cargo_bin("qdev").unwrap();
    cmd0.current_dir(root)
        .args(["release", "--json"])
        .assert()
        .failure()
        .code(1);

    // Claim 1 story
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Bare `qdev release` detects and releases active lease
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["release"])
        .assert()
        .success()
        .code(0);

    assert!(!root.join(".qdev/leases/E12S4.json").exists());
}

#[test]
fn test_cli_release_other_holder_requires_force() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    // Claim as simon
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S4", "--author-id", "simon"])
        .assert()
        .success();

    // Non-holder attempts release without force: exit 3
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["release", "E12S4", "--author-id", "alice"])
        .assert()
        .failure()
        .code(3);

    let stderr = std::str::from_utf8(&assert2.get_output().stderr).unwrap();
    assert!(stderr.contains("policy_refusal"), "stderr: {}", stderr);
    assert!(stderr.contains("--force --justification"), "stderr: {}", stderr);

    // Force without justification: exit 3 needs_justification
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    cmd3.current_dir(root)
        .args(["release", "E12S4", "--force", "--author-id", "alice", "--json"])
        .assert()
        .failure()
        .code(3);

    // Force with justification: exit 0, creates DEC- record
    let mut cmd4 = Command::cargo_bin("qdev").unwrap();
    let assert4 = cmd4
        .current_dir(root)
        .args([
            "release",
            "E12S4",
            "--force",
            "--justification",
            "Agent crashed and left stale lease",
            "--author-id",
            "alice",
            "--json",
        ])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert4.get_output().stdout).unwrap();
    assert_eq!(val["released"], true);
    let dec_id = val["decision_id"].as_str().unwrap();
    assert!(dec_id.starts_with("DEC-"));

    // Verify decision file exists in docs/state/decisions
    let dec_file = root.join("docs/state/decisions").join(format!("{}.md", dec_id));
    assert!(dec_file.is_file());
    let dec_content = fs::read_to_string(&dec_file).unwrap();
    assert!(dec_content.contains("decision_type: lease_override"));
    assert!(dec_content.contains("Agent crashed and left stale lease"));
}

#[test]
fn test_cli_transition_done_auto_releases_lease() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "review");

    // Claim lease
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();
    assert!(root.join(".qdev/leases/E12S4.json").is_file());

    // Transition to done
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["transition", "story", "E12S4", "done"])
        .assert()
        .success()
        .code(0);

    // Lease should be gone
    assert!(!root.join(".qdev/leases/E12S4.json").exists());
}

#[test]
fn test_cli_doctor_reports_stale_leases() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S1", "Story 1", "in-progress");
    write_story(root, "E12S2", "Story 2", "in-progress");

    // Claim E12S1 fresh
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["claim", "story", "E12S1"])
        .assert()
        .success();

    // Create stale lease for E12S2
    let lease_dir = root.join(".qdev/leases");
    fs::create_dir_all(&lease_dir).unwrap();
    let stale_lease = qdev_core::lease::StoryLease {
        story_id: "E12S2".to_string(),
        holder: "bob".to_string(),
        author_type: "human".to_string(),
        worktree_path: root.display().to_string(),
        branch: "main".to_string(),
        started_at: "2026-09-01T00:00:00Z".to_string(),
        session_token: "qs_E12S2_0001".to_string(),
    };
    fs::write(
        lease_dir.join("E12S2.json"),
        serde_json::to_string_pretty(&stale_lease).unwrap(),
    )
    .unwrap();

    // Run doctor in text mode
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2.current_dir(root).args(["doctor"]).assert().success();
    let text = std::str::from_utf8(&assert2.get_output().stdout).unwrap();
    assert!(text.contains("[leases]"), "text: {}", text);
    assert!(text.contains("active_count = 2"), "text: {}", text);
    assert!(text.contains("stale_count = 1"), "text: {}", text);

    // Run doctor in JSON mode
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    let assert3 = cmd3
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert3.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let leases_sec = sections.iter().find(|s| s["name"] == "leases").unwrap();

    assert_eq!(leases_sec["status"], "ok");
    assert_eq!(leases_sec["active_count"], 2);
    assert_eq!(leases_sec["stale_count"], 1);
    assert_eq!(leases_sec["stale_age_days"], 3);

    let stale_leases = leases_sec["stale_leases"].as_array().unwrap();
    assert_eq!(stale_leases.len(), 1);
    assert_eq!(stale_leases[0]["story_id"], "E12S2");
}

#[test]
fn test_cli_doctor_custom_stale_age_days() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Set custom stale_age_days in qdev.toml
    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push_str("\n[leases]\nstale_age_days = 7\n");
    fs::write(toml_path, toml).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let leases_sec = sections.iter().find(|s| s["name"] == "leases").unwrap();
    assert_eq!(leases_sec["stale_age_days"], 7);
}

#[test]
fn test_cli_forced_release_creates_queryable_decision() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S1", "Story 1", "in-progress");

    // Alice claims story
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S1", "--author-id", "alice"])
        .assert()
        .success();

    // Bob breaks lease with force and justification
    let mut rel_cmd = Command::cargo_bin("qdev").unwrap();
    let rel_assert = rel_cmd
        .current_dir(root)
        .args([
            "release",
            "story",
            "E12S1",
            "--force",
            "--justification",
            "Reassigning to bob for urgent fix",
            "--author-id",
            "bob",
            "--json",
        ])
        .assert()
        .success();

    let rel_val: Value = serde_json::from_slice(&rel_assert.get_output().stdout).unwrap();
    let dec_id = rel_val["decision_id"].as_str().expect("must have decision_id");
    assert!(dec_id.starts_with("DEC-"), "dec_id: {}", dec_id);

    // Query the decision entity via qdev get <dec_id> --json
    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    let get_assert = get_cmd
        .current_dir(root)
        .args(["get", dec_id, "--json"])
        .assert()
        .success();

    let get_val: Value = serde_json::from_slice(&get_assert.get_output().stdout).unwrap();
    assert_eq!(get_val["id"], dec_id);
    assert_eq!(get_val["kind"], "decision");
    assert_eq!(get_val["status"], "active");
}

#[test]
fn test_cli_claim_and_release_reject_non_story_entities() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create an Epic entity
    let epics_dir = root.join("docs/specs/epics");
    fs::create_dir_all(&epics_dir).unwrap();
    fs::write(
        epics_dir.join("E12.md"),
        r#"---
id: E12
title: "Epic Twelve"
status: active
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# Epic Twelve
"#,
    )
    .unwrap();

    // Hydrate cache
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync"]).assert().success();

    // 1. qdev claim epic E12 -> usage error (exit 2)
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    let assert1 = cmd1
        .current_dir(root)
        .args(["claim", "epic", "E12"])
        .assert()
        .failure()
        .code(2);
    let err1 = std::str::from_utf8(&assert1.get_output().stderr).unwrap();
    assert!(err1.contains("Only story entities can be leased"), "err1: {}", err1);

    // 2. qdev claim E12 (bare entity ID) -> usage error (exit 2)
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["claim", "E12"])
        .assert()
        .failure()
        .code(2);
    let err2 = std::str::from_utf8(&assert2.get_output().stderr).unwrap();
    assert!(err2.contains("Only story entities can be leased"), "err2: {}", err2);

    // 3. qdev release epic E12 -> usage error (exit 2)
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    let assert3 = cmd3
        .current_dir(root)
        .args(["release", "epic", "E12"])
        .assert()
        .failure()
        .code(2);
    let err3 = std::str::from_utf8(&assert3.get_output().stderr).unwrap();
    assert!(err3.contains("Only story entities can be leased"), "err3: {}", err3);

    // 4. qdev release E12 (bare entity ID) -> usage error (exit 2)
    let mut cmd4 = Command::cargo_bin("qdev").unwrap();
    let assert4 = cmd4
        .current_dir(root)
        .args(["release", "E12"])
        .assert()
        .failure()
        .code(2);
    let err4 = std::str::from_utf8(&assert4.get_output().stderr).unwrap();
    assert!(err4.contains("Only story entities can be leased"), "err4: {}", err4);
}

