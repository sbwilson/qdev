//! Integration tests for `qdev install skills` CLI and doctor skill detection per Story 4.4.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
#[path = "../src/cli.rs"]
#[allow(dead_code)]
mod cli;

use clap::CommandFactory;
use cli::Cli;
use qdev_core::{CommandCatalog, PayloadKind, CORE_SKILL_NAMES, MANAGED_SKILL_PATHS};
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

#[test]
fn test_command_catalog_aligned_with_clap_cli() {
    let mut clap_cmd = Cli::command();
    clap_cmd.build();

    // Direction 1: Every command in CommandCatalog exists in clap's Cli::command()
    for cmd in CommandCatalog::all() {
        let sub = clap_cmd
            .find_subcommand(cmd.name)
            .unwrap_or_else(|| panic!("command '{}' from CommandCatalog not found in clap CLI", cmd.name));

        for subcmd in cmd.subcommands {
            assert!(
                sub.find_subcommand(subcmd.name).is_some(),
                "subcommand '{} {}' from CommandCatalog not found in clap CLI",
                cmd.name,
                subcmd.name
            );
        }
    }

    // Direction 2: Every clap CLI subcommand exists in CommandCatalog
    for clap_sub in clap_cmd.get_subcommands() {
        let name = clap_sub.get_name();
        if name == "help" {
            continue;
        }
        let catalog_cmd = CommandCatalog::find(name)
            .unwrap_or_else(|| panic!("clap CLI command '{}' not found in CommandCatalog", name));

        for clap_child in clap_sub.get_subcommands() {
            let child_name = clap_child.get_name();
            if child_name == "help" {
                continue;
            }
            assert!(
                catalog_cmd.subcommands.iter().any(|s| s.name == child_name),
                "clap CLI subcommand '{} {}' not found in CommandCatalog",
                name,
                child_name
            );
        }
    }
}

#[test]
fn test_install_skills_missing_flags_exits_2_usage_error() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(temp.path())
        .args(["install", "skills"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicates::str::contains("specify at least one of --claude, --cursor, or --agents"));

    // Also test in --json mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert = cmd_json
        .current_dir(temp.path())
        .args(["install", "skills", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let val: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("specify at least one of --claude, --cursor, or --agents"));
}

#[test]
fn test_install_skills_claude() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(temp.path())
        .args(["install", "skills", "--claude"])
        .assert()
        .success()
        .code(0)
        .stdout(predicates::str::contains("Installed 5 skill/rule file(s) across target(s) [claude]"));

    let version = env!("CARGO_PKG_VERSION");
    for skill in CORE_SKILL_NAMES {
        let path = temp.path().join(".claude/skills").join(skill).join("SKILL.md");
        assert!(path.is_file(), "file {:?} must exist", path);
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(&format!("version: \"{}\"", version)));
        assert!(content.contains(&format!("qdev_version: \"{}\"", version)));
        assert!(content.contains("--json"));
        assert!(content.contains("qdev context"));
    }
}

#[test]
fn test_install_skills_cursor() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(temp.path())
        .args(["install", "skills", "--cursor"])
        .assert()
        .success()
        .code(0)
        .stdout(predicates::str::contains("Installed 1 skill/rule file(s) across target(s) [cursor]"));

    let rule_file = temp.path().join(".cursor/rules/qdev.mdc");
    assert!(rule_file.is_file());
    let content = fs::read_to_string(&rule_file).unwrap();
    assert!(content.contains(env!("CARGO_PKG_VERSION")));
    assert!(content.contains("--json"));
    assert!(content.contains("qdev context"));
}

#[test]
fn test_install_skills_agents() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(temp.path())
        .args(["install", "skills", "--agents"])
        .assert()
        .success()
        .code(0)
        .stdout(predicates::str::contains("Installed 5 skill/rule file(s) across target(s) [agents]"));

    for skill in CORE_SKILL_NAMES {
        let path = temp.path().join(".agents/skills").join(skill).join("SKILL.md");
        assert!(path.is_file());
    }
}

#[test]
fn test_install_skills_all_targets_json() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(temp.path())
        .args(["install", "skills", "--claude", "--cursor", "--agents", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["version"], env!("CARGO_PKG_VERSION"));

    let targets = val["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 3);
    assert_eq!(targets[0], "claude");
    assert_eq!(targets[1], "cursor");
    assert_eq!(targets[2], "agents");

    let files = val["installed_files"].as_array().unwrap();
    assert_eq!(files.len(), 11);
    assert_eq!(files.len(), MANAGED_SKILL_PATHS.len());

    // Validate against schema
    let schema_json = PayloadKind::SkillInstall.schema_json();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    let errors: Vec<String> = validator.iter_errors(&val).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "payload must validate against schema: {:?}", errors);
}

