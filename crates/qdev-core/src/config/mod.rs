pub mod source;
pub mod types;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub use source::{AnnotatedConfig, AnnotatedValue, ConfigSource};
pub use types::{
    CommitMessagesConfig, Config, EnvironmentConfig, GateConfig, GitConfig, HygieneConfig,
    IdentityConfig, ModelsConfig, ModuleConfig, PreferencesConfig, ProjectConfig, RegulatoryConfig,
    SoupConfig, StorageConfig, TeamsConfig, DEFAULT_CITATION_PATTERN,
};

use crate::errors::QdevError;

const ALLOWED_TOP_LEVEL_SECTIONS: &[&str] = &[
    "project",
    "teams",
    "git",
    "storage",
    "modules",
    "hygiene",
    "regulatory",
    "soup",
    "models",
    "commit_messages",
    "environment",
    "gates",
    "identity",
    "preferences",
];

/// Validates a parsed TOML table against the strict configuration schema.
/// Returns exit code 2 (usage error) naming key and file if invalid per spec.
pub fn validate_config_table(table: &toml::Table, filename: &str) -> Result<(), QdevError> {
    for (key, val) in table {
        if !ALLOWED_TOP_LEVEL_SECTIONS.contains(&key.as_str()) {
            return Err(QdevError::usage_error(format!(
                "Schema violation in {}: unknown key '{}'",
                filename, key
            ))
            .with_details(serde_json::json!({
                "file": filename,
                "key": key,
            })));
        }

        match key.as_str() {
            "project" => validate_project_section(val, filename)?,
            "teams" => validate_teams_section(val, filename)?,
            "git" => validate_git_section(val, filename)?,
            "storage" => validate_storage_section(val, filename)?,
            "modules" => validate_modules_section(val, filename)?,
            "hygiene" => validate_hygiene_section(val, filename)?,
            "regulatory" => validate_regulatory_section(val, filename)?,
            "soup" => validate_soup_section(val, filename)?,
            "models" => validate_models_section(val, filename)?,
            "commit_messages" => validate_commit_messages_section(val, filename)?,
            "environment" => validate_environment_section(val, filename)?,
            "gates" => validate_gates_section(val, filename)?,
            "identity" => validate_identity_section(val, filename)?,
            "preferences" => validate_preferences_section(val, filename)?,
            _ => unreachable!(),
        }
    }

    Ok(())
}

fn validate_project_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "project", filename)?;
    let allowed = &["name", "default_sprint"];
    check_unknown_keys(table, allowed, "project", filename)?;

    if let Some(v) = table.get("name") {
        if !v.is_str() {
            return Err(type_mismatch_error("name", "project", "string", filename));
        }
    }
    if let Some(v) = table.get("default_sprint") {
        if let Some(i) = v.as_integer() {
            if i < 0 || i > u32::MAX as i64 {
                return Err(QdevError::usage_error(format!(
                    "Schema violation in {}: key 'default_sprint' in [project] must be a valid u32 integer",
                    filename
                ))
                .with_details(serde_json::json!({
                    "file": filename,
                    "key": "project.default_sprint",
                })));
            }
        } else {
            return Err(type_mismatch_error(
                "default_sprint",
                "project",
                "integer",
                filename,
            ));
        }
    }
    Ok(())
}

fn validate_teams_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "teams", filename)?;
    for (team_name, members) in table {
        if let Some(arr) = members.as_array() {
            for item in arr {
                if !item.is_str() {
                    return Err(type_mismatch_error(
                        team_name,
                        "teams",
                        "array of strings",
                        filename,
                    ));
                }
            }
        } else {
            return Err(type_mismatch_error(
                team_name,
                "teams",
                "array of strings",
                filename,
            ));
        }
    }
    Ok(())
}

