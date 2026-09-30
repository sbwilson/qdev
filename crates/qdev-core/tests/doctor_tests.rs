//! `qdev doctor` section tests at the `Store` level (spec-doctor-sees-computed-findings).
//!
//! The broken-cache row of the spec's I/O matrix is only reachable here: every `qdev` invocation
//! in an initialized workspace runs the boot-time `ensure_cache` sweep, which rebuilds a cache
//! whose tables are missing before any command handler opens it — so the CLI can never observe
//! one. `SqliteStore::open` does not create the schema, which makes the state constructible.

use qdev_core::config::Config;
use qdev_core::store::sqlite::SqliteStore;
use qdev_core::{CacheDoctorSection, DoctorSection, ValidationDoctorSection};
use tempfile::TempDir;

fn field<'a>(report: &'a qdev_core::DoctorSectionReport, key: &str) -> &'a serde_json::Value {
    &report
        .fields
        .iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("field '{}' must be present", key))
        .1
}

/// `doctor` is the command a user runs when something is already wrong, so a validation pass it
/// cannot complete must be reported, not fatal.
#[test]
fn test_validation_section_reports_unavailable_on_a_cache_missing_its_tables() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open(root.join("cache.sqlite")).unwrap();

    let cache = CacheDoctorSection.run(&store).unwrap();
    assert_eq!(*field(&cache, "schema_status"), "mismatch");
    assert!(
        !field(&cache, "missing_tables")
            .as_array()
            .unwrap()
            .is_empty(),
        "the fixture's whole point is a cache with no tables: {:?}",
        cache
    );

    let validation =
        ValidationDoctorSection::new(root.to_path_buf(), Config::default()).run(&store);
    let validation = validation.expect("a failed validation must be reported, not propagated");

    assert_eq!(*field(&validation, "status"), "unavailable");
    assert!(field(&validation, "finding_count").is_null());
    assert!(field(&validation, "findings_by_code").is_null());
    // The reason travels with the status: `cache`'s `schema_status` explains a cache-shaped
    // failure, but nothing else in the payload would explain any other kind.
    assert!(
        field(&validation, "unavailable_reason").is_string(),
        "an unavailable pass must name the error code that stopped it: {:?}",
        validation
    );
}

/// The section keeps its documented field order and reports zero — not `unavailable` — for a
/// healthy but empty workspace, with no reason attached.
#[test]
fn test_validation_section_reports_zero_for_an_empty_healthy_cache() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let report = ValidationDoctorSection::new(temp.path().to_path_buf(), Config::default())
        .run(&store)
        .unwrap();

    assert_eq!(report.name, "validation");
    let keys: Vec<&str> = report.fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "status",
            "unavailable_reason",
            "finding_count",
            "findings_by_code"
        ]
    );
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "finding_count"), 0);
    assert_eq!(*field(&report, "findings_by_code"), serde_json::json!({}));
}

/// `default_doctor_sections` stays the single wiring point, and reports `cache` before
/// `validation` before `leases` before `hooks` before `skills` before `mcp` before `git` before
/// `modules` before `gates`.
#[test]
fn test_default_doctor_sections_order() {
    let temp = TempDir::new().unwrap();
    let sections = qdev_core::default_doctor_sections(temp.path(), &Config::default());
    let names: Vec<&str> = sections.iter().map(|s| s.name()).collect();
    assert_eq!(
        names,
        vec![
            "cache",
            "validation",
            "leases",
            "hooks",
            "skills",
            "mcp",
            "git",
            "modules",
            "gates"
        ]
    );
}

#[test]
fn test_doctor_hooks_section_reports_unavailable_on_non_git_repo() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let section = qdev_core::HooksDoctorSection::new(temp.path().to_path_buf());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "hooks");
    assert_eq!(*field(&report, "status"), "unavailable");
    assert_eq!(
        *field(&report, "unavailable_reason"),
        "not_a_git_repository"
    );
    assert!(field(&report, "all_installed").is_null());
    assert!(field(&report, "missing_hooks").is_null());
    assert!(field(&report, "outdated_hooks").is_null());
}

