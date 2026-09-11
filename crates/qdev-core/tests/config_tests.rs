use std::fs;
use std::process::Command;
use tempfile::TempDir;

use qdev_core::{load_config, ConfigSource, ExitCode};

#[test]
fn test_both_configs_exist_merging() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "ProjectFromProject"
default_sprint = 10

[git]
remote = "upstream"
integration_branch = "main"

[identity]
developer_id = "project_dev"
"#;

    let local_toml = r#"
[project]
name = "ProjectFromLocal"

[identity]
developer_id = "local_dev"
teams = ["core-team"]
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).expect("Configuration should load successfully");
    let cfg = &annotated.config;

    // Local overrides project individually
    assert_eq!(cfg.project.name, "ProjectFromLocal");
    assert_eq!(
        annotated.get_source("project.name"),
        Some(ConfigSource::Local)
    );

    // Project key preserved when not in local
    assert_eq!(cfg.project.default_sprint, Some(10));
    assert_eq!(
        annotated.get_source("project.default_sprint"),
        Some(ConfigSource::Project)
    );

    assert_eq!(cfg.git.remote, "upstream");
    assert_eq!(
        annotated.get_source("git.remote"),
        Some(ConfigSource::Project)
    );

    assert_eq!(cfg.identity.developer_id, "local_dev");
    assert_eq!(
        annotated.get_source("identity.developer_id"),
        Some(ConfigSource::Local)
    );
    assert_eq!(cfg.identity.teams, vec!["core-team"]);
}

#[test]
fn test_local_config_absent_fallback_git() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let git_init = Command::new("git")
        .args(["init"])
        .current_dir(root)
        .output()
        .expect("git init failed");
    assert!(git_init.status.success());

    let git_config = Command::new("git")
        .args(["config", "user.email", "dev@example.com"])
        .current_dir(root)
        .output()
        .expect("git config user.email failed");
    assert!(git_config.status.success());

    let project_toml = r#"
[project]
name = "ProjectOnly"
default_sprint = 3
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    // No .qdev.local.toml

    let annotated = load_config(root).expect("Absence of local config is not an error");
    let cfg = &annotated.config;

    assert_eq!(cfg.project.name, "ProjectOnly");
    assert_eq!(cfg.project.default_sprint, Some(3));

    // identity.developer_id falls back to git config user.email unconditionally
    assert_eq!(cfg.identity.developer_id, "dev@example.com");
    assert_eq!(
        annotated.get_source("identity.developer_id"),
        Some(ConfigSource::Git)
    );
}

#[test]
fn test_both_configs_absent_default() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let annotated = load_config(root).expect("Both configs absent loads default configuration");
    let cfg = &annotated.config;

    assert_eq!(cfg.project.name, "");
    assert_eq!(cfg.project.default_sprint, None);
    assert_eq!(cfg.git.remote, "origin");
    assert_eq!(cfg.git.integration_branch, "develop");
    assert_eq!(cfg.storage.specs_dir, "docs/specs");
    assert!(cfg.modules.is_empty());
    assert!(cfg.gates.is_empty());
}

#[test]
fn test_array_merge_isolation_gates() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[gates]]
id = "fmt"
command = "cargo fmt --all --check"

[[gates]]
id = "lint"
command = "cargo clippy"
"#;

    let local_toml = r#"
[[gates]]
id = "local-only-gate"
command = "echo local"
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).unwrap();
    let cfg = &annotated.config;

    // Must NEVER merge element-wise: local replaces project entirely
    assert_eq!(cfg.gates.len(), 1);
    assert_eq!(cfg.gates[0].id, "local-only-gate");
    assert_eq!(cfg.gates[0].command.as_deref(), Some("echo local"));
    assert_eq!(annotated.get_source("gates"), Some(ConfigSource::Local));
}

#[test]
fn test_array_merge_isolation_modules() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[modules]]
id = "core"
paths = ["crates/core/**"]

[[modules]]
id = "cli"
paths = ["crates/cli/**"]
"#;

    let local_toml = r#"
