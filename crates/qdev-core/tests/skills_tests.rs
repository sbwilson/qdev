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

#[test]
fn test_skills_frontmatter_model_tier_hints_defaults() {
    let qdev = qdev_core::generate_skill_content("qdev");
    assert!(qdev.contains("model_hint: \"reasoning\""));
    assert!(qdev.contains("model: \"reasoning\""));

    let plan = qdev_core::generate_skill_content("qdev-plan");
    assert!(plan.contains("model_hint: \"reasoning\""));
    assert!(plan.contains("model: \"reasoning\""));

    let create_story = qdev_core::generate_skill_content("qdev-create-story");
    assert!(create_story.contains("model_hint: \"reasoning\""));
    assert!(create_story.contains("model: \"reasoning\""));

    let develop = qdev_core::generate_skill_content("qdev-develop");
    assert!(develop.contains("model_hint: \"fast-coding\""));
    assert!(develop.contains("model: \"fast-coding\""));

    let review = qdev_core::generate_skill_content("qdev-review");
    assert!(review.contains("model_hint: \"strongest\""));
    assert!(review.contains("model: \"strongest\""));
}

#[test]
fn test_skills_frontmatter_custom_models_config() {
    let models = qdev_core::ModelsConfig {
        specify: Some("custom-reasoner".to_string()),
        develop: Some("custom-coder".to_string()),
        review: Some("custom-reviewer".to_string()),
    };

    let qdev = qdev_core::generate_skill_content_with_models("qdev", Some(&models));
    assert!(qdev.contains("model_hint: \"custom-reasoner\""));
    assert!(qdev.contains("model: \"custom-reasoner\""));

    let plan = qdev_core::generate_skill_content_with_models("qdev-plan", Some(&models));
    assert!(plan.contains("model_hint: \"custom-reasoner\""));
    assert!(plan.contains("model: \"custom-reasoner\""));

    let create = qdev_core::generate_skill_content_with_models("qdev-create-story", Some(&models));
    assert!(create.contains("model_hint: \"custom-reasoner\""));
    assert!(create.contains("model: \"custom-reasoner\""));

    let dev = qdev_core::generate_skill_content_with_models("qdev-develop", Some(&models));
    assert!(dev.contains("model_hint: \"custom-coder\""));
    assert!(dev.contains("model: \"custom-coder\""));

    let rev = qdev_core::generate_skill_content_with_models("qdev-review", Some(&models));
    assert!(rev.contains("model_hint: \"custom-reviewer\""));
    assert!(rev.contains("model: \"custom-reviewer\""));

    // Verify generate_cursor_rule_with_models with custom models
    let (_, cursor) = qdev_core::generate_cursor_rule_with_models(Some(&models));
    assert!(cursor.contains("model_hint: \"custom-reasoner\""));
    assert!(cursor.contains("model: \"custom-reasoner\""));

    // Verify generate_agent_skills_with_models with custom models
    let agent_skills = qdev_core::generate_agent_skills_with_models(Some(&models));
    assert_eq!(agent_skills.len(), 5);
    for (rel_path, content) in &agent_skills {
        let path_str = rel_path.to_string_lossy();
        if path_str.contains("qdev-develop") {
            assert!(content.contains("model_hint: \"custom-coder\""));
        } else if path_str.contains("qdev-review") {
            assert!(content.contains("model_hint: \"custom-reviewer\""));
        } else {
            assert!(content.contains("model_hint: \"custom-reasoner\""));
        }
    }

    // Partial config fallback
    let partial = qdev_core::ModelsConfig {
        specify: Some("partial-reasoner".to_string()),
        develop: None,
        review: None,
    };
    let partial_dev = qdev_core::generate_skill_content_with_models("qdev-develop", Some(&partial));
    assert!(partial_dev.contains("model_hint: \"fast-coding\""));
    let partial_rev = qdev_core::generate_skill_content_with_models("qdev-review", Some(&partial));
    assert!(partial_rev.contains("model_hint: \"strongest\""));
}

#[test]
fn test_qdev_plan_multi_perspective_synthesis_template() {
    let plan = qdev_core::generate_skill_content("qdev-plan");

    // Must contain Structured Multi-Perspective Synthesis Template
    assert!(
        plan.contains("### Structured Multi-Perspective Synthesis Template"),
        "qdev-plan must contain Structured Multi-Perspective Synthesis Template: {}",
        plan
    );

    // Must contain all four mandatory headings
    assert!(plan.contains("Product & domain value"));
    assert!(plan.contains("Architectural constraints"));
    assert!(plan.contains("Safety & risk profile"));
    assert!(plan.contains("Implementation directives"));

    // Must instruct entity creation and relationship establishment
    assert!(plan.contains("qdev create story"));
    assert!(plan.contains("qdev relate"));
}

#[test]
fn test_qdev_pulse_rendering_instructions() {
    let qdev = qdev_core::generate_skill_content("qdev");
    assert!(qdev.contains("qdev --json"));
    assert!(qdev.contains("Workspace status"));
    assert!(qdev.contains("Sprint state"));
    assert!(qdev.contains("Gate status"));
    assert!(qdev.contains("leases"));
    assert!(qdev.contains("Next recommended steps"));
}

