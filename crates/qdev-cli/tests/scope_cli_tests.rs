//! CLI integration tests for scope enforcement and cross-team overrides (Story 2.4).

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
            "sally",
            "--team",
            "ui-shell",
        ])
        .assert()
        .success();

    // Update config to declare teams
    let config_path = root.join("qdev.toml");
    let mut config_content = fs::read_to_string(&config_path).unwrap();
    config_content.push_str("core-platform = [\"alice\", \"bob\"]\n");
    fs::write(&config_path, config_content).unwrap();

    // Rebuild cache
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd
        .current_dir(root)
        .args(["sync", "--rebuild"])
        .assert()
        .success();
}

fn write_story(root: &Path, id: &str, title: &str, status: &str, owners: &[&str]) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let owners_yaml = if owners.is_empty() {
        String::new()
    } else {
        format!(
            "owners:\n{}",
            owners
                .iter()
                .map(|o| format!("  - \"{}\"", o))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

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
  id: sally
updated_by:
  type: human
  id: sally
{owners_yaml}
---

## Acceptance Criteria
- AC1.
"#
        ),
    )
    .unwrap();
}

fn write_epic(root: &Path, id: &str, title: &str, owners: &[&str]) {
    let epics_dir = root.join("docs/specs/epics");
    fs::create_dir_all(&epics_dir).unwrap();
    let owners_yaml = if owners.is_empty() {
        String::new()
    } else {
        format!(
            "owners:\n{}",
            owners
                .iter()
                .map(|o| format!("  - \"{}\"", o))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    fs::write(
        epics_dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: in-progress
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
{owners_yaml}
---

# Epic
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_cli_update_in_lease_owned_story_succeeds_without_override() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);

    // Claim lease on E12S4
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Update E12S4 while holding lease
    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    update_cmd
        .current_dir(root)
        .args(["update", "E12S4", "--title", "Updated Title"])
        .assert()
        .success()
        .code(0);

    // Verify file updated
    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(
        content.contains("title: Updated Title")
            || content.contains("title: \"Updated Title\"")
    );

    // Verify no DEC files created
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(dec_files.is_empty());
}

#[test]
fn test_cli_update_out_of_lease_and_cross_team_non_interactive_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    // Claim lease on E12S4
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Attempting update on E12 non-interactively must exit 3 (policy_refusal)
    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = update_cmd
        .current_dir(root)
        .args(["update", "E12", "--title", "New Title", "--non-interactive"])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(stderr.contains("--override"));
    assert!(stderr.contains("--justification"));
}

#[test]
fn test_cli_update_non_interactive_json_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = update_cmd
        .current_dir(root)
        .args([
            "update",
            "E12",
            "--title",
            "New Title",
            "--non-interactive",
            "--json",
        ])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let parsed: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(parsed["error"]["code"], "needs_confirmation");
    let msg = parsed["error"]["message"].as_str().unwrap();
    assert!(msg.contains("--override"));
    assert!(msg.contains("--justification"));
}

#[test]
fn test_cli_update_with_override_and_justification_creates_cross_team_dec() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    update_cmd
        .current_dir(root)
        .args([
            "update",
            "E12",
            "--title",
            "Updated Epic Title",
            "--override",
            "--justification",
            "Architect signed off on cross-team dependency",
            "--non-interactive",
        ])
        .assert()
        .success()
        .code(0);

    // Verify epic updated
    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(
        content.contains("title: Updated Epic Title")
            || content.contains("title: \"Updated Epic Title\"")
    );

    // Verify DEC record created
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
    let dec_content = fs::read_to_string(dec_files[0].path()).unwrap();
    assert!(dec_content.contains("decision_type: cross_team_override"));
    assert!(dec_content.contains("subject_id: E12"));
    assert!(dec_content.contains("ruling: Architect signed off on cross-team dependency"));
}

#[test]
fn test_cli_update_with_override_empty_justification_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = update_cmd
        .current_dir(root)
        .args([
            "update",
            "E12",
            "--title",
            "New Title",
            "--override",
            "--justification",
            "   ",
            "--non-interactive",
        ])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    assert!(stderr.contains("needs_justification"));
}