[[modules]]
id = "my-module"
paths = ["crates/my/**"]
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).unwrap();
    let cfg = &annotated.config;

    // Must NEVER merge element-wise: local replaces project entirely
    assert_eq!(cfg.modules.len(), 1);
    assert_eq!(cfg.modules[0].id, "my-module");
    assert_eq!(cfg.modules[0].paths, vec!["crates/my/**"]);
    assert_eq!(annotated.get_source("modules"), Some(ConfigSource::Local));
}

#[test]
fn test_schema_violation_in_project_unknown_key() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "MyProj"
nonexistent_key = "boom"
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let err = load_config(root).expect_err("Should fail with schema violation");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);

    let msg = err.message();
    assert!(
        msg.contains("nonexistent_key"),
        "Error message must name offending key: {}",
        msg
    );
    assert!(
        msg.contains("qdev.toml"),
        "Error message must name offending file qdev.toml: {}",
        msg
    );
}

#[test]
fn test_schema_violation_in_project_invalid_type() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = 12345
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let err = load_config(root).expect_err("Should fail with schema violation");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);

    let msg = err.message();
    assert!(
        msg.contains("name"),
        "Error message must name offending key: {}",
        msg
    );
    assert!(
        msg.contains("qdev.toml"),
        "Error message must name offending file qdev.toml: {}",
        msg
    );
}

#[test]
fn test_schema_violation_in_local_unknown_key() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "ValidProj"
"#;

    let local_toml = r#"
[identity]
developer_id = "simon"
unknown_field = 42
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let err = load_config(root).expect_err("Should fail with schema violation");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);

    let msg = err.message();
    assert!(
        msg.contains("unknown_field"),
        "Error message must name offending key: {}",
        msg
    );
    assert!(
        msg.contains(".qdev.local.toml"),
        "Error message must name offending file .qdev.local.toml: {}",
        msg
    );
}

#[test]
fn test_schema_violation_in_local_invalid_type() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let local_toml = r#"
[preferences]
color = "not-a-bool"
"#;

    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let err = load_config(root).expect_err("Should fail with schema violation");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);

    let msg = err.message();
    assert!(
        msg.contains("color"),
        "Error message must name offending key: {}",
        msg
    );
    assert!(
        msg.contains(".qdev.local.toml"),
        "Error message must name offending file .qdev.local.toml: {}",
        msg
    );
}

#[test]
fn test_all_14_sections_parsed_into_typed_structs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let toml_content = r#"
[project]
name = "CompleteProject"
default_sprint = 5

[teams]
core = ["alice", "bob"]

[git]
remote = "origin"
integration_branch = "develop"
branching_mode = "story-branch"
branch_template = "feat/{story_id}"
require_clean_tree_in_scope = true
max_integration_staleness_commits = 15

[storage]
specs_dir = "specs"
state_dir = "state"
cache_dir = "cache"

[[modules]]
id = "mod1"
paths = ["crates/mod1/**"]
layer = 1
may_depend_on = []

[hygiene]
enabled = true
max_inline_comment_lines = 4
forbid_patterns = ["TODO"]
citation_pattern = "\\[E\\d+\\]"
languages = ["rust"]

[regulatory]
iec62304_class = "ClassB"
require_rationale_for = ["unacceptable"]

[soup]
audit_command = "cargo audit"
deny_command = "cargo deny"
sbom_command = "cargo cyclonedx"

[models]
specify = "gemini-pro"
develop = "claude-code"
review = "gpt-4o"

[commit_messages]
enabled = true
format = "conventional"

[environment]
TEST_VAR = "hello"

[[gates]]
id = "fmt"
command = "cargo fmt"
timeout_ms = 5000

[identity]
developer_id = "test_dev"
teams = ["core"]