#[test]
fn test_doctor_detects_outdated_skills_and_reports_mismatch() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    // Install claude skills
    let mut install_cmd = Command::cargo_bin("qdev").unwrap();
    install_cmd
        .current_dir(temp.path())
        .args(["install", "skills", "--claude"])
        .assert()
        .success();

    // Overwrite one skill with older version
    let target = temp.path().join(".claude/skills/qdev/SKILL.md");
    fs::write(
        &target,
        r#"---
name: qdev
description: Outdated
version: "0.0.9"
qdev_version: "0.0.9"
---
# Outdated
"#,
    )
    .unwrap();

    // Run doctor --json
    let mut doc_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = doc_cmd
        .current_dir(temp.path())
        .args(["doctor", "--json"])
        .assert()
        .success() // Doctor is diagnostic, not a gate
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let skills_sec = sections
        .iter()
        .find(|s| s["name"] == "skills")
        .expect("skills section must be present");

    assert_eq!(skills_sec["status"], "mismatch");
    assert_eq!(skills_sec["up_to_date"], false);
    assert_eq!(skills_sec["installed_count"], 5);
    assert_eq!(skills_sec["outdated_count"], 1);
    let outdated = skills_sec["outdated_skills"].as_array().unwrap();
    assert_eq!(outdated.len(), 1);
    assert_eq!(outdated[0], ".claude/skills/qdev/SKILL.md");
}

#[test]
fn test_doctor_clean_skills_reports_ok() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut install_cmd = Command::cargo_bin("qdev").unwrap();
    install_cmd
        .current_dir(temp.path())
        .args(["install", "skills", "--claude", "--cursor"])
        .assert()
        .success();

    let mut doc_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = doc_cmd
        .current_dir(temp.path())
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let skills_sec = sections
        .iter()
        .find(|s| s["name"] == "skills")
        .expect("skills section must be present");

    assert_eq!(skills_sec["status"], "ok");
    assert_eq!(skills_sec["up_to_date"], true);
    assert_eq!(skills_sec["installed_count"], 6);
    assert_eq!(skills_sec["outdated_count"], 0);
    assert_eq!(skills_sec["outdated_skills"].as_array().unwrap().len(), 0);
}

#[test]
fn test_doctor_no_skills_installed_reports_ok() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut doc_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = doc_cmd
        .current_dir(temp.path())
        .args(["doctor", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = val["sections"].as_array().unwrap();
    let skills_sec = sections
        .iter()
        .find(|s| s["name"] == "skills")
        .expect("skills section must be present");

    assert_eq!(skills_sec["status"], "ok");
    assert_eq!(skills_sec["up_to_date"], true);
    assert_eq!(skills_sec["installed_count"], 0);
    assert_eq!(skills_sec["outdated_count"], 0);
}

#[test]
fn test_schema_payload_skill_install() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.args(["schema", "payload", "skill_install"])
        .assert()
        .success()
        .code(0)
        .stdout(predicates::str::contains("Skill Install Payload Schema"));

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.args(["schema", "payload", "skills_install"])
        .assert()
        .success()
        .code(0)
        .stdout(predicates::str::contains("Skill Install Payload Schema"));
}

#[test]
fn test_doctor_text_clean_skills() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut install_cmd = Command::cargo_bin("qdev").unwrap();
    install_cmd
        .current_dir(temp.path())
        .args(["install", "skills", "--claude"])
        .assert()
        .success();

    let mut doc_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = doc_cmd
        .current_dir(temp.path())
        .args(["doctor"])
        .assert()
        .success()
        .code(0);

    let text = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(text.contains("[skills]"), "output must contain [skills]: {}", text);
    assert!(text.contains("status = ok"), "output must contain status = ok: {}", text);
    assert!(
        text.contains("outdated_skills = none"),
        "output must contain outdated_skills = none: {}",
        text
    );
}

#[test]
fn test_doctor_text_outdated_skills() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let mut install_cmd = Command::cargo_bin("qdev").unwrap();
    install_cmd
        .current_dir(temp.path())
        .args(["install", "skills", "--claude"])
        .assert()
        .success();

    // Overwrite one skill with older version
    let target = temp.path().join(".claude/skills/qdev/SKILL.md");
    fs::write(
        &target,
        r#"---
name: qdev
description: Outdated
version: "0.0.9"
qdev_version: "0.0.9"
---
# Outdated
"#,
    )
    .unwrap();

    let mut doc_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = doc_cmd
        .current_dir(temp.path())
        .args(["doctor"])
        .assert()
        .success()
        .code(0);

    let text = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(text.contains("[skills]"), "output must contain [skills]: {}", text);
    assert!(
        text.contains("status = mismatch"),
        "output must contain status = mismatch: {}",
        text
    );
    assert!(
        text.contains("outdated_skills = .claude/skills/qdev/SKILL.md"),
        "output must contain outdated path: {}",
        text
    );
}
