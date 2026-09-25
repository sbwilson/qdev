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
            "PreflightTest",
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

    // Set up modules and git config in qdev.toml
    let toml = r#"
[project]
name = "PreflightTest"

[identity]
developer_id = "simon"
teams = ["core-platform"]

[git]
remote = "origin"
integration_branch = "develop"
branching_mode = "story-branch"
require_clean_tree_in_scope = true
max_integration_staleness_commits = 20

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
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    fs::create_dir_all(root.join("crates/foundation/src")).unwrap();
    fs::create_dir_all(root.join("crates/bridge/src")).unwrap();
    fs::write(root.join("crates/foundation/src/lib.rs"), "// foundation\n").unwrap();
    fs::write(root.join("crates/bridge/src/lib.rs"), "// bridge\n").unwrap();
    fs::write(root.join("README.md"), "# Readme\n").unwrap();

    let story_content = r#"---
id: E12S4
title: Test Story
status: in_progress
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
target_modules:
  - foundation
---
# Test Story
"#;
    fs::write(root.join("docs/specs/stories/E12S4.md"), story_content).unwrap();

    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "Initial commit"]);

    // Create develop branch so integration branch exists
    git(root, &["branch", "develop"]);

    // Sync cache so SQLite is populated
    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    sync_cmd.current_dir(root).args(["sync"]).assert().success();
}

#[test]
fn test_cli_preflight_clean_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight"]).assert().success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("✓ preflight: working tree in scope, integration branch fresh, zero blocking findings"));
}

#[test]
fn test_cli_preflight_clean_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight", "--json"]).assert().success();

    let json_val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(json_val["schema_version"], "1");
    assert_eq!(json_val["status"], "pass");
    assert_eq!(
        json_val["summary"],
        "working tree in scope, integration branch fresh, zero blocking findings"
    );

    // Validate against schema
    let schema_str = qdev_core::PayloadKind::Preflight.schema_str();
    let schema_val: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_val).unwrap();
    assert!(validator.is_valid(&json_val));
}

#[test]
fn test_cli_preflight_non_git_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight"]).assert().code(4);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("not_a_git_repository"));

    // Also test --json mode
    let mut json_cmd = Command::cargo_bin("qdev").unwrap();
    let json_assert = json_cmd.current_dir(root).args(["preflight", "--json"]).assert().code(4);
    let json_val: Value = serde_json::from_slice(&json_assert.get_output().stdout).unwrap();
    assert_eq!(json_val["error"]["code"], "not_a_git_repository");
}

#[test]
fn test_cli_preflight_dirty_no_lease_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Modify a non-exempt file
    fs::write(root.join("README.md"), "# Modified\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight"]).assert().code(3);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("✖ preflight: no story lease or chore active"));
    assert!(stderr.contains("Fix: git stash -u"));

    // In JSON mode
    let mut json_cmd = Command::cargo_bin("qdev").unwrap();
    let json_assert = json_cmd.current_dir(root).args(["preflight", "--json"]).assert().code(3);
    let json_val: Value = serde_json::from_slice(&json_assert.get_output().stdout).unwrap();
    assert_eq!(json_val["status"], "refusal");
    assert_eq!(json_val["remediation_commands"][0], "git stash -u");

    let schema_str = qdev_core::PayloadKind::Preflight.schema_str();
    let schema_val: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_val).unwrap();
    assert!(validator.is_valid(&json_val));
}

#[test]
fn test_cli_preflight_story_lease_in_scope_and_out_of_scope() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Claim lease for E12S4 (target_modules: foundation)
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd.current_dir(root).args(["claim", "E12S4"]).assert().success();

    // Modify file inside foundation (in scope)
    fs::write(root.join("crates/foundation/src/lib.rs"), "// updated in foundation\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["preflight"]).assert().success();

    // Now modify file outside foundation (bridge is out of scope for E12S4)
    fs::write(root.join("crates/bridge/src/lib.rs"), "// updated bridge\n").unwrap();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2.current_dir(root).args(["preflight"]).assert().code(3);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("uncommitted changes outside target modules for story 'E12S4'"));
    assert!(stderr.contains("crates/bridge/src/lib.rs"));
    assert!(stderr.contains("Fix: git stash push -u -m \"out-of-scope\" -- \"crates/bridge/src/lib.rs\""));
}

#[test]
fn test_cli_preflight_explicit_story_flag() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Modify file in foundation without lease, but pass `--story E12S4`
    fs::write(root.join("crates/foundation/src/lib.rs"), "// foundation edit\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["preflight", "--story", "E12S4"]).assert().success();

    // Also modify bridge -> should fail
    fs::write(root.join("crates/bridge/src/lib.rs"), "// bridge edit\n").unwrap();
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root).args(["preflight", "--story", "E12S4"]).assert().code(3);
}