[preferences]
color = true
default_format = "json"
editor = "vim"
"#;

    fs::write(root.join("qdev.toml"), toml_content).unwrap();

    let annotated = load_config(root).expect("Should load full valid configuration");
    let cfg = &annotated.config;

    assert_eq!(cfg.project.name, "CompleteProject");
    assert_eq!(cfg.project.default_sprint, Some(5));
    assert_eq!(
        cfg.teams.get("core"),
        Some(&vec!["alice".to_string(), "bob".to_string()])
    );
    assert_eq!(cfg.git.branch_template, "feat/{story_id}");
    assert_eq!(cfg.storage.specs_dir, "specs");
    assert_eq!(cfg.modules.len(), 1);
    assert_eq!(cfg.hygiene.max_inline_comment_lines, 4);
    assert_eq!(cfg.regulatory.iec62304_class.as_deref(), Some("ClassB"));
    assert_eq!(cfg.soup.audit_command.as_deref(), Some("cargo audit"));
    assert_eq!(cfg.models.develop.as_deref(), Some("claude-code"));
    assert!(cfg.commit_messages.enabled);
    assert_eq!(
        cfg.environment.get("TEST_VAR").map(|s| s.as_str()),
        Some("hello")
    );
    assert_eq!(cfg.gates.len(), 1);
    assert_eq!(cfg.identity.developer_id, "test_dev");
    assert_eq!(cfg.preferences.default_format, "json");
    assert_eq!(cfg.preferences.editor.as_deref(), Some("vim"));
}

#[test]
fn test_teams_and_environment_key_by_key_merge() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[teams]
team-a = ["alice"]
team-b = ["bob"]

[environment]
VAR_A = "from_project"
VAR_B = "from_project"
"#;

    let local_toml = r#"
[teams]
team-b = ["bob_local"]
team-c = ["carol"]

[environment]
VAR_B = "from_local"
VAR_C = "from_local"
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).unwrap();
    let cfg = &annotated.config;

    assert_eq!(cfg.teams.get("team-a"), Some(&vec!["alice".to_string()]));
    assert_eq!(
        annotated.get_source("teams.team-a"),
        Some(ConfigSource::Project)
    );

    assert_eq!(
        cfg.teams.get("team-b"),
        Some(&vec!["bob_local".to_string()])
    );
    assert_eq!(
        annotated.get_source("teams.team-b"),
        Some(ConfigSource::Local)
    );

    assert_eq!(cfg.teams.get("team-c"), Some(&vec!["carol".to_string()]));
    assert_eq!(
        annotated.get_source("teams.team-c"),
        Some(ConfigSource::Local)
    );

    assert_eq!(
        cfg.environment.get("VAR_A").map(|s| s.as_str()),
        Some("from_project")
    );
    assert_eq!(
        annotated.get_source("environment.VAR_A"),
        Some(ConfigSource::Project)
    );

    assert_eq!(
        cfg.environment.get("VAR_B").map(|s| s.as_str()),
        Some("from_local")
    );
    assert_eq!(
        annotated.get_source("environment.VAR_B"),
        Some(ConfigSource::Local)
    );

    assert_eq!(
        cfg.environment.get("VAR_C").map(|s| s.as_str()),
        Some("from_local")
    );
    assert_eq!(
        annotated.get_source("environment.VAR_C"),
        Some(ConfigSource::Local)
    );
}

#[test]
fn test_local_gates_table_with_skip() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[gates]]
id = "fmt"
command = "cargo fmt --all --check"

[[gates]]
id = "warning-count"
command = "check-warnings.sh"
"#;

    let local_toml = r#"
[gates]
skip = ["warning-count"]
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).unwrap();
    let cfg = &annotated.config;

    assert_eq!(cfg.gates.len(), 2);
    let fmt_gate = cfg.gates.iter().find(|g| g.id == "fmt").unwrap();
    assert_eq!(fmt_gate.skip, None);

    let warning_gate = cfg.gates.iter().find(|g| g.id == "warning-count").unwrap();
    assert_eq!(warning_gate.skip, Some(true));
}

#[test]
fn test_local_gates_boolean_skip_all() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[gates]]
id = "fmt"
command = "cargo fmt --all --check"

[[gates]]
id = "warning-count"
command = "check-warnings.sh"
"#;

    let local_toml = r#"
