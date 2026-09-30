//! Unit tests for skills generation, CommandCatalog, version stamping, and installation per Story 4.4.

use std::fs;
use tempfile::TempDir;

use qdev_core::{
    generate_agent_skills, generate_claude_skills, generate_cursor_rule, inspect_skills,
    install_skills, CommandCatalog, ExitCode, SkillInstallOptions, CORE_SKILL_NAMES,
    MANAGED_SKILL_PATHS,
};

#[test]
fn test_command_catalog_all_contains_commands() {
    let commands = CommandCatalog::all();
    assert!(!commands.is_empty(), "catalog must not be empty");

    let required = [
        "status",
        "init",
        "create",
        "transition",
        "claim",
        "release",
        "context",
        "gate",
        "install",
        "doctor",
    ];

    for name in &required {
        let found = CommandCatalog::find(name);
        assert!(found.is_some(), "command '{}' must exist in catalog", name);
        let cmd = found.unwrap();
        assert!(!cmd.summary.is_empty(), "command '{}' must have a summary", name);
    }
}

#[test]
fn test_skill_template_generation_claude() {
    let skills = generate_claude_skills();
    assert_eq!(skills.len(), 5, "must generate exactly 5 Claude skills");

    let version = env!("CARGO_PKG_VERSION");

    for (rel_path, content) in &skills {
        let path_str = rel_path.to_string_lossy();
        assert!(path_str.starts_with(".claude/skills/"), "path must start with .claude/skills/");
        assert!(path_str.ends_with("/SKILL.md"), "path must end with /SKILL.md");

        // Frontmatter verification
        assert!(
            content.contains(&format!("version: \"{}\"", version)),
            "content must contain version frontmatter: {}",
            content
        );
        assert!(
            content.contains(&format!("qdev_version: \"{}\"", version)),
            "content must contain qdev_version frontmatter: {}",
            content
        );

        // Directive verification
        assert!(
            content.contains("--json"),
            "content must instruct agent to use --json: {}",
            content
        );
        assert!(
            content.contains("qdev context"),
            "content must instruct agent to use qdev context: {}",
            content
        );
    }
}

#[test]
fn test_cursor_rule_generation() {
    let (rel_path, content) = generate_cursor_rule();
    assert_eq!(rel_path.to_string_lossy(), ".cursor/rules/qdev.mdc");

    let version = env!("CARGO_PKG_VERSION");
    assert!(content.contains(&format!("version: \"{}\"", version)));
    assert!(content.contains(&format!("qdev_version: \"{}\"", version)));
    assert!(content.contains("--json"));
    assert!(content.contains("qdev context"));
}

#[test]
fn test_skill_template_generation_agents() {
    let skills = generate_agent_skills();
    assert_eq!(skills.len(), 5, "must generate exactly 5 Agent skills");

    let version = env!("CARGO_PKG_VERSION");

    for (rel_path, content) in &skills {
        let path_str = rel_path.to_string_lossy();
        assert!(path_str.starts_with(".agents/skills/"), "path must start with .agents/skills/");
        assert!(path_str.ends_with("/SKILL.md"), "path must end with /SKILL.md");
        assert!(content.contains(&format!("version: \"{}\"", version)));
        assert!(content.contains(&format!("qdev_version: \"{}\"", version)));
        assert!(content.contains("--json"));
        assert!(content.contains("qdev context"));
    }
}

#[test]
fn test_install_skills_requires_at_least_one_target() {
    let temp = TempDir::new().unwrap();
    let options = SkillInstallOptions {
        claude: false,
        cursor: false,
        agents: false,
    };

    let err = install_skills(temp.path(), &options).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(
        err.message().contains("specify at least one of --claude, --cursor, or --agents"),
        "error message: {}",
        err.message()
    );
}

#[test]
fn test_install_skills_claude() {
    let temp = TempDir::new().unwrap();
    let options = SkillInstallOptions {
        claude: true,
        cursor: false,
        agents: false,
    };

    let report = install_skills(temp.path(), &options).unwrap();
    assert_eq!(report.targets, vec!["claude"]);
    assert_eq!(report.installed_files.len(), 5);
    assert_eq!(report.version, env!("CARGO_PKG_VERSION"));

    for name in CORE_SKILL_NAMES {
        let file = temp.path().join(".claude/skills").join(name).join("SKILL.md");
        assert!(file.is_file(), "file {:?} must exist", file);
    }

    let status = inspect_skills(temp.path()).unwrap();
    assert_eq!(status.installed_count, 5);
    assert_eq!(status.outdated_count, 0);
    assert!(status.up_to_date);
    assert!(status.outdated_skills.is_empty());
}