#[test]
fn test_doctor_skills_section_uninstalled_reports_ok() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let section = qdev_core::SkillsDoctorSection::new(temp.path().to_path_buf());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "skills");
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "installed_count"), 0);
    assert_eq!(*field(&report, "outdated_count"), 0);
    assert_eq!(*field(&report, "up_to_date"), true);
    assert_eq!(*field(&report, "outdated_skills"), serde_json::json!([]));
}

#[test]
fn test_doctor_skills_section_clean_reports_ok() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let options = qdev_core::SkillInstallOptions {
        claude: true,
        cursor: false,
        agents: false,
    };
    qdev_core::install_skills(temp.path(), &options).unwrap();

    let section = qdev_core::SkillsDoctorSection::new(temp.path().to_path_buf());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "skills");
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "installed_count"), 5);
    assert_eq!(*field(&report, "outdated_count"), 0);
    assert_eq!(*field(&report, "up_to_date"), true);
    assert_eq!(*field(&report, "outdated_skills"), serde_json::json!([]));
}

#[test]
fn test_doctor_skills_section_outdated_reports_mismatch() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let claude_skill_dir = temp.path().join(".claude/skills/qdev");
    std::fs::create_dir_all(&claude_skill_dir).unwrap();
    std::fs::write(
        claude_skill_dir.join("SKILL.md"),
        r#"---
name: qdev
description: "Outdated skill"
version: "0.0.9"
qdev_version: "0.0.9"
---
# Outdated
"#,
    )
    .unwrap();

    let section = qdev_core::SkillsDoctorSection::new(temp.path().to_path_buf());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "skills");
    assert_eq!(*field(&report, "status"), "mismatch");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "installed_count"), 1);
    assert_eq!(*field(&report, "outdated_count"), 1);
    assert_eq!(*field(&report, "up_to_date"), false);
    assert_eq!(
        *field(&report, "outdated_skills"),
        serde_json::json!([".claude/skills/qdev/SKILL.md"])
    );
}

#[test]
fn test_doctor_git_section_non_git_reports_unavailable() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let section = qdev_core::GitDoctorSection::new(temp.path().to_path_buf(), Config::default());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "git");
    assert_eq!(*field(&report, "status"), "unavailable");
    assert_eq!(
        *field(&report, "unavailable_reason"),
        "not_a_git_repository"
    );
    assert!(field(&report, "clean").is_null());
    assert!(field(&report, "dirty_files").is_null());
}

#[test]
fn test_doctor_git_section_clean_reports_ok() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    std::process::Command::new("git")
        .args(["init", "-b", "develop"])
        .current_dir(root)
        .output()
        .expect("git init");
    std::process::Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Simon"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "simon@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--no-gpg-sign", "-m", "init"])
        .current_dir(root)
        .output()
        .unwrap();
    let origin_temp = TempDir::new().unwrap();
    let origin_dir = origin_temp.path().join("origin.git");
    std::process::Command::new("git")
        .args(["init", "--bare", origin_dir.to_str().unwrap()])
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["remote", "add", "origin", origin_dir.to_str().unwrap()])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["push", "-u", "origin", "develop"])
        .current_dir(root)
        .output()
        .unwrap();

    let section = qdev_core::GitDoctorSection::new(root.to_path_buf(), Config::default());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "git");
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "clean"), true);
    assert_eq!(*field(&report, "dirty_files"), 0);
    assert_eq!(*field(&report, "integration_state"), "up_to_date");
}

#[test]
fn test_doctor_git_section_missing_remote_ref_reports_mismatch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    std::process::Command::new("git")
        .args(["init", "-b", "develop"])
        .current_dir(root)
        .output()
        .expect("git init");
    std::process::Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.name", "Simon"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "simon@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    std::fs::write(root.join("README.md"), "hello\n").unwrap();
    std::process::Command::new("git")
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["commit", "--no-gpg-sign", "-m", "init"])
        .current_dir(root)
        .output()
        .unwrap();

    let section = qdev_core::GitDoctorSection::new(root.to_path_buf(), Config::default());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "git");
    assert_eq!(*field(&report, "status"), "mismatch");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "clean"), true);
    assert_eq!(*field(&report, "integration_state"), "remote_ref_missing");
}