#[test]
fn test_cli_update_out_of_lease_same_team_creates_lease_override_dec() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:ui-shell"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    update_cmd
        .current_dir(root)
        .args([
            "update",
            "E12",
            "--title",
            "Same Team Updated Epic",
            "--override",
            "--justification",
            "Lead signoff for epic change during story work",
            "--non-interactive",
        ])
        .assert()
        .success()
        .code(0);

    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
    let dec_content = fs::read_to_string(dec_files[0].path()).unwrap();
    assert!(dec_content.contains("decision_type: lease_override"));
    assert!(dec_content.contains("subject_id: E12"));
}

#[test]
fn test_cli_transition_scope_enforcement() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_story(root, "E12S5", "Story 5", "ready", &["team:ui-shell"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Transitioning sibling story E12S5 refuses without override
    let mut trans_cmd = Command::cargo_bin("qdev").unwrap();
    trans_cmd
        .current_dir(root)
        .args(["transition", "story", "E12S5", "in-progress", "--non-interactive"])
        .assert()
        .failure()
        .code(3);

    // With override and justification, succeeds and logs DEC
    let mut trans_ok = Command::cargo_bin("qdev").unwrap();
    trans_ok
        .current_dir(root)
        .args([
            "transition",
            "story",
            "E12S5",
            "in-progress",
            "--override",
            "--justification",
            "Emergency transition approved by PM",
            "--non-interactive",
        ])
        .assert()
        .success()
        .code(0);

    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
    let dec_content = fs::read_to_string(dec_files[0].path()).unwrap();
    assert!(dec_content.contains("decision_type: lease_override"));
    assert!(dec_content.contains("subject_id: E12S5"));
}

#[test]
fn test_cli_relate_scope_enforcement() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);
    write_epic(root, "E13", "Epic 13", &["team:core-platform"]);

    // Sync cache so relate knows about entities
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync", "--rebuild"]).assert().success();

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Relate on out-of-lease & cross-team entity E12 refuses
    let mut relate_cmd = Command::cargo_bin("qdev").unwrap();
    relate_cmd
        .current_dir(root)
        .args(["relate", "E12", "supersedes", "E13", "--non-interactive"])
        .assert()
        .failure()
        .code(3);

    // Relate with override succeeds
    let mut relate_ok = Command::cargo_bin("qdev").unwrap();
    relate_ok
        .current_dir(root)
        .args([
            "relate",
            "E12",
            "supersedes",
            "E13",
            "--override",
            "--justification",
            "Cross-epic link approved",
            "--non-interactive",
        ])
        .assert()
        .success()
        .code(0);

    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
}

#[test]
fn test_cli_cross_team_interactive_option_1_override() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .args(["update", "E12", "--title", "Interactive Updated Epic"])
        .write_stdin("1\nArchitect approval\n")
        .assert()
        .success()
        .code(0);

    // Verify epic updated
    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(
        content.contains("title: Interactive Updated Epic")
            || content.contains("title: \"Interactive Updated Epic\"")
    );

    // Verify DEC record
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
    let dec_content = fs::read_to_string(dec_files[0].path()).unwrap();
    assert!(dec_content.contains("decision_type: cross_team_override"));
    assert!(dec_content.contains("ruling: Architect approval"));
}

#[test]
fn test_cli_cross_team_interactive_option_2_add_team_to_owners() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .args(["update", "E12", "--title", "Co-owned Epic Title"])
        .write_stdin("2\nTransferring component\n")
        .assert()
        .success()
        .code(0);

    // Verify epic updated and owner appended
    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(
        content.contains("title: Co-owned Epic Title")
            || content.contains("title: \"Co-owned Epic Title\"")
    );
    assert!(content.contains("team:ui-shell"));

    // Verify DEC record created
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
    let dec_content = fs::read_to_string(dec_files[0].path()).unwrap();
    assert!(dec_content.contains("decision_type: cross_team_override"));
    assert!(dec_content.contains("ruling: Transferring component"));
}

#[test]
fn test_cli_cross_team_interactive_option_3_abort() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .args(["update", "E12", "--title", "Should Not Change"])
        .write_stdin("3\n")
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert!(stdout.contains("Aborted."));

    // Verify epic NOT updated
    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(!content.contains("Should Not Change"));

    // Verify NO DEC record created
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(dec_files.is_empty());
}

