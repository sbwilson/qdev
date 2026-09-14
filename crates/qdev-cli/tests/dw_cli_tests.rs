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

    let mut qdev_toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    qdev_toml.push_str(
        r#"
[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]

[[modules]]
id = "foundation"
paths = ["crates/foundation/**"]
"#,
    );
    fs::write(root.join("qdev.toml"), qdev_toml).unwrap();
}

fn write_story(root: &Path, id: &str, title: &str, status: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: {status}
version: 1
owners:
  - simon
target_modules:
  - bridge
appetite: small
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Verified resolution.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_cli_dw_add_happy_path_text_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // 1. Text mode add with negligible risk (no rationale needed)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Reduce buffer allocations",
        ])
        .assert()
        .success()
        .code(0);

    let stdout_str = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(stdout_str.contains("DW-"));
    assert!(stdout_str.contains("bridge"));

    // 2. JSON mode add with acceptable_with_mitigation and rationale
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "foundation",
            "--risk",
            "acceptable_with_mitigation",
            "--title",
            "Async queue overflow",
            "--rationale",
            "Bounded channel with drop-oldest policy",
            "--json",
        ])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert!(val["id"].as_str().unwrap().starts_with("DW-"));
    assert_eq!(val["target_module"], "foundation");
    assert_eq!(val["safety_risk"], "acceptable_with_mitigation");
    assert_eq!(val["status"], "open");
    assert_eq!(
        val["rationale"],
        "Bounded channel with drop-oldest policy"
    );

    // Verify file created
    let dw_id = val["id"].as_str().unwrap();
    let dw_file = root.join(format!("docs/state/dw/{}.md", dw_id));
    assert!(dw_file.exists());
}

#[test]
fn test_cli_dw_add_missing_rationale_exit_1() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Text mode without rationale for acceptable_with_mitigation fails with exit 1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "acceptable_with_mitigation",
            "--title",
            "Needs rationale",
        ])
        .assert()
        .failure()
        .code(1);

    let stderr_str = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(
        stderr_str.contains("regulatory.require_rationale_for"),
        "Must cite regulatory.require_rationale_for, got: {}",
        stderr_str
    );

    // JSON mode without rationale returns regulatory.require_rationale_for
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "unacceptable",
            "--title",
            "Unacceptable risk work",
            "--json",
        ])
        .assert()
        .failure()
        .code(1);

    let val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "regulatory.require_rationale_for");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("regulatory.require_rationale_for"));
}

#[test]
fn test_cli_dw_add_unregistered_module_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "nonexistent_mod",
            "--risk",
            "negligible",
            "--title",
            "Bad module",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"].as_str().unwrap().contains("nonexistent_mod"));
}

#[test]
fn test_cli_dw_add_invalid_risk_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "critical",
            "--title",
            "Bad risk",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"].as_str().unwrap().contains("valid risk levels are"));
}

#[test]
fn test_cli_dw_add_unresolvable_story_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Bad story",
            "--story",
            "E99S99",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "entity_not_found");
    assert!(val["error"]["message"].as_str().unwrap().contains("E99S99"));
}