fn validate_git_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "git", filename)?;
    let allowed = &[
        "remote",
        "integration_branch",
        "branching_mode",
        "branch_template",
        "require_clean_tree_in_scope",
        "max_integration_staleness_commits",
    ];
    check_unknown_keys(table, allowed, "git", filename)?;

    if let Some(v) = table.get("remote") {
        if !v.is_str() {
            return Err(type_mismatch_error("remote", "git", "string", filename));
        }
    }
    if let Some(v) = table.get("integration_branch") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "integration_branch",
                "git",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("branching_mode") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "branching_mode",
                "git",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("branch_template") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "branch_template",
                "git",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("require_clean_tree_in_scope") {
        if !v.is_bool() {
            return Err(type_mismatch_error(
                "require_clean_tree_in_scope",
                "git",
                "boolean",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("max_integration_staleness_commits") {
        if let Some(i) = v.as_integer() {
            if i < 0 || i > u32::MAX as i64 {
                return Err(QdevError::usage_error(format!(
                    "Schema violation in {}: key 'max_integration_staleness_commits' in [git] must be a valid u32 integer",
                    filename
                ))
                .with_details(serde_json::json!({
                    "file": filename,
                    "key": "git.max_integration_staleness_commits",
                })));
            }
        } else {
            return Err(type_mismatch_error(
                "max_integration_staleness_commits",
                "git",
                "integer",
                filename,
            ));
        }
    }
    Ok(())
}

fn validate_storage_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "storage", filename)?;
    let allowed = &["specs_dir", "state_dir", "cache_dir"];
    check_unknown_keys(table, allowed, "storage", filename)?;

    for key in allowed {
        if let Some(v) = table.get(*key) {
            if !v.is_str() {
                return Err(type_mismatch_error(key, "storage", "string", filename));
            }
        }
    }
    Ok(())
}

fn validate_modules_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let arr = match val.as_array() {
        Some(a) => a,
        None => {
            return Err(QdevError::usage_error(format!(
                "Schema violation in {}: invalid type for key 'modules', expected array of tables",
                filename
            ))
            .with_details(serde_json::json!({
                "file": filename,
                "key": "modules",
            })));
        }
    };

    let allowed = &["id", "paths", "layer", "may_depend_on"];
    for item in arr {
        let table = match item.as_table() {
            Some(t) => t,
            None => {
                return Err(QdevError::usage_error(format!(
                    "Schema violation in {}: invalid type for module item, expected table",
                    filename
                ))
                .with_details(serde_json::json!({
                    "file": filename,
                    "key": "modules",
                })));
            }
        };

        check_unknown_keys(table, allowed, "modules", filename)?;

        if let Some(v) = table.get("id") {
            if !v.is_str() {
                return Err(type_mismatch_error("id", "modules", "string", filename));
            }
        } else {
            return Err(QdevError::usage_error(format!(
                "Schema violation in {}: missing required key 'id' in [[modules]]",
                filename
            ))
            .with_details(serde_json::json!({
                "file": filename,
                "key": "modules.id",
            })));
        }

        if let Some(v) = table.get("paths") {
            validate_string_array(v, "paths", "modules", filename)?;
        } else {
            return Err(QdevError::usage_error(format!(
                "Schema violation in {}: missing required key 'paths' in [[modules]]",
                filename
            ))
            .with_details(serde_json::json!({
                "file": filename,
                "key": "modules.paths",
            })));
        }

        if let Some(v) = table.get("layer") {
            if let Some(i) = v.as_integer() {
                if i < 0 || i > u32::MAX as i64 {
                    return Err(QdevError::usage_error(format!(
                        "Schema violation in {}: key 'layer' in [[modules]] must be a valid u32 integer",
                        filename
                    ))
                    .with_details(serde_json::json!({
                        "file": filename,
                        "key": "modules.layer",
                    })));
                }
            } else {
                return Err(type_mismatch_error("layer", "modules", "integer", filename));
            }
        }
        if let Some(v) = table.get("may_depend_on") {
            validate_string_array(v, "may_depend_on", "modules", filename)?;
        }
    }
    Ok(())
}

fn validate_hygiene_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "hygiene", filename)?;
    let allowed = &[
        "enabled",
        "max_inline_comment_lines",
        "forbid_patterns",
        "citation_pattern",
        "languages",
    ];
    check_unknown_keys(table, allowed, "hygiene", filename)?;

    if let Some(v) = table.get("enabled") {
        if !v.is_bool() {
            return Err(type_mismatch_error(
                "enabled", "hygiene", "boolean", filename,
            ));
        }
    }
    if let Some(v) = table.get("max_inline_comment_lines") {
        if let Some(i) = v.as_integer() {
            if i < 0 || i > u32::MAX as i64 {
                return Err(QdevError::usage_error(format!(
                    "Schema violation in {}: key 'max_inline_comment_lines' in [hygiene] must be a valid u32 integer",
                    filename
                ))
                .with_details(serde_json::json!({
                    "file": filename,
                    "key": "hygiene.max_inline_comment_lines",
                })));
            }
        } else {
            return Err(type_mismatch_error(
                "max_inline_comment_lines",
                "hygiene",
                "integer",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("forbid_patterns") {
        validate_string_array(v, "forbid_patterns", "hygiene", filename)?;
    }
    if let Some(v) = table.get("citation_pattern") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "citation_pattern",
                "hygiene",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("languages") {
        validate_string_array(v, "languages", "hygiene", filename)?;
    }
    Ok(())
}

fn validate_regulatory_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "regulatory", filename)?;
    let allowed = &["iec62304_class", "require_rationale_for"];
    check_unknown_keys(table, allowed, "regulatory", filename)?;

    if let Some(v) = table.get("iec62304_class") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "iec62304_class",
                "regulatory",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("require_rationale_for") {
        validate_string_array(v, "require_rationale_for", "regulatory", filename)?;
    }
    Ok(())
}

fn validate_soup_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "soup", filename)?;
    let allowed = &["audit_command", "deny_command", "sbom_command"];
    check_unknown_keys(table, allowed, "soup", filename)?;

    for key in allowed {
        if let Some(v) = table.get(*key) {
            if !v.is_str() {
                return Err(type_mismatch_error(key, "soup", "string", filename));
            }
        }
    }
    Ok(())
}