[gates]
skip = true
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).unwrap();
    let cfg = &annotated.config;

    assert_eq!(cfg.gates.len(), 2);
    let fmt_gate = cfg.gates.iter().find(|g| g.id == "fmt").unwrap();
    assert_eq!(fmt_gate.skip, Some(true));

    let warning_gate = cfg.gates.iter().find(|g| g.id == "warning-count").unwrap();
    assert_eq!(warning_gate.skip, Some(true));
}

#[test]
fn test_gates_table_in_project_toml_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[gates]
skip = true
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let err = load_config(root).expect_err("Should reject [gates] table in qdev.toml");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);
    let msg = err.message();
    assert!(msg.contains("gates"), "Error should mention gates: {}", msg);
    assert!(
        msg.contains("qdev.toml"),
        "Error should mention qdev.toml: {}",
        msg
    );
}

#[test]
fn test_module_missing_required_id_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[modules]]
paths = ["crates/**"]
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let err = load_config(root).expect_err("Should reject module without id");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);
    let msg = err.message();
    assert!(
        msg.contains("id"),
        "Error should mention missing id: {}",
        msg
    );
    assert!(
        msg.contains("qdev.toml"),
        "Error should mention qdev.toml: {}",
        msg
    );
}

#[test]
fn test_module_missing_required_paths_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[modules]]
id = "core"
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let err = load_config(root).expect_err("Should reject module without paths");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);
    let msg = err.message();
    assert!(
        msg.contains("paths"),
        "Error should mention missing paths: {}",
        msg
    );
    assert!(
        msg.contains("qdev.toml"),
        "Error should mention qdev.toml: {}",
        msg
    );
}

#[test]
fn test_gate_missing_required_id_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[gates]]
command = "cargo check"
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let err = load_config(root).expect_err("Should reject gate without id");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.exit_code().as_i32(), 2);
    let msg = err.message();
    assert!(
        msg.contains("id"),
        "Error should mention missing id: {}",
        msg
    );
    assert!(
        msg.contains("qdev.toml"),
        "Error should mention qdev.toml: {}",
        msg
    );
}

#[test]
fn test_negative_integers_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. project.default_sprint
    fs::write(root.join("qdev.toml"), "[project]\ndefault_sprint = -1\n").unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("default_sprint"));

    // 2. git.max_integration_staleness_commits
    fs::write(
        root.join("qdev.toml"),
        "[git]\nmax_integration_staleness_commits = -5\n",
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("max_integration_staleness_commits"));

    // 3. modules.layer
    fs::write(
        root.join("qdev.toml"),
        "[[modules]]\nid = \"m\"\npaths = [\"*\"]\nlayer = -2\n",
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("layer"));

    // 4. hygiene.max_inline_comment_lines
    fs::write(
        root.join("qdev.toml"),
        "[hygiene]\nmax_inline_comment_lines = -3\n",
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("max_inline_comment_lines"));

    // 5. gates.timeout_ms
    fs::write(
        root.join("qdev.toml"),
        "[[gates]]\nid = \"g\"\ntimeout_ms = -100\n",
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("timeout_ms"));
}

#[test]
fn test_workspace_root_discovery_from_subdirectory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[project]
name = "RootProject"
"#;
    fs::write(root.join("qdev.toml"), project_toml).unwrap();

    let deep_dir = root.join("crates").join("subpkg").join("src");
    fs::create_dir_all(&deep_dir).unwrap();

    let annotated =
        load_config(&deep_dir).expect("Should discover root config from deep subdirectory");
    assert_eq!(annotated.config.project.name, "RootProject");
}

#[test]
fn test_default_identity_fallback_without_git() {
    let annotated = qdev_core::config::merge_configs(None, None, None).unwrap();
    assert_eq!(annotated.config.identity.developer_id, "");
    assert_eq!(
        annotated.get_source("identity.developer_id"),
        Some(ConfigSource::Default)
    );
}