#[test]
fn test_doctor_git_section_dirty_reports_mismatch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    std::process::Command::new("git")
        .args(["init", "-b", "develop"])
        .current_dir(root)
        .output()
        .expect("git init");
    std::process::Command::new("git")
        .args(["config", "user.name", "Simon"])
        .current_dir(root)
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["config", "user.email", "simon@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    std::fs::write(root.join("uncommitted.txt"), "dirty\n").unwrap();

    let section = qdev_core::GitDoctorSection::new(root.to_path_buf(), Config::default());
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "git");
    assert_eq!(*field(&report, "status"), "mismatch");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "clean"), false);
    assert_eq!(*field(&report, "dirty_files"), 1);
}

#[test]
fn test_doctor_modules_section_clean_reports_ok() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    std::fs::create_dir_all(root.join("crates/foundation")).unwrap();
    std::fs::write(root.join("crates/foundation/lib.rs"), "// code\n").unwrap();

    let mut config = Config::default();
    config.modules = vec![qdev_core::config::ModuleConfig {
        id: "foundation".to_string(),
        paths: vec!["crates/foundation/**".to_string()],
        layer: Some(0),
        may_depend_on: Vec::new(),
    }];

    let section = qdev_core::ModulesDoctorSection::new(root.to_path_buf(), config);
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "modules");
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "declared_count"), 1);
    assert_eq!(*field(&report, "unmatched_count"), 0);
    assert_eq!(*field(&report, "unmatched_globs"), serde_json::json!([]));
}

#[test]
fn test_doctor_modules_section_unmatched_reports_mismatch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    let mut config = Config::default();
    config.modules = vec![qdev_core::config::ModuleConfig {
        id: "missing_module".to_string(),
        paths: vec!["nonexistent/**".to_string()],
        layer: Some(0),
        may_depend_on: Vec::new(),
    }];

    let section = qdev_core::ModulesDoctorSection::new(root.to_path_buf(), config);
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "modules");
    assert_eq!(*field(&report, "status"), "mismatch");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "declared_count"), 1);
    assert_eq!(*field(&report, "unmatched_count"), 1);
    assert_eq!(
        *field(&report, "unmatched_globs"),
        serde_json::json!(["nonexistent/**"])
    );
}

#[test]
fn test_doctor_gates_section_clean_and_skipped_locally() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    let mut config = Config::default();
    config.gates = vec![
        qdev_core::config::GateConfig {
            id: "qdev-scope".to_string(),
            command: None,
            timeout_ms: None,
            depends_on: Vec::new(),
            output_adapter: None,
            on_transition: Vec::new(),
            verifies: Vec::new(),
            kind: None,
            metric: None,
            direction: None,
            skip: None,
        },
        qdev_core::config::GateConfig {
            id: "cargo-check".to_string(),
            command: Some("nonexistent_binary_xyz_123".to_string()),
            timeout_ms: None,
            depends_on: Vec::new(),
            output_adapter: None,
            on_transition: Vec::new(),
            verifies: Vec::new(),
            kind: None,
            metric: None,
            direction: None,
            skip: Some(true),
        },
    ];

    let section = qdev_core::GatesDoctorSection::new(root.to_path_buf(), config);
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "gates");
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "configured_count"), 2);
    assert_eq!(*field(&report, "missing_count"), 0);
    assert_eq!(*field(&report, "skipped_locally_count"), 1);
}

#[test]
fn test_doctor_gates_section_missing_executable_reports_mismatch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    let mut config = Config::default();
    config.gates = vec![qdev_core::config::GateConfig {
        id: "nonexistent-gate".to_string(),
        command: Some("this_binary_really_does_not_exist_xyz123 --check".to_string()),
        timeout_ms: None,
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let section = qdev_core::GatesDoctorSection::new(root.to_path_buf(), config);
    let report = section.run(&store).unwrap();

    assert_eq!(report.name, "gates");
    assert_eq!(*field(&report, "status"), "mismatch");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "configured_count"), 1);
    assert_eq!(*field(&report, "missing_count"), 1);
    assert_eq!(*field(&report, "skipped_locally_count"), 0);
}