fn validate_models_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "models", filename)?;
    let allowed = &["specify", "develop", "review"];
    check_unknown_keys(table, allowed, "models", filename)?;

    for key in allowed {
        if let Some(v) = table.get(*key) {
            if !v.is_str() {
                return Err(type_mismatch_error(key, "models", "string", filename));
            }
        }
    }
    Ok(())
}

fn validate_commit_messages_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "commit_messages", filename)?;
    let allowed = &["enabled", "format"];
    check_unknown_keys(table, allowed, "commit_messages", filename)?;

    if let Some(v) = table.get("enabled") {
        if !v.is_bool() {
            return Err(type_mismatch_error(
                "enabled",
                "commit_messages",
                "boolean",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("format") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "format",
                "commit_messages",
                "string",
                filename,
            ));
        }
    }
    Ok(())
}

fn validate_environment_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "environment", filename)?;
    for (k, v) in table {
        if !v.is_str() {
            return Err(type_mismatch_error(k, "environment", "string", filename));
        }
    }
    Ok(())
}

fn validate_gates_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    if let Some(arr) = val.as_array() {
        let allowed = &[
            "id",
            "command",
            "timeout_ms",
            "depends_on",
            "output_adapter",
            "on_transition",
            "verifies",
            "kind",
            "metric",
            "direction",
            "skip",
        ];
        for item in arr {
            let table = match item.as_table() {
                Some(t) => t,
                None => {
                    return Err(QdevError::usage_error(format!(
                        "Schema violation in {}: invalid type for gate item, expected table",
                        filename
                    ))
                    .with_details(serde_json::json!({
                        "file": filename,
                        "key": "gates",
                    })));
                }
            };

            check_unknown_keys(table, allowed, "gates", filename)?;

            if let Some(v) = table.get("id") {
                if !v.is_str() {
                    return Err(type_mismatch_error("id", "gates", "string", filename));
                }
            } else {
                return Err(QdevError::usage_error(format!(
                    "Schema violation in {}: missing required key 'id' in [[gates]]",
                    filename
                ))
                .with_details(serde_json::json!({
                    "file": filename,
                    "key": "gates.id",
                })));
            }
            if let Some(v) = table.get("command") {
                if !v.is_str() {
                    return Err(type_mismatch_error("command", "gates", "string", filename));
                }
            }
            if let Some(v) = table.get("timeout_ms") {
                if let Some(i) = v.as_integer() {
                    if i < 0 {
                        return Err(QdevError::usage_error(format!(
                            "Schema violation in {}: key 'timeout_ms' in [[gates]] must be non-negative",
                            filename
                        ))
                        .with_details(serde_json::json!({
                            "file": filename,
                            "key": "gates.timeout_ms",
                        })));
                    }
                } else {
                    return Err(type_mismatch_error(
                        "timeout_ms",
                        "gates",
                        "integer",
                        filename,
                    ));
                }
            }
            if let Some(v) = table.get("depends_on") {
                validate_string_array(v, "depends_on", "gates", filename)?;
            }
            if let Some(v) = table.get("output_adapter") {
                if !v.is_str() {
                    return Err(type_mismatch_error(
                        "output_adapter",
                        "gates",
                        "string",
                        filename,
                    ));
                }
            }
            if let Some(v) = table.get("on_transition") {
                validate_string_array(v, "on_transition", "gates", filename)?;
            }
            if let Some(v) = table.get("verifies") {
                validate_string_array(v, "verifies", "gates", filename)?;
            }
            if let Some(v) = table.get("kind") {
                if !v.is_str() {
                    return Err(type_mismatch_error("kind", "gates", "string", filename));
                }
            }
            if let Some(v) = table.get("metric") {
                if !v.is_str() {
                    return Err(type_mismatch_error("metric", "gates", "string", filename));
                }
            }
            if let Some(v) = table.get("direction") {
                if !v.is_str() {
                    return Err(type_mismatch_error(
                        "direction",
                        "gates",
                        "string",
                        filename,
                    ));
                }
            }
            if let Some(v) = table.get("skip") {
                if !v.is_bool() {
                    return Err(type_mismatch_error("skip", "gates", "boolean", filename));
                }
            }
        }
    } else if let Some(table) = val.as_table() {
        if filename == "qdev.toml" {
            return Err(QdevError::usage_error(format!(
                "Schema violation in {}: key 'gates' must be an array of tables in qdev.toml",
                filename
            ))
            .with_details(serde_json::json!({
                "file": filename,
                "key": "gates",
            })));
        }
        let allowed = &["skip"];
        check_unknown_keys(table, allowed, "gates", filename)?;
        if let Some(v) = table.get("skip") {
            if !v.is_bool() && validate_string_array(v, "skip", "gates", filename).is_err() {
                return Err(type_mismatch_error(
                    "skip",
                    "gates",
                    "array of strings or boolean",
                    filename,
                ));
            }
        }
    } else {
        return Err(QdevError::usage_error(format!(
            "Schema violation in {}: invalid type for key 'gates', expected array of tables or table",
            filename
        ))
        .with_details(serde_json::json!({
            "file": filename,
            "key": "gates",
        })));
    }
    Ok(())
}