#[test]
fn test_integer_overflow_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // 1. default_sprint > u32::MAX
    let large_int = (u32::MAX as u64) + 10;
    fs::write(
        root.join("qdev.toml"),
        format!("[project]\ndefault_sprint = {}\n", large_int),
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("default_sprint"));

    // 2. git.max_integration_staleness_commits > u32::MAX
    fs::write(
        root.join("qdev.toml"),
        format!("[git]\nmax_integration_staleness_commits = {}\n", large_int),
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("max_integration_staleness_commits"));

    // 3. modules.layer > u32::MAX
    fs::write(
        root.join("qdev.toml"),
        format!(
            "[[modules]]\nid = \"m\"\npaths = [\"*\"]\nlayer = {}\n",
            large_int
        ),
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("layer"));

    // 4. hygiene.max_inline_comment_lines > u32::MAX
    fs::write(
        root.join("qdev.toml"),
        format!("[hygiene]\nmax_inline_comment_lines = {}\n", large_int),
    )
    .unwrap();
    let err = load_config(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("max_inline_comment_lines"));
}

#[test]
fn test_to_text_report_comprehensive() {
    let toml_content = r#"
[project]
name = "FullTest"
default_sprint = 2

[git]
remote = "upstream"
integration_branch = "main"

[storage]
specs_dir = "specs"

[[modules]]
id = "core"
paths = ["crates/core/**"]
layer = 1
may_depend_on = ["utils"]

[[gates]]
id = "test-gate"
command = "cargo test"
timeout_ms = 5000

[hygiene]
enabled = true

[preferences]
color = true
editor = "vim"
"#;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("qdev.toml"), toml_content).unwrap();

    let annotated = load_config(root).unwrap();
    let report = annotated.to_text_report();

    assert!(report.contains("[project]"));
    assert!(report.contains("FullTest"));
    assert!(report.contains("[git]"));
    assert!(report.contains("upstream"));
    assert!(report.contains("[storage]"));
    assert!(report.contains("[[modules]]"));
    assert!(report.contains("layer = 1"));
    assert!(report.contains("may_depend_on = [\"utils\"]"));
    assert!(report.contains("[[gates]]"));
    assert!(report.contains("timeout_ms = 5000"));
    assert!(report.contains("[hygiene]"));
    assert!(report.contains("[preferences]"));
}

#[test]
fn test_local_gates_table_with_skip_false() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let project_toml = r#"
[[gates]]
id = "gate1"
command = "echo 1"
skip = true
"#;

    let local_toml = r#"
[gates]
skip = false
"#;

    fs::write(root.join("qdev.toml"), project_toml).unwrap();
    fs::write(root.join(".qdev.local.toml"), local_toml).unwrap();

    let annotated = load_config(root).unwrap();
    assert_eq!(annotated.config.gates.len(), 1);
    assert_eq!(annotated.config.gates[0].skip, Some(false));
}

/// Option C: the cache is a machine-local, rebuildable artifact, so `.qdev.local.toml` may
/// relocate it. It overrides the project value key by key like every other scalar, and `config
/// show` attributes it to the local file.
#[test]
fn test_local_storage_may_override_cache_dir_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(
        root.join("qdev.toml"),
        "[storage]\nspecs_dir = \"planning/specs\"\nstate_dir = \"planning/state\"\ncache_dir = \".qdev/cache\"\n",
    )
    .unwrap();
    fs::write(
        root.join(".qdev.local.toml"),
        "[storage]\ncache_dir = \"local/cache\"\n",
    )
    .unwrap();

    let annotated = load_config(root).expect("a local cache_dir is legal");
    assert_eq!(annotated.config.storage.cache_dir, "local/cache");
    assert_eq!(annotated.config.storage.specs_dir, "planning/specs");
    assert_eq!(annotated.config.storage.state_dir, "planning/state");
    assert_eq!(
        annotated.sources.get("storage.cache_dir"),
        Some(&ConfigSource::Local)
    );

    // The project layout is the same answer with the local table withheld — what the committed
    // `.gitignore` must keep covering.
    let project = qdev_core::load_project_storage(root).unwrap();
    assert_eq!(project.cache_dir, ".qdev/cache");
    assert_eq!(project.specs_dir, "planning/specs");
}