#[test]
fn test_cli_unrelate_scope_enforcement() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:core-platform"]);
    write_epic(root, "E13", "Epic 13", &["team:core-platform"]);

    // First, relate E12 supersedes E13 with override
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync", "--rebuild"]).assert().success();

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut relate_cmd = Command::cargo_bin("qdev").unwrap();
    relate_cmd
        .current_dir(root)
        .args([
            "relate",
            "E12",
            "supersedes",
            "E13",
            "--override",
            "--justification",
            "Setup relation",
            "--non-interactive",
        ])
        .assert()
        .success();

    // Now, attempting unrelate on out-of-lease & cross-team entity E12 without override must refuse
    let mut unrelate_cmd = Command::cargo_bin("qdev").unwrap();
    unrelate_cmd
        .current_dir(root)
        .args(["unrelate", "E12", "supersedes", "E13", "--non-interactive"])
        .assert()
        .failure()
        .code(3);

    // Unrelate with override succeeds
    let mut unrelate_ok = Command::cargo_bin("qdev").unwrap();
    unrelate_ok
        .current_dir(root)
        .args([
            "unrelate",
            "E12",
            "supersedes",
            "E13",
            "--override",
            "--justification",
            "Unrelate approved",
            "--non-interactive",
        ])
        .assert()
        .success()
        .code(0);

    // Verify 2 DEC files created (one for relate, one for unrelate)
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 2);

    // Verify E12 relations no longer contain supersedes
    let e12_content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(!e12_content.contains("supersedes"));
}

#[test]
fn test_cli_exempt_decision_entity() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);

    // Create a DEC file
    let dec_dir = root.join("docs/state/decisions");
    fs::create_dir_all(&dec_dir).unwrap();
    fs::write(
        dec_dir.join("DEC-01a2.md"),
        r#"---
id: DEC-01a2
title: "Exempt Decision"
status: active
version: 1
created_by:
  type: human
  id: sally
updated_by:
  type: human
  id: sally
decision_type: human_ruling
ruling: Original ruling
context: Original context
---

# Decision
"#,
    )
    .unwrap();

    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync", "--rebuild"]).assert().success();

    // Claim lease on E12S4
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    // Updating DEC-01a2 is exempt from lease override and must succeed without --override
    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    update_cmd
        .current_dir(root)
        .args([
            "update",
            "DEC-01a2",
            "--title",
            "Updated Decision Title",
            "--non-interactive",
        ])
        .assert()
        .success()
        .code(0);

    let content = fs::read_to_string(dec_dir.join("DEC-01a2.md")).unwrap();
    assert!(
        content.contains("title: Updated Decision Title")
            || content.contains("title: \"Updated Decision Title\"")
    );
}

#[test]
fn test_cli_out_of_lease_same_team_interactive_option_1_override() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:ui-shell"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .args(["update", "E12", "--title", "Interactive Same Team Epic"])
        .write_stdin("1\nTeam approved epic change\n")
        .assert()
        .success()
        .code(0);

    // Verify epic updated
    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(
        content.contains("title: Interactive Same Team Epic")
            || content.contains("title: \"Interactive Same Team Epic\"")
    );

    // Verify DEC record logged with lease_override
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(dec_files.len(), 1);
    let dec_content = fs::read_to_string(dec_files[0].path()).unwrap();
    assert!(dec_content.contains("decision_type: lease_override"));
    assert!(dec_content.contains("ruling: Team approved epic change"));
}

#[test]
fn test_cli_out_of_lease_same_team_interactive_option_3_abort() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic(root, "E12", "Epic 12", &["team:ui-shell"]);

    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "story", "E12S4"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .args(["update", "E12", "--title", "Should Not Change Same Team"])
        .write_stdin("3\n")
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert!(stdout.contains("Aborted."));

    // Verify epic NOT updated
    let content = fs::read_to_string(root.join("docs/specs/epics/E12.md")).unwrap();
    assert!(!content.contains("Should Not Change Same Team"));

    // Verify NO DEC record created
    let dec_files: Vec<_> = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(dec_files.is_empty());
}