fn validate_identity_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "identity", filename)?;
    let allowed = &["developer_id", "teams"];
    check_unknown_keys(table, allowed, "identity", filename)?;

    if let Some(v) = table.get("developer_id") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "developer_id",
                "identity",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("teams") {
        validate_string_array(v, "teams", "identity", filename)?;
    }
    Ok(())
}

fn validate_preferences_section(val: &toml::Value, filename: &str) -> Result<(), QdevError> {
    let table = expect_table(val, "preferences", filename)?;
    let allowed = &["color", "default_format", "editor"];
    check_unknown_keys(table, allowed, "preferences", filename)?;

    if let Some(v) = table.get("color") {
        if !v.is_bool() {
            return Err(type_mismatch_error(
                "color",
                "preferences",
                "boolean",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("default_format") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "default_format",
                "preferences",
                "string",
                filename,
            ));
        }
    }
    if let Some(v) = table.get("editor") {
        if !v.is_str() {
            return Err(type_mismatch_error(
                "editor",
                "preferences",
                "string",
                filename,
            ));
        }
    }
    Ok(())
}

fn expect_table<'a>(
    val: &'a toml::Value,
    key: &str,
    filename: &str,
) -> Result<&'a toml::Table, QdevError> {
    match val.as_table() {
        Some(t) => Ok(t),
        None => Err(QdevError::usage_error(format!(
            "Schema violation in {}: invalid type for key '{}', expected table",
            filename, key
        ))
        .with_details(serde_json::json!({
            "file": filename,
            "key": key,
        }))),
    }
}

fn check_unknown_keys(
    table: &toml::Table,
    allowed: &[&str],
    section_name: &str,
    filename: &str,
) -> Result<(), QdevError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(QdevError::usage_error(format!(
                "Schema violation in {}: unknown key '{}' in [{}]",
                filename, key, section_name
            ))
            .with_details(serde_json::json!({
                "file": filename,
                "key": format!("{}.{}", section_name, key),
            })));
        }
    }
    Ok(())
}

fn validate_string_array(
    val: &toml::Value,
    key: &str,
    section: &str,
    filename: &str,
) -> Result<(), QdevError> {
    if let Some(arr) = val.as_array() {
        for item in arr {
            if !item.is_str() {
                return Err(type_mismatch_error(
                    key,
                    section,
                    "array of strings",
                    filename,
                ));
            }
        }
        Ok(())
    } else {
        Err(type_mismatch_error(
            key,
            section,
            "array of strings",
            filename,
        ))
    }
}

fn type_mismatch_error(key: &str, section: &str, expected: &str, filename: &str) -> QdevError {
    QdevError::usage_error(format!(
        "Schema violation in {}: invalid type for key '{}' in [{}], expected {}",
        filename, key, section, expected
    ))
    .with_details(serde_json::json!({
        "file": filename,
        "key": format!("{}.{}", section, key),
    }))
}