#[test]
fn test_cli_dw_add_out_of_lease_gate_and_override() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E12S1", "Story with active lease", "in-progress");
    write_story(root, "E12S4", "Unleased target story", "in-progress");

    // Developer claims the lease on E12S1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["claim", "story", "E12S1"])
        .assert()
        .success();

    // Trying to add DW linked to E12S4 while holding lease on E12S1 -> exits 3 (PolicyRefusal)
    let mut add_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = add_cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--story",
            "E12S4",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Gated addition",
            "--json",
        ])
        .assert()
        .failure()
        .code(3);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "needs_confirmation");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .to_lowercase()
        .contains("out-of-lease"));

    // With --override and --justification, the addition succeeds with exit 0
    let mut override_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_override = override_cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--story",
            "E12S4",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Overridden addition",
            "--override",
            "--justification",
            "Emergency blocker mitigation",
            "--json",
        ])
        .assert()
        .success()
        .code(0);

    let val_override: Value = serde_json::from_slice(&assert_override.get_output().stdout).unwrap();
    assert!(val_override["id"].as_str().unwrap().starts_with("DW-"));

    // Verify DEC record was created in docs/state/decisions or docs/specs/decisions
    let mut dec_found = false;
    for sub in &["docs/state/decisions", "docs/specs/decisions"] {
        let dec_dir = root.join(sub);
        if dec_dir.exists() {
            for entry in fs::read_dir(dec_dir).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    let content = fs::read_to_string(&path).unwrap();
                    if content.contains("Emergency blocker mitigation") {
                        dec_found = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(dec_found, "DEC override record must be created");
}

#[test]
fn test_cli_dw_close() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add a DW
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Work to finish",
            "--json",
        ])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let dw_id1 = val["id"].as_str().unwrap().to_string();

    // Close as default (done) with resolution
    let mut close_cmd = Command::cargo_bin("qdev").unwrap();
    let close_assert = close_cmd
        .current_dir(root)
        .args([
            "dw",
            "close",
            &dw_id1,
            "--resolution",
            "Addressed in refactor",
            "--json",
        ])
        .assert()
        .success()
        .code(0);

    let close_val: Value = serde_json::from_slice(&close_assert.get_output().stdout).unwrap();
    assert_eq!(close_val["status"], "done");
    assert_eq!(close_val["resolution"], "Addressed in refactor");

    // Add another DW
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Work to drop",
            "--json",
        ])
        .assert()
        .success();
    let val2: Value = serde_json::from_slice(&assert2.get_output().stdout).unwrap();
    let dw_id2 = val2["id"].as_str().unwrap().to_string();

    // Close as wont_fix without justification/resolution -> fails with exit 2
    let mut close_wont_fix_bad = Command::cargo_bin("qdev").unwrap();
    let wont_fix_bad = close_wont_fix_bad
        .current_dir(root)
        .args(["dw", "close", &dw_id2, "--status", "wont_fix", "--json"])
        .assert()
        .failure()
        .code(2);
    let wont_fix_bad_val: Value =
        serde_json::from_slice(&wont_fix_bad.get_output().stdout).unwrap();
    assert_eq!(wont_fix_bad_val["error"]["code"], "usage_error");

    // Close as wont_fix with justification -> succeeds
    let mut close_wont_fix_ok = Command::cargo_bin("qdev").unwrap();
    let wont_fix_ok = close_wont_fix_ok
        .current_dir(root)
        .args([
            "dw",
            "close",
            &dw_id2,
            "--status",
            "wont_fix",
            "--justification",
            "Out of scope for this architecture",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let wont_fix_ok_val: Value = serde_json::from_slice(&wont_fix_ok.get_output().stdout).unwrap();
    assert_eq!(wont_fix_ok_val["status"], "wont_fix");
    assert_eq!(
        wont_fix_ok_val["resolution"],
        "Out of scope for this architecture"
    );
}

#[test]
fn test_cli_dw_list_and_filters() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E1S1", "Story 1", "ready");

    // Add DW 1 (bridge, negligible, story E1S1)
    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "DW One",
            "--story",
            "E1S1",
        ])
        .assert()
        .success();

    // Add DW 2 (foundation, acceptable_with_mitigation)
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "foundation",
            "--risk",
            "acceptable_with_mitigation",
            "--title",
            "DW Two",
            "--rationale",
            "Tested limits",
        ])
        .assert()
        .success();

    // Add DW 3 (bridge, unacceptable)
    let mut cmd3 = Command::cargo_bin("qdev").unwrap();
    cmd3.current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "unacceptable",
            "--title",
            "DW Three",
            "--rationale",
            "Approved deviation",
        ])
        .assert()
        .success();

    // List all
    let mut list_all = Command::cargo_bin("qdev").unwrap();
    let assert_all = list_all
        .current_dir(root)
        .args(["dw", "list", "--json"])
        .assert()
        .success();
    let val_all: Value = serde_json::from_slice(&assert_all.get_output().stdout).unwrap();
    assert_eq!(val_all["items"].as_array().unwrap().len(), 3);

    // List filter by module: bridge
    let mut list_bridge = Command::cargo_bin("qdev").unwrap();
    let assert_bridge = list_bridge
        .current_dir(root)
        .args(["dw", "list", "--module", "bridge", "--json"])
        .assert()
        .success();
    let val_bridge: Value = serde_json::from_slice(&assert_bridge.get_output().stdout).unwrap();
    assert_eq!(val_bridge["items"].as_array().unwrap().len(), 2);

    // List filter by risk: acceptable_with_mitigation
    let mut list_risk = Command::cargo_bin("qdev").unwrap();
    let assert_risk = list_risk
        .current_dir(root)
        .args([
            "dw",
            "list",
            "--risk",
            "acceptable_with_mitigation",
            "--json",
        ])
        .assert()
        .success();
    let val_risk: Value = serde_json::from_slice(&assert_risk.get_output().stdout).unwrap();
    assert_eq!(val_risk["items"].as_array().unwrap().len(), 1);

    // List filter by story: E1S1
    let mut list_story = Command::cargo_bin("qdev").unwrap();
    let assert_story = list_story
        .current_dir(root)
        .args(["dw", "list", "--story", "E1S1", "--json"])
        .assert()
        .success();
    let val_story: Value = serde_json::from_slice(&assert_story.get_output().stdout).unwrap();
    assert_eq!(val_story["items"].as_array().unwrap().len(), 1);

    // Text mode list
    let mut list_txt = Command::cargo_bin("qdev").unwrap();
    let assert_txt = list_txt.current_dir(root).args(["dw", "list"]).assert().success();
    let txt = std::str::from_utf8(&assert_txt.get_output().stdout).unwrap();
    assert!(txt.contains("DW-"));
    assert!(txt.contains("bridge"));
    assert!(txt.contains("foundation"));
}