/// `specs_dir` and `state_dir` hold committed content, so they are a project decision: either key
/// in the local file is a schema error naming the key and the file, from every command.
#[test]
fn test_local_storage_rejects_specs_dir_and_state_dir() {
    for key in ["specs_dir", "state_dir"] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("qdev.toml"), "[project]\nname = \"Demo\"\n").unwrap();
        fs::write(
            root.join(".qdev.local.toml"),
            format!("[storage]\n{} = \"elsewhere\"\n", key),
        )
        .unwrap();

        let err = load_config(root).expect_err("a project-only storage key must be refused");
        assert_eq!(err.exit_code(), ExitCode::UsageError);
        assert!(
            err.message().contains(key) && err.message().contains(".qdev.local.toml"),
            "the error must name the key and the file, got: {}",
            err.message()
        );
        assert_eq!(err.details().unwrap()["key"], format!("storage.{}", key));
        assert_eq!(err.details().unwrap()["file"], ".qdev.local.toml");

        // The same keys remain legal in the committed file.
        fs::write(
            root.join(".qdev.local.toml"),
            "[storage]\ncache_dir = \"local/cache\"\n",
        )
        .unwrap();
        fs::write(
            root.join("qdev.toml"),
            format!("[storage]\n{} = \"elsewhere\"\n", key),
        )
        .unwrap();
        load_config(root).expect("the layout is configurable in qdev.toml");
    }
}

/// A `[storage]` value names a directory *inside* the workspace. The private reader `init` used
/// to have skipped an empty value and fell back to the default; deleting it removed that guard
/// with nothing in its place, and the reviewers demonstrated the consequences against the built
/// binary: `cache_dir = ""` made `root.join()` yield the absolute `/cache.sqlite` and wrote a
/// bare `/` into the committed `.gitignore`, and `specs_dir = ""` tried to create `/prd`.
#[test]
fn test_storage_paths_must_be_relative_non_empty_and_inside_the_workspace() {
    for key in ["specs_dir", "state_dir", "cache_dir"] {
        for (value, expected) in [
            ("", "is empty"),
            ("   ", "is empty"),
            ("/absolute/path", "is absolute"),
            ("../escape", "contains '..'"),
            ("nested/../..", "contains '..'"),
            // The same shapes spelled the Windows way. `qdev.toml` is committed and shared, so
            // every one of these is refused on every platform: a value that loads on Linux and
            // fails on Windows is a workspace that works for one developer and not the next.
            // `\\absolute\\path` is what `/absolute/path` looks like to Windows — and neither is
            // "absolute" there, since a drive prefix is what makes a Windows path absolute, which
            // is why asking `Path::is_absolute` let this through.
            ("\\\\absolute\\\\path", "is absolute"),
            ("C:\\\\x", "is absolute"),
            ("C:x", "is absolute"),
            ("nested\\\\..\\\\..", "contains '..'"),
        ] {
            let temp = TempDir::new().unwrap();
            let root = temp.path();
            fs::write(
                root.join("qdev.toml"),
                format!("[project]\nname = \"Demo\"\n\n[storage]\n{key} = \"{value}\"\n"),
            )
            .unwrap();

            let err = load_config(root)
                .expect_err("a storage value outside the workspace must be refused");
            assert_eq!(err.exit_code(), ExitCode::UsageError);
            assert!(
                err.message().contains(key) && err.message().contains(expected),
                "the error must name the key and why, got: {}",
                err.message()
            );
            assert_eq!(err.details().unwrap()["key"], format!("storage.{key}"));
        }
    }
}

/// The loader normalizes a layout value once, so `init` and every other command join the same
/// string. While `init` trimmed privately and the loader did not, a padded `cache_dir` produced
/// two databases — the two-caches defect in a different disguise.
#[test]
fn test_storage_paths_are_normalized_by_the_loader() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("qdev.toml"),
        "[project]\nname = \"Demo\"\n\n[storage]\nspecs_dir = \"  planning/specs/  \"\ncache_dir = \" var/cache \"\n",
    )
    .unwrap();

    let cfg = load_config(root).expect("a padded value is legal, just normalized");
    assert_eq!(cfg.config.storage.specs_dir, "planning/specs");
    assert_eq!(cfg.config.storage.cache_dir, "var/cache");
}