/// Fall back to `git config user.email` if identity is not locally defined.
pub fn resolve_git_email(repo_dir: Option<&Path>) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(["config", "user.email"]);
    if let Some(dir) = repo_dir {
        cmd.current_dir(dir);
    }
    let output = cmd.output().ok()?;
    if output.status.success() {
        let email = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !email.is_empty() {
            return Some(email);
        }
    }
    None
}

/// Merges project table with local table and git email fallback.
pub fn merge_configs(
    project_table: Option<&toml::Table>,
    local_table: Option<&toml::Table>,
    git_email: Option<&str>,
) -> Result<AnnotatedConfig, QdevError> {
    let mut config = Config::default();
    let mut sources = BTreeMap::new();

    // Helper closures
    let get_val = |sec: &str, key: &str| -> (Option<toml::Value>, ConfigSource) {
        if let Some(loc_val) = local_table
            .and_then(|t| t.get(sec))
            .and_then(|s| s.get(key))
        {
            (Some(loc_val.clone()), ConfigSource::Local)
        } else if let Some(proj_val) = project_table
            .and_then(|t| t.get(sec))
            .and_then(|s| s.get(key))
        {
            (Some(proj_val.clone()), ConfigSource::Project)
        } else {
            (None, ConfigSource::Default)
        }
    };

    let section_source = |sec: &str| -> ConfigSource {
        if local_table.and_then(|t| t.get(sec)).is_some() {
            ConfigSource::Local
        } else if project_table.and_then(|t| t.get(sec)).is_some() {
            ConfigSource::Project
        } else {
            ConfigSource::Default
        }
    };

    // 1. [project]
    sources.insert("project".to_string(), section_source("project"));
    let (name_val, src) = get_val("project", "name");
    sources.insert("project.name".to_string(), src);
    if let Some(v) = name_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.project.name = v;
    }

    let (sprint_val, src) = get_val("project", "default_sprint");
    sources.insert("project.default_sprint".to_string(), src);
    if let Some(v) = sprint_val.and_then(|v| v.as_integer().map(|i| i as u32)) {
        config.project.default_sprint = Some(v);
    }

    // 2. [teams]
    sources.insert("teams".to_string(), section_source("teams"));
    let mut merged_teams = BTreeMap::new();
    // Start with local teams
    if let Some(loc_teams) = local_table
        .and_then(|t| t.get("teams"))
        .and_then(|v| v.as_table())
    {
        for (team, members) in loc_teams {
            if let Some(arr) = members.as_array() {
                let member_strs: Vec<String> = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();
                merged_teams.insert(team.clone(), member_strs);
                sources.insert(format!("teams.{}", team), ConfigSource::Local);
            }
        }
    }
    // Add project teams if not overridden
    if let Some(proj_teams) = project_table
        .and_then(|t| t.get("teams"))
        .and_then(|v| v.as_table())
    {
        for (team, members) in proj_teams {
            if !merged_teams.contains_key(team) {
                if let Some(arr) = members.as_array() {
                    let member_strs: Vec<String> = arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    merged_teams.insert(team.clone(), member_strs);
                    sources.insert(format!("teams.{}", team), ConfigSource::Project);
                }
            }
        }
    }
    config.teams = TeamsConfig::new(merged_teams);

    // 3. [git]
    sources.insert("git".to_string(), section_source("git"));
    let (rem_val, src) = get_val("git", "remote");
    sources.insert("git.remote".to_string(), src);
    if let Some(v) = rem_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.git.remote = v;
    }

    let (branch_val, src) = get_val("git", "integration_branch");
    sources.insert("git.integration_branch".to_string(), src);
    if let Some(v) = branch_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.git.integration_branch = v;
    }

    let (mode_val, src) = get_val("git", "branching_mode");
    sources.insert("git.branching_mode".to_string(), src);
    if let Some(v) = mode_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.git.branching_mode = v;
    }

    let (tmpl_val, src) = get_val("git", "branch_template");
    sources.insert("git.branch_template".to_string(), src);
    if let Some(v) = tmpl_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.git.branch_template = v;
    }

    let (clean_val, src) = get_val("git", "require_clean_tree_in_scope");
    sources.insert("git.require_clean_tree_in_scope".to_string(), src);
    if let Some(v) = clean_val.and_then(|v| v.as_bool()) {
        config.git.require_clean_tree_in_scope = v;
    }

    let (stale_val, src) = get_val("git", "max_integration_staleness_commits");
    sources.insert("git.max_integration_staleness_commits".to_string(), src);
    if let Some(v) = stale_val.and_then(|v| v.as_integer().map(|i| i as u32)) {
        config.git.max_integration_staleness_commits = v;
    }

    // 4. [storage]
    sources.insert("storage".to_string(), section_source("storage"));
    let (specs_val, src) = get_val("storage", "specs_dir");
    sources.insert("storage.specs_dir".to_string(), src);
    if let Some(v) = specs_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.storage.specs_dir = v;
    }

    let (state_val, src) = get_val("storage", "state_dir");
    sources.insert("storage.state_dir".to_string(), src);
    if let Some(v) = state_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.storage.state_dir = v;
    }

    let (cache_val, src) = get_val("storage", "cache_dir");
    sources.insert("storage.cache_dir".to_string(), src);
    if let Some(v) = cache_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.storage.cache_dir = v;
    }

    // 5. [[modules]] - WHOLESALE ARRAY REPLACEMENT PER SPEC
    if let Some(loc_mods) = local_table.and_then(|t| t.get("modules")) {
        sources.insert("modules".to_string(), ConfigSource::Local);
        config.modules = loc_mods.clone().try_into().map_err(|e| {
            QdevError::usage_error(format!("Failed to deserialize local modules: {}", e))
        })?;
    } else if let Some(proj_mods) = project_table.and_then(|t| t.get("modules")) {
        sources.insert("modules".to_string(), ConfigSource::Project);
        config.modules = proj_mods.clone().try_into().map_err(|e| {
            QdevError::usage_error(format!("Failed to deserialize project modules: {}", e))
        })?;
    } else {
        sources.insert("modules".to_string(), ConfigSource::Default);
        config.modules = Vec::new();
    }

    // 6. [hygiene]
    sources.insert("hygiene".to_string(), section_source("hygiene"));
    let (enabled_val, src) = get_val("hygiene", "enabled");
    sources.insert("hygiene.enabled".to_string(), src);
    if let Some(v) = enabled_val.and_then(|v| v.as_bool()) {
        config.hygiene.enabled = v;
    }

    let (lines_val, src) = get_val("hygiene", "max_inline_comment_lines");
    sources.insert("hygiene.max_inline_comment_lines".to_string(), src);
    if let Some(v) = lines_val.and_then(|v| v.as_integer().map(|i| i as u32)) {
        config.hygiene.max_inline_comment_lines = v;
    }

    let (pat_val, src) = get_val("hygiene", "forbid_patterns");
    sources.insert("hygiene.forbid_patterns".to_string(), src);
    if let Some(v) = pat_val {
        if let Ok(patterns) = v.try_into() {
            config.hygiene.forbid_patterns = patterns;
        }
    }

    let (cit_val, src) = get_val("hygiene", "citation_pattern");
    sources.insert("hygiene.citation_pattern".to_string(), src);
    if let Some(v) = cit_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.hygiene.citation_pattern = Some(v);
    }

    let (lang_val, src) = get_val("hygiene", "languages");
    sources.insert("hygiene.languages".to_string(), src);
    if let Some(v) = lang_val {
        if let Ok(languages) = v.try_into() {
            config.hygiene.languages = languages;
        }
    }

    // 7. [regulatory]
    sources.insert("regulatory".to_string(), section_source("regulatory"));
    let (class_val, src) = get_val("regulatory", "iec62304_class");
    sources.insert("regulatory.iec62304_class".to_string(), src);
    if let Some(v) = class_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.regulatory.iec62304_class = Some(v);
    }

    let (req_val, src) = get_val("regulatory", "require_rationale_for");
    sources.insert("regulatory.require_rationale_for".to_string(), src);
    if let Some(v) = req_val {
        if let Ok(r) = v.try_into() {
            config.regulatory.require_rationale_for = r;
        }
    }

    // 8. [soup]
    sources.insert("soup".to_string(), section_source("soup"));
    let (audit_val, src) = get_val("soup", "audit_command");
    sources.insert("soup.audit_command".to_string(), src);
    if let Some(v) = audit_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.soup.audit_command = Some(v);
    }

    let (deny_val, src) = get_val("soup", "deny_command");
    sources.insert("soup.deny_command".to_string(), src);
    if let Some(v) = deny_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.soup.deny_command = Some(v);
    }

    let (sbom_val, src) = get_val("soup", "sbom_command");
    sources.insert("soup.sbom_command".to_string(), src);
    if let Some(v) = sbom_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.soup.sbom_command = Some(v);
    }

    // 9. [models]
    sources.insert("models".to_string(), section_source("models"));
    let (spec_val, src) = get_val("models", "specify");
    sources.insert("models.specify".to_string(), src);
    if let Some(v) = spec_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.models.specify = Some(v);
    }

    let (dev_val, src) = get_val("models", "develop");
    sources.insert("models.develop".to_string(), src);
    if let Some(v) = dev_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.models.develop = Some(v);
    }

    let (rev_val, src) = get_val("models", "review");
    sources.insert("models.review".to_string(), src);
    if let Some(v) = rev_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.models.review = Some(v);
    }

    // 10. [commit_messages]
    sources.insert(
        "commit_messages".to_string(),
        section_source("commit_messages"),
    );
    let (en_val, src) = get_val("commit_messages", "enabled");
    sources.insert("commit_messages.enabled".to_string(), src);
    if let Some(v) = en_val.and_then(|v| v.as_bool()) {
        config.commit_messages.enabled = v;
    }

    let (fmt_val, src) = get_val("commit_messages", "format");
    sources.insert("commit_messages.format".to_string(), src);
    if let Some(v) = fmt_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.commit_messages.format = v;
    }

    // 11. [environment]
    sources.insert("environment".to_string(), section_source("environment"));
    let mut merged_env = BTreeMap::new();
    if let Some(loc_env) = local_table
        .and_then(|t| t.get("environment"))
        .and_then(|v| v.as_table())
    {
        for (k, v) in loc_env {
            if let Some(s) = v.as_str() {
                merged_env.insert(k.clone(), s.to_string());
                sources.insert(format!("environment.{}", k), ConfigSource::Local);
            }
        }
    }
    if let Some(proj_env) = project_table
        .and_then(|t| t.get("environment"))
        .and_then(|v| v.as_table())
    {
        for (k, v) in proj_env {
            if !merged_env.contains_key(k) {
                if let Some(s) = v.as_str() {
                    merged_env.insert(k.clone(), s.to_string());
                    sources.insert(format!("environment.{}", k), ConfigSource::Project);
                }
            }
        }
    }
    config.environment = EnvironmentConfig::new(merged_env);

    // 12. [[gates]] - WHOLESALE ARRAY REPLACEMENT PER SPEC
    if let Some(loc_gates) = local_table.and_then(|t| t.get("gates")) {
        if let Some(arr) = loc_gates.as_array() {
            sources.insert("gates".to_string(), ConfigSource::Local);
            config.gates = arr
                .clone()
                .into_iter()
                .map(|item| {
                    item.try_into().map_err(|e| {
                        QdevError::usage_error(format!("Failed to deserialize gate: {}", e))
                    })
                })
                .collect::<Result<Vec<GateConfig>, QdevError>>()?;
        } else if let Some(tbl) = loc_gates.as_table() {
            // Local table format [gates] with skip list or boolean
            sources.insert("gates".to_string(), ConfigSource::Local);
            let mut gates: Vec<GateConfig> = if let Some(proj_gates) =
                project_table.and_then(|t| t.get("gates"))
            {
                proj_gates.clone().try_into().map_err(|e| {
                    QdevError::usage_error(format!("Failed to deserialize project gates: {}", e))
                })?
            } else {
                Vec::new()
            };

            if let Some(skip_val) = tbl.get("skip") {
                if let Some(skip_all) = skip_val.as_bool() {
                    for gate in &mut gates {
                        gate.skip = Some(skip_all);
                    }
                } else if let Some(skip_arr) = skip_val.as_array() {
                    let skip_ids: Vec<String> = skip_arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect();
                    for gate in &mut gates {
                        if skip_ids.contains(&gate.id) {
                            gate.skip = Some(true);
                        }
                    }
                }
            }
            config.gates = gates;
        }
    } else if let Some(proj_gates) = project_table.and_then(|t| t.get("gates")) {
        sources.insert("gates".to_string(), ConfigSource::Project);
        config.gates = proj_gates.clone().try_into().map_err(|e| {
            QdevError::usage_error(format!("Failed to deserialize project gates: {}", e))
        })?;
    } else {
        sources.insert("gates".to_string(), ConfigSource::Default);
        config.gates = Vec::new();
    }

    // 13. [identity]
    sources.insert("identity".to_string(), section_source("identity"));
    let loc_dev_id = local_table
        .and_then(|t| t.get("identity"))
        .and_then(|i| i.get("developer_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let proj_dev_id = project_table
        .and_then(|t| t.get("identity"))
        .and_then(|i| i.get("developer_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(dev_id) = loc_dev_id {
        config.identity.developer_id = dev_id;
        sources.insert("identity.developer_id".to_string(), ConfigSource::Local);
    } else if let Some(dev_id) = proj_dev_id {
        config.identity.developer_id = dev_id;
        sources.insert("identity.developer_id".to_string(), ConfigSource::Project);
    } else if let Some(email) = git_email.filter(|e| !e.trim().is_empty()) {
        config.identity.developer_id = email.trim().to_string();
        sources.insert("identity.developer_id".to_string(), ConfigSource::Git);
    } else {
        config.identity.developer_id = String::new();
        sources.insert("identity.developer_id".to_string(), ConfigSource::Default);
    }

    let (teams_val, src) = get_val("identity", "teams");
    sources.insert("identity.teams".to_string(), src);
    if let Some(v) = teams_val {
        if let Ok(teams) = v.try_into() {
            config.identity.teams = teams;
        }
    }

    // 14. [preferences]
    sources.insert("preferences".to_string(), section_source("preferences"));
    let (color_val, src) = get_val("preferences", "color");
    sources.insert("preferences.color".to_string(), src);
    if let Some(v) = color_val.and_then(|v| v.as_bool()) {
        config.preferences.color = v;
    }

    let (fmt_val, src) = get_val("preferences", "default_format");
    sources.insert("preferences.default_format".to_string(), src);
    if let Some(v) = fmt_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.preferences.default_format = v;
    }

    let (ed_val, src) = get_val("preferences", "editor");
    sources.insert("preferences.editor".to_string(), src);
    if let Some(v) = ed_val.and_then(|v| v.as_str().map(|s| s.to_string())) {
        config.preferences.editor = Some(v);
    }

    Ok(AnnotatedConfig::new(config, sources))
}

/// Discovers the workspace root by searching start and its ancestors for `qdev.toml` or `.git`.
pub fn find_workspace_root(start: &Path) -> PathBuf {
    let mut current = if start.is_relative() {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(start)
    } else {
        start.to_path_buf()
    };

    loop {
        if current.join("qdev.toml").is_file() || current.join(".git").exists() {
            return current;
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => break,
        }
    }

    start.to_path_buf()
}

/// Loads committed `qdev.toml` and optional `.qdev.local.toml` from the specified directory or its ancestors,
/// performs strict schema validation, merges key-by-key for tables and wholesale for arrays,
/// and falls back to git config user.email for identity attribution.
pub fn load_config(root: &Path) -> Result<AnnotatedConfig, QdevError> {
    let ws_root = find_workspace_root(root);
    let root = ws_root.as_path();

    let project_file = root.join("qdev.toml");
    let local_file = root.join(".qdev.local.toml");

    let project_table = if project_file.exists() {
        let content = std::fs::read_to_string(&project_file).map_err(|e| {
            QdevError::usage_error(format!("Failed to read qdev.toml: {}", e))
                .with_details(serde_json::json!({ "file": "qdev.toml" }))
        })?;
        let table: toml::Table = toml::from_str(&content).map_err(|e| {
            QdevError::usage_error(format!(
                "Schema violation in qdev.toml: invalid TOML syntax: {}",
                e
            ))
            .with_details(serde_json::json!({ "file": "qdev.toml" }))
        })?;
        validate_config_table(&table, "qdev.toml")?;
        Some(table)
    } else {
        None
    };

    let local_table = if local_file.exists() {
        let content = std::fs::read_to_string(&local_file).map_err(|e| {
            QdevError::usage_error(format!("Failed to read .qdev.local.toml: {}", e))
                .with_details(serde_json::json!({ "file": ".qdev.local.toml" }))
        })?;
        let table: toml::Table = toml::from_str(&content).map_err(|e| {
            QdevError::usage_error(format!(
                "Schema violation in .qdev.local.toml: invalid TOML syntax: {}",
                e
            ))
            .with_details(serde_json::json!({ "file": ".qdev.local.toml" }))
        })?;
        validate_config_table(&table, ".qdev.local.toml")?;
        Some(table)
    } else {
        None
    };

    let git_email = resolve_git_email(Some(root));
    merge_configs(
        project_table.as_ref(),
        local_table.as_ref(),
        git_email.as_deref(),
    )
}