#[test]
fn test_cli_preflight_open_chore() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Start chore with allowlist crates/bridge/**
    let mut chore_cmd = Command::cargo_bin("qdev").unwrap();
    chore_cmd
        .current_dir(root)
        .args(["chore", "start", "Tweak bridge", "--paths", "crates/bridge/**"])
        .assert()
        .success();

    // Edit file in bridge -> passes
    fs::write(root.join("crates/bridge/src/lib.rs"), "// chore modification\n").unwrap();
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["preflight"]).assert().success();

    // Edit file in foundation -> out of chore allowlist
    fs::write(root.join("crates/foundation/src/lib.rs"), "// foundation edit\n").unwrap();
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2.current_dir(root).args(["preflight"]).assert().code(3);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("outside allowlist for chore"));
}

#[test]
fn test_cli_preflight_staleness_limit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Update max_integration_staleness_commits to 2 in qdev.toml
    let toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    let new_toml = toml.replace("max_integration_staleness_commits = 20", "max_integration_staleness_commits = 2");
    fs::write(root.join("qdev.toml"), new_toml).unwrap();
    git(root, &["commit", "-am", "Update staleness limit"]);

    // Create feature branch
    git(root, &["checkout", "-b", "feature/E12S4-staleness"]);

    // Advance develop by 3 commits
    git(root, &["checkout", "develop"]);
    for i in 1..=3 {
        fs::write(root.join("crates/foundation/src/lib.rs"), format!("// develop commit {}\n", i)).unwrap();
        git(root, &["commit", "-am", &format!("develop commit {}", i)]);
    }

    // Switch back to feature branch
    git(root, &["checkout", "feature/E12S4-staleness"]);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight", "--story", "E12S4"]).assert().code(3);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("is 3 commits behind develop at merge-base (limit 2)"));
    assert!(stderr.contains("Rebase onto develop before continuing E12S4."));
    assert!(stderr.contains("Fix: git rebase develop"));
}

#[test]
fn test_cli_preflight_integration_branch_behind_remote() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create bare remote
    let remote_dir = TempDir::new().unwrap();
    git(remote_dir.path(), &["init", "--bare"]);
    git(root, &["remote", "add", "origin", remote_dir.path().to_str().unwrap()]);

    git(root, &["checkout", "develop"]);
    git(root, &["push", "-u", "origin", "develop"]);

    // Advance remote develop
    fs::write(root.join("crates/foundation/src/lib.rs"), "// remote commit\n").unwrap();
    git(root, &["commit", "-am", "Remote commit"]);
    git(root, &["push", "origin", "develop"]);

    // Rewind local develop
    git(root, &["reset", "--hard", "HEAD~1"]);

    // When on develop, remediation recommends git pull
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight"]).assert().code(3);
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("local 'develop' is 1 commit(s) behind 'origin/develop'"));
    assert!(stderr.contains("Fix: git pull origin develop"));

    // When on another branch (e.g. main), remediation recommends git fetch
    git(root, &["checkout", "main"]);
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2.current_dir(root).args(["preflight"]).assert().code(3);
    let stderr2 = String::from_utf8_lossy(&assert2.get_output().stderr);
    assert!(stderr2.contains("local 'develop' is 1 commit(s) behind 'origin/develop'"));
    assert!(stderr2.contains("Fix: git fetch origin develop:develop"));
}

#[test]
fn test_cli_preflight_trunk_mode_behind_remote() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Switch to trunk mode in qdev.toml
    let toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    let new_toml = toml.replace("branching_mode = \"story-branch\"", "branching_mode = \"trunk\"");
    fs::write(root.join("qdev.toml"), new_toml).unwrap();
    git(root, &["commit", "-am", "Use trunk mode"]);

    let remote_dir = TempDir::new().unwrap();
    git(remote_dir.path(), &["init", "--bare"]);
    git(root, &["remote", "add", "origin", remote_dir.path().to_str().unwrap()]);
    git(root, &["push", "-u", "origin", "main"]);

    // Remote commit
    fs::write(root.join("crates/foundation/src/lib.rs"), "// remote main\n").unwrap();
    git(root, &["commit", "-am", "Remote main"]);
    git(root, &["push", "origin", "main"]);

    // Rewind local
    git(root, &["reset", "--hard", "HEAD~1"]);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight"]).assert().code(3);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("branch 'main' is 1 commit(s) behind 'origin/main'"));
    assert!(stderr.contains("Fix: git pull origin main"));
}

#[test]
fn test_cli_preflight_blocking_validation_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Introduce an unparseable story YAML file that triggers an error finding on validate
    fs::write(
        root.join("docs/specs/stories/E12S99.md"),
        "---\nid: E12S99\ntitle: Broken Story\nstatus: [bad yaml\n---\n",
    ).unwrap();

    let mut sync_cmd = Command::cargo_bin("qdev").unwrap();
    // sync records hydration errors into cache
    let _ = sync_cmd.current_dir(root).args(["sync"]).assert();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.current_dir(root).args(["preflight"]).assert().code(3);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("workspace has") && stderr.contains("blocking validation error"));
    assert!(stderr.contains("Fix: qdev validate"));
}