#[test]
fn test_cli_list_with_risk_flag() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add DW
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "acceptable_with_mitigation",
            "--title",
            "Risk filtering test",
            "--rationale",
            "Mitigated",
        ])
        .assert()
        .success();

    // Query via qdev list dw --risk acceptable_with_mitigation --json
    let mut list_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_list = list_cmd
        .current_dir(root)
        .args([
            "list",
            "dw",
            "--risk",
            "acceptable_with_mitigation",
            "--json",
        ])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert_list.get_output().stdout).unwrap();
    assert_eq!(val["items"].as_array().unwrap().len(), 1);

    // Query via qdev list dw --risk negligible --json -> returns 0 items
    let mut list_cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert_list2 = list_cmd2
        .current_dir(root)
        .args(["list", "dw", "--risk", "negligible", "--json"])
        .assert()
        .success();
    let val2: Value = serde_json::from_slice(&assert_list2.get_output().stdout).unwrap();
    assert_eq!(val2["items"].as_array().unwrap().len(), 0);
}

#[test]
fn test_cli_story_done_closes_dw() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add DW
    let mut add_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_add = add_cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "Will be closed by story",
            "--json",
        ])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert_add.get_output().stdout).unwrap();
    let dw_id = val["id"].as_str().unwrap().to_string();

    // Write story in review state declaring relations.closes_dw
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("E1S1.md"),
        format!(
            r#"---
id: E1S1
title: "Resolving story"
status: review
version: 1
owners:
  - simon
target_modules:
  - bridge
appetite: small
relations:
  closes_dw:
    - {dw_id}
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Close referenced DW.
"#
        ),
    )
    .unwrap();

    // Transition story to done using qdev transition story E1S1 done
    let mut trans_cmd = Command::cargo_bin("qdev").unwrap();
    trans_cmd
        .current_dir(root)
        .args(["transition", "story", "E1S1", "done"])
        .assert()
        .success();

    // Verify DW is now status: done and resolution: E1S1
    let mut check_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_check = check_cmd
        .current_dir(root)
        .args(["dw", "list", "--status", "done", "--json"])
        .assert()
        .success();
    let check_val: Value = serde_json::from_slice(&assert_check.get_output().stdout).unwrap();
    let items = check_val["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], dw_id);
    assert_eq!(items[0]["status"], "done");
    assert_eq!(items[0]["resolution"], "E1S1");
}

#[test]
fn test_cli_dw_add_empty_title_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "   ",
            "--json",
        ])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Title cannot be empty"));
}

#[test]
fn test_cli_dw_close_nonexistent_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["dw", "close", "DW-0000", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "entity_not_found");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("DW-0000"));
}

#[test]
fn test_cli_dw_close_invalid_status_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut add_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_add = add_cmd
        .current_dir(root)
        .args([
            "dw",
            "add",
            "--module",
            "bridge",
            "--risk",
            "negligible",
            "--title",
            "DW for invalid status close test",
            "--json",
        ])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert_add.get_output().stdout).unwrap();
    let dw_id = val["id"].as_str().unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["dw", "close", dw_id, "--status", "in_progress", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Invalid close status"));
}