#[test]
fn test_install_skills_cursor() {
    let temp = TempDir::new().unwrap();
    let options = SkillInstallOptions {
        claude: false,
        cursor: true,
        agents: false,
    };

    let report = install_skills(temp.path(), &options).unwrap();
    assert_eq!(report.targets, vec!["cursor"]);
    assert_eq!(report.installed_files, vec![".cursor/rules/qdev.mdc"]);

    let file = temp.path().join(".cursor/rules/qdev.mdc");
    assert!(file.is_file());

    let status = inspect_skills(temp.path()).unwrap();
    assert_eq!(status.installed_count, 1);
    assert_eq!(status.outdated_count, 0);
    assert!(status.up_to_date);
}

#[test]
fn test_install_skills_agents() {
    let temp = TempDir::new().unwrap();
    let options = SkillInstallOptions {
        claude: false,
        cursor: false,
        agents: true,
    };

    let report = install_skills(temp.path(), &options).unwrap();
    assert_eq!(report.targets, vec!["agents"]);
    assert_eq!(report.installed_files.len(), 5);

    for name in CORE_SKILL_NAMES {
        let file = temp.path().join(".agents/skills").join(name).join("SKILL.md");
        assert!(file.is_file());
    }

    let status = inspect_skills(temp.path()).unwrap();
    assert_eq!(status.installed_count, 5);
    assert_eq!(status.outdated_count, 0);
    assert!(status.up_to_date);
}

#[test]
fn test_install_skills_all_targets() {
    let temp = TempDir::new().unwrap();
    let options = SkillInstallOptions {
        claude: true,
        cursor: true,
        agents: true,
    };

    let report = install_skills(temp.path(), &options).unwrap();
    assert_eq!(report.targets, vec!["claude", "cursor", "agents"]);
    assert_eq!(report.installed_files.len(), 11);
    assert_eq!(report.installed_files.len(), MANAGED_SKILL_PATHS.len());

    let status = inspect_skills(temp.path()).unwrap();
    assert_eq!(status.installed_count, 11);
    assert_eq!(status.outdated_count, 0);
    assert!(status.up_to_date);
}

#[test]
fn test_install_skills_refuses_to_overwrite_non_qdev_file() {
    let temp = TempDir::new().unwrap();
    let alien_skill_dir = temp.path().join(".claude/skills/qdev");
    fs::create_dir_all(&alien_skill_dir).unwrap();
    fs::write(
        alien_skill_dir.join("SKILL.md"),
        "---
name: custom-foreign-skill
description: Not a qdev skill
---
Custom content
",
    )
    .unwrap();

    let options = SkillInstallOptions {
        claude: true,
        cursor: false,
        agents: false,
    };

    let err = install_skills(temp.path(), &options).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
}

#[test]
fn test_inspect_skills_detects_outdated_version() {
    let temp = TempDir::new().unwrap();
    let rule_dir = temp.path().join(".cursor/rules");
    fs::create_dir_all(&rule_dir).unwrap();
    fs::write(
        rule_dir.join("qdev.mdc"),
        "---
description: Test
globs: '*'
version: '0.0.1'
qdev_version: '0.0.1'
---
# Rules
",
    )
    .unwrap();

    let status = inspect_skills(temp.path()).unwrap();
    assert_eq!(status.installed_count, 1);
    assert_eq!(status.outdated_count, 1);
    assert!(!status.up_to_date);
    assert_eq!(status.outdated_skills, vec![".cursor/rules/qdev.mdc"]);
}

#[test]
fn test_inspect_skills_ignores_foreign_files_without_version_stamp() {
    let temp = TempDir::new().unwrap();
    let skill_dir = temp.path().join(".claude/skills/qdev");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---
name: custom-foreign-skill
description: Not a qdev skill
---
# Some alien rule
",
    )
    .unwrap();

    let status = inspect_skills(temp.path()).unwrap();
    // Foreign file lacking a qdev version stamp must be ignored
    assert_eq!(status.installed_count, 0);
    assert_eq!(status.outdated_count, 0);
    assert!(status.up_to_date);
    assert!(status.outdated_skills.is_empty());
}

#[test]
fn test_generate_skill_content_unknown_name_does_not_panic() {
    let content = qdev_core::generate_skill_content("some-unknown-skill");
    assert!(content.contains("some-unknown-skill"));
    assert!(content.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_qdev_develop_quotes_cited_ids_on_refusals() {
    let content = qdev_core::generate_skill_content("qdev-develop");
    assert!(
        content.contains("cited IDs")
            || content.contains("constraint_id")
            || content.contains("gate_id"),
        "qdev-develop must instruct quoting cited IDs on refusals: {}",
        content
    );
}