#[test]
fn test_qdev_create_story_workflow() {
    let create = qdev_core::generate_skill_content("qdev-create-story");
    assert!(create.contains("qdev context <epic-id> --phase specify --json"));
    assert!(create.contains("qdev create story"));
    assert!(create.contains("qdev constraint add"));
    assert!(create.contains("qdev transition story <story-id> ready --json"));
}

#[test]
fn test_qdev_develop_workflow() {
    let dev = qdev_core::generate_skill_content("qdev-develop");
    assert!(dev.contains("qdev preflight --story <story-id> --json"));
    assert!(dev.contains("qdev claim <story-id> --json"));
    assert!(dev.contains("qdev context <story-id> --phase develop --json"));
    assert!(dev.contains("qdev scratch append"));
    assert!(dev.contains("qdev gate run --for-transition review --story <story-id> --json"));
    assert!(dev.contains("qdev transition story <story-id> review --json"));
    assert!(dev.contains("constraint_id"));
    assert!(dev.contains("gate_id"));
    assert!(dev.contains("policy"));
    assert!(dev.contains("blocking_ids"));
    assert!(dev.contains("holder"));
}

#[test]
fn test_qdev_review_workflow() {
    let rev = qdev_core::generate_skill_content("qdev-review");
    assert!(rev.contains("qdev context <story-id> --phase review --json"));
    assert!(rev.contains("qdev impact <story-id> --json"));
    assert!(rev.contains("qdev gate run --all"));
    assert!(rev.contains("qdev transition story <story-id> done --json"));
    assert!(rev.contains("qdev transition story <story-id> in_progress --justification"));
}

#[test]
fn test_cursor_rule_synthesis_template_and_models() {
    let (_, content) = qdev_core::generate_cursor_rule();
    assert!(content.contains("Structured Multi-Perspective Synthesis Template"));
    assert!(content.contains("Product & domain value"));
    assert!(content.contains("Architectural constraints"));
    assert!(content.contains("Safety & risk profile"));
    assert!(content.contains("Implementation directives"));
    assert!(content.contains("model_hint: \"reasoning\""));
    assert!(content.contains("model: \"reasoning\""));
}

#[test]
fn test_install_skills_loads_workspace_qdev_toml_models() {
    let temp = TempDir::new().unwrap();
    let qdev_toml = r#"
[project]
name = "CustomModelsTest"

[models]
specify = "workspace-reasoner"
develop = "workspace-coder"
review = "workspace-reviewer"
"#;
    fs::write(temp.path().join("qdev.toml"), qdev_toml).unwrap();

    let options = SkillInstallOptions {
        claude: true,
        cursor: false,
        agents: false,
    };

    // 2-argument install_skills auto-loads from workspace qdev.toml
    let report = install_skills(temp.path(), &options).unwrap();
    assert_eq!(report.installed_files.len(), 5);

    let plan_file = temp.path().join(".claude/skills/qdev-plan/SKILL.md");
    let plan_content = fs::read_to_string(plan_file).unwrap();
    assert!(plan_content.contains("model_hint: \"workspace-reasoner\""));
    assert!(plan_content.contains("model: \"workspace-reasoner\""));

    let dev_file = temp.path().join(".claude/skills/qdev-develop/SKILL.md");
    let dev_content = fs::read_to_string(dev_file).unwrap();
    assert!(dev_content.contains("model_hint: \"workspace-coder\""));

    let rev_file = temp.path().join(".claude/skills/qdev-review/SKILL.md");
    let rev_content = fs::read_to_string(rev_file).unwrap();
    assert!(rev_content.contains("model_hint: \"workspace-reviewer\""));
}

#[test]
fn test_install_skills_with_explicit_models() {
    let temp = TempDir::new().unwrap();
    let models = qdev_core::ModelsConfig {
        specify: Some("explicit-reasoner".to_string()),
        develop: Some("explicit-coder".to_string()),
        review: Some("explicit-reviewer".to_string()),
    };
    let options = SkillInstallOptions {
        claude: true,
        cursor: true,
        agents: true,
    };
    let report = qdev_core::install_skills_with_models(temp.path(), &options, Some(&models)).unwrap();
    assert_eq!(report.installed_files.len(), 11);
    let dev_file = temp.path().join(".claude/skills/qdev-develop/SKILL.md");
    let dev_content = fs::read_to_string(dev_file).unwrap();
    assert!(dev_content.contains("model_hint: \"explicit-coder\""));
}

#[test]
fn test_skills_frontmatter_escapes_yaml_model_hints() {
    let models = qdev_core::ModelsConfig {
        specify: Some("custom\"reasoner\\test".to_string()),
        develop: None,
        review: None,
    };
    let content = qdev_core::generate_skill_content_with_models("qdev-plan", Some(&models));
    assert!(content.contains("model_hint: \"custom\\\"reasoner\\\\test\""));
    assert!(content.contains("model: \"custom\\\"reasoner\\\\test\""));
}
