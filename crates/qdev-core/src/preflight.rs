//! Git Preflight Guard per Story 3.7.
//!
//! Enforces working tree cleanliness within leased story module boundaries (or active chore
//! allowlists), integration branch remote freshness, merge-base staleness limits, and zero
//! blocking validation errors, providing exact Git remediation commands on refusal (exit 3).

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::chore::find_open_chore;
use crate::config::{Config, StorageConfig};
use crate::errors::QdevError;
use crate::lease::{discover_git_branch, find_active_lease_with_storage};
use crate::modules::ModuleRegistry;
use crate::schema::{extract_frontmatter, EntityKind};
use crate::store::{FindingRecord, Store};
use crate::validate::{glob_match, run_validation};
use crate::write::resolve_entity_file;

/// Options configuring the preflight execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreflightOptions {
    /// Optional story ID to evaluate module scope against.
    pub story: Option<String>,
}

/// Preflight pass/refusal outcome status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreflightStatus {
    Pass,
    Refusal,
}

impl PreflightStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PreflightStatus::Pass => "pass",
            PreflightStatus::Refusal => "refusal",
        }
    }
}

/// A structured preflight diagnostic record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightDiagnostic {
    pub category: String,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

/// Full outcome of a preflight guard evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightOutcome {
    pub status: PreflightStatus,
    pub summary: String,
    pub diagnostics: Vec<PreflightDiagnostic>,
    pub remediation_commands: Vec<String>,
}

/// JSON payload structure for `--json` output conforming to `payload-preflight.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightPayload {
    pub status: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<PreflightDiagnostic>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remediation_commands: Vec<String>,
}

impl From<PreflightOutcome> for PreflightPayload {
    fn from(outcome: PreflightOutcome) -> Self {
        Self {
            status: outcome.status.as_str().to_string(),
            summary: outcome.summary,
            diagnostics: outcome.diagnostics,
            remediation_commands: outcome.remediation_commands,
        }
    }
}

/// Checks whether `rel_path` belongs to exempt workspace metadata.
pub fn is_metadata_exempt(rel_path: &str, storage: &StorageConfig) -> bool {
    let mut normalized = rel_path.replace('\\', "/");
    loop {
        if let Some(stripped) = normalized.strip_prefix('/') {
            normalized = stripped.to_string();
        } else if let Some(stripped) = normalized.strip_prefix("./") {
            normalized = stripped.to_string();
        } else {
            break;
        }
    }

    if normalized == "qdev.toml" || normalized == ".qdev.local.toml" || normalized == ".gitignore" {
        return true;
    }
    if normalized == ".qdev" || normalized.starts_with(".qdev/") {
        return true;
    }
    if normalized == ".git" || normalized.starts_with(".git/") {
        return true;
    }
    if normalized == "docs" || normalized.starts_with("docs/") {
        return true;
    }

    let normalize_dir = |d: &str| -> String {
        let mut s = d.replace('\\', "/");
        loop {
            if let Some(stripped) = s.strip_prefix("./") {
                s = stripped.to_string();
            } else if let Some(stripped) = s.strip_prefix('/') {
                s = stripped.to_string();
            } else {
                break;
            }
        }
        s.trim_end_matches('/').to_string()
    };

    let specs = normalize_dir(&storage.specs_dir);
    if !specs.is_empty() && (normalized == specs || normalized.starts_with(&format!("{}/", specs))) {
        return true;
    }
    let state = normalize_dir(&storage.state_dir);
    if !state.is_empty() && (normalized == state || normalized.starts_with(&format!("{}/", state))) {
        return true;
    }
    let cache = normalize_dir(&storage.cache_dir);
    if !cache.is_empty() && (normalized == cache || normalized.starts_with(&format!("{}/", cache))) {
        return true;
    }

    false
}

/// Resolves target modules for a story either from SQLite store or from the story specification file.
pub fn resolve_story_target_modules(
    workspace_root: &Path,
    config: &Config,
    opt_store: Option<&dyn Store>,
    story_id: &str,
) -> Result<Vec<String>, QdevError> {
    if let Some(store) = opt_store {
        if let Ok(Some(story)) = store.get_story_details(story_id) {
            if let Some(ref tm_json) = story.target_modules {
                if let Ok(modules) = serde_json::from_str::<Vec<String>>(tm_json) {
                    return Ok(modules);
                }
            }
        }
    }

    let (_, _, story_file) = resolve_entity_file(
        workspace_root,
        Some(EntityKind::Story),
        story_id,
        Some(&config.storage),
    ).map_err(|_| {
        QdevError::usage_error(format!("Story '{}' not found", story_id))
    })?;

    let content = std::fs::read_to_string(&story_file).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to read story file '{}': {}", story_file.display(), e),
        )
    })?;

    let frontmatter = extract_frontmatter(&content).map_err(|e| {
        QdevError::logical_failure(
            "schema_error",
            format!("Failed to parse story frontmatter in '{}': {}", story_file.display(), e),
        )
    })?;

    let mut target_modules = Vec::new();
    if let Some(arr) = frontmatter.get("target_modules").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() && !target_modules.contains(&trimmed.to_string()) {
                    target_modules.push(trimmed.to_string());
                }
            }
        }
    }

    Ok(target_modules)
}

/// Inspects working tree cleanliness and checks uncommitted files against module/chore scope.
pub fn check_working_tree_scope(
    workspace_root: &Path,
    config: &Config,
    opt_store: Option<&dyn Store>,
    options: &PreflightOptions,
) -> Result<Option<PreflightDiagnostic>, QdevError> {
    if !config.git.require_clean_tree_in_scope {
        return Ok(None);
    }

    let status_output = Command::new("git")
        .args(["-c", "core.quotePath=false", "status", "--porcelain=v1", "-uall"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to run 'git status': {}", e),
            )
        })?;

    if !status_output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_status_failed",
            format!(
                "'git status' failed: {}",
                String::from_utf8_lossy(&status_output.stderr).trim()
            ),
        ));
    }

    let stdout_str = String::from_utf8_lossy(&status_output.stdout);
    let mut uncommitted_files = HashSet::new();

    for line in stdout_str.lines() {
        let trimmed_line = line.trim_end();
        if trimmed_line.len() >= 3 {
            let mut path_part = trimmed_line[3..].trim();
            if let Some(idx) = path_part.find(" -> ") {
                path_part = &path_part[idx + 4..];
            }
            let cleaned_path = path_part.trim_matches('"');
            if !cleaned_path.is_empty() && !is_metadata_exempt(cleaned_path, &config.storage) {
                uncommitted_files.insert(cleaned_path.to_string());
            }
        }
    }

    if uncommitted_files.is_empty() {
        return Ok(None);
    }

    // Scope determination hierarchy
    enum ActiveScope {
        Story(String, Vec<String>), // (story_id, target_modules)
        Chore(String, Vec<String>), // (chore_id, allowlist_patterns)
        None,
    }

    let active_scope = if let Some(ref sid) = options.story {
        let modules = resolve_story_target_modules(workspace_root, config, opt_store, sid)?;
        ActiveScope::Story(sid.clone(), modules)
    } else {
        match find_active_lease_with_storage(workspace_root, Some(&config.storage)) {
            Ok(lease) => {
                let modules = resolve_story_target_modules(workspace_root, config, opt_store, &lease.story_id)?;
                ActiveScope::Story(lease.story_id, modules)
            }
            Err(e) => {
                if e.code() != "no_active_lease" {
                    return Ok(Some(PreflightDiagnostic {
                        category: "working_tree".to_string(),
                        code: "multiple_active_leases".to_string(),
                        message: format!(
                            "multiple active leases in workspace; specify story with '--story <id>' to resolve scope ({})",
                            e.message()
                        ),
                        paths: Vec::new(),
                        remediation: Some("qdev preflight --story <id>".to_string()),
                    }));
                }
                if let Ok(Some(chore)) = find_open_chore(workspace_root, Some(&config.storage)) {
                    ActiveScope::Chore(chore.id, chore.paths)
                } else {
                    ActiveScope::None
                }
            }
        }
    };

    let registry = ModuleRegistry::from_config(config);
    let mut out_of_scope = Vec::new();

    for path in &uncommitted_files {
        let in_scope = match &active_scope {
            ActiveScope::Story(_, target_modules) => {
                let mut matched = false;
                for mod_id in target_modules {
                    if let Some(module) = registry.get(mod_id) {
                        if module.paths.iter().any(|pattern| glob_match(pattern, path)) {
                            matched = true;
                            break;
                        }
                    }
                }
                matched
            }
            ActiveScope::Chore(_, patterns) => {
                patterns.iter().any(|pattern| glob_match(pattern, path))
            }
            ActiveScope::None => false,
        };

        if !in_scope {
            out_of_scope.push(path.clone());
        }
    }

    if out_of_scope.is_empty() {
        return Ok(None);
    }

    out_of_scope.sort();

    let quoted_paths = out_of_scope
        .iter()
        .map(|p| format!("\"{}\"", p))
        .collect::<Vec<_>>()
        .join(" ");

    let (message, remediation) = match &active_scope {
        ActiveScope::Story(sid, _) => (
            format!(
                "uncommitted changes outside target modules for story '{}':\n{}",
                sid,
                out_of_scope.iter().map(|p| format!("    {}", p)).collect::<Vec<_>>().join("\n")
            ),
            format!("git stash push -u -m \"out-of-scope\" -- {}", quoted_paths),
        ),
        ActiveScope::Chore(cid, _) => (
            format!(
                "uncommitted changes outside allowlist for chore '{}':\n{}",
                cid,
                out_of_scope.iter().map(|p| format!("    {}", p)).collect::<Vec<_>>().join("\n")
            ),
            format!("git stash push -u -m \"out-of-scope\" -- {}", quoted_paths),
        ),
        ActiveScope::None => (
            format!(
                "no story lease or chore active in workspace; uncommitted changes are out of scope:\n{}",
                out_of_scope.iter().map(|p| format!("    {}", p)).collect::<Vec<_>>().join("\n")
            ),
            "git stash -u".to_string(),
        ),
    };

    Ok(Some(PreflightDiagnostic {
        category: "working_tree".to_string(),
        code: "out_of_scope_changes".to_string(),
        message,
        paths: out_of_scope,
        remediation: Some(remediation),
    }))
}

/// Checks branch freshness and integration merge-base staleness.
pub fn check_branch_freshness(
    workspace_root: &Path,
    config: &Config,
    active_story_id: Option<&str>,
) -> Result<Vec<PreflightDiagnostic>, QdevError> {
    let mut diagnostics = Vec::new();
    let current_branch = discover_git_branch(workspace_root);
    let remote = &config.git.remote;

    match config.git.branching_mode.as_str() {
        "trunk" => {
            // Verify current branch is not behind upstream tracking ref
            let tracking_ref = format!("refs/remotes/{}/{}", remote, current_branch);
            let verify = Command::new("git")
                .args(["rev-parse", "--verify", "--quiet", &tracking_ref])
                .current_dir(workspace_root)
                .output()
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "git_unavailable",
                        format!("Failed to verify remote tracking ref: {}", e),
                    )
                })?;

            if verify.status.success() {
                let rev_spec = format!("HEAD..{}/{}", remote, current_branch);
                let count_out = Command::new("git")
                    .args(["rev-list", "--count", &rev_spec])
                    .current_dir(workspace_root)
                    .output()
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "git_unavailable",
                            format!("Failed to run 'git rev-list': {}", e),
                        )
                    })?;

                if count_out.status.success() {
                    let count_str = String::from_utf8_lossy(&count_out.stdout).trim().to_string();
                    let behind_count: u32 = count_str.parse().unwrap_or(0);
                    if behind_count > 0 {
                        let remediation = format!("git pull {} {}", remote, current_branch);
                        diagnostics.push(PreflightDiagnostic {
                            category: "branch_freshness".to_string(),
                            code: "trunk_branch_behind_remote".to_string(),
                            message: format!(
                                "branch '{}' is {} commit(s) behind '{}/{}'",
                                current_branch, behind_count, remote, current_branch
                            ),
                            paths: Vec::new(),
                            remediation: Some(remediation),
                        });
                    }
                }
            }
        }
        _ => {
            // "story-branch" mode
            let integration_branch = &config.git.integration_branch;

            let local_exists = Command::new("git")
                .args(["rev-parse", "--verify", "--quiet", integration_branch])
                .current_dir(workspace_root)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            let remote_ref = format!("refs/remotes/{}/{}", remote, integration_branch);
            let remote_exists = Command::new("git")
                .args(["rev-parse", "--verify", "--quiet", &remote_ref])
                .current_dir(workspace_root)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if !local_exists && !remote_exists {
                diagnostics.push(PreflightDiagnostic {
                    category: "branch_freshness".to_string(),
                    code: "integration_branch_missing".to_string(),
                    message: format!(
                        "configured integration branch '{}' does not exist locally or on remote '{}'",
                        integration_branch, remote
                    ),
                    paths: Vec::new(),
                    remediation: Some(format!("git checkout -b {}", integration_branch)),
                });
                return Ok(diagnostics);
            }

            // 1. Check if remote tracking ref exists and local integration branch is behind
            if remote_exists && local_exists {
                let rev_spec = format!("{}..{}/{}", integration_branch, remote, integration_branch);
                let count_out = Command::new("git")
                    .args(["rev-list", "--count", &rev_spec])
                    .current_dir(workspace_root)
                    .output()
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "git_unavailable",
                            format!("Failed to run 'git rev-list': {}", e),
                        )
                    })?;

                if count_out.status.success() {
                    let count_str = String::from_utf8_lossy(&count_out.stdout).trim().to_string();
                    let behind_count: u32 = count_str.parse().unwrap_or(0);
                    if behind_count > 0 {
                        let remediation = if current_branch == *integration_branch {
                            format!("git pull {} {}", remote, integration_branch)
                        } else {
                            format!(
                                "git fetch {} {}:{}",
                                remote, integration_branch, integration_branch
                            )
                        };
                        diagnostics.push(PreflightDiagnostic {
                            category: "branch_freshness".to_string(),
                            code: "integration_branch_behind_remote".to_string(),
                            message: format!(
                                "local '{}' is {} commit(s) behind '{}/{}'",
                                integration_branch, behind_count, remote, integration_branch
                            ),
                            paths: Vec::new(),
                            remediation: Some(remediation),
                        });
                    }
                }
            } else if remote_exists && !local_exists {
                diagnostics.push(PreflightDiagnostic {
                    category: "branch_freshness".to_string(),
                    code: "local_integration_branch_missing".to_string(),
                    message: format!(
                        "local integration branch '{}' does not exist; remote tracking ref '{}/{}' exists",
                        integration_branch, remote, integration_branch
                    ),
                    paths: Vec::new(),
                    remediation: Some(format!(
                        "git checkout -b {} {}/{}",
                        integration_branch, remote, integration_branch
                    )),
                });
            }

            // 2. Check merge-base staleness against local integration branch
            let verify_local_ib = Command::new("git")
                .args(["rev-parse", "--verify", "--quiet", integration_branch])
                .current_dir(workspace_root)
                .output()
                .map_err(|e| {
                    QdevError::infrastructure_failure(
                        "git_unavailable",
                        format!("Failed to check integration branch: {}", e),
                    )
                })?;

            if verify_local_ib.status.success() {
                let mb_output = Command::new("git")
                    .args(["merge-base", "HEAD", integration_branch])
                    .current_dir(workspace_root)
                    .output()
                    .map_err(|e| {
                        QdevError::infrastructure_failure(
                            "git_unavailable",
                            format!("Failed to compute merge-base: {}", e),
                        )
                    })?;

                if mb_output.status.success() {
                    let mb = String::from_utf8_lossy(&mb_output.stdout).trim().to_string();
                    if !mb.is_empty() {
                        let staleness_spec = format!("{}..{}", mb, integration_branch);
                        let count_out = Command::new("git")
                            .args(["rev-list", "--count", &staleness_spec])
                            .current_dir(workspace_root)
                            .output()
                            .map_err(|e| {
                                QdevError::infrastructure_failure(
                                    "git_unavailable",
                                    format!("Failed to count staleness commits: {}", e),
                                )
                            })?;

                        if count_out.status.success() {
                            let count_str = String::from_utf8_lossy(&count_out.stdout).trim().to_string();
                            let staleness: u32 = count_str.parse().unwrap_or(0);
                            let max_staleness = config.git.max_integration_staleness_commits;
                            if staleness > max_staleness {
                                let story_part = if let Some(sid) = active_story_id {
                                    format!(" before continuing {}", sid)
                                } else {
                                    String::new()
                                };
                                let message = format!(
                                    "{} is {} commits behind {} at merge-base (limit {}).\n  Rebase onto {}{}.",
                                    current_branch, staleness, integration_branch, max_staleness, integration_branch, story_part
                                );
                                let remediation = format!("git rebase {}", integration_branch);
                                diagnostics.push(PreflightDiagnostic {
                                    category: "branch_freshness".to_string(),
                                    code: "merge_base_stale".to_string(),
                                    message,
                                    paths: Vec::new(),
                                    remediation: Some(remediation),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(diagnostics)
}

/// Evaluates live validation findings and flags any error-severity findings as blocking.
pub fn check_validation_health(
    workspace_root: &Path,
    config: &Config,
    opt_store: Option<&dyn Store>,
) -> Result<Option<PreflightDiagnostic>, QdevError> {
    let findings: Vec<FindingRecord> = if let Some(store) = opt_store {
        run_validation(store, workspace_root, config)?
    } else {
        let db_path = workspace_root.join(&config.storage.cache_dir).join("cache.sqlite");
        if db_path.is_file() {
            if let Ok(store) = crate::store::SqliteStore::open(&db_path) {
                run_validation(&store, workspace_root, config)?
            } else {
                Vec::new()
            }
        } else if let Ok(mem_store) = crate::store::SqliteStore::open_in_memory() {
            run_validation(&mem_store, workspace_root, config)?
        } else {
            Vec::new()
        }
    };

    let error_findings: Vec<&FindingRecord> = findings.iter().filter(|f| f.severity == "error").collect();
    if error_findings.is_empty() {
        return Ok(None);
    }

    let mut paths = Vec::new();
    let mut details = Vec::new();
    for f in &error_findings {
        if !paths.contains(&f.path) {
            paths.push(f.path.clone());
        }
        details.push(format!("[{}] {}", f.code, f.path));
    }

    let message = format!(
        "workspace has {} blocking validation error(s):\n{}",
        error_findings.len(),
        details.iter().map(|d| format!("    {}", d)).collect::<Vec<_>>().join("\n")
    );

    Ok(Some(PreflightDiagnostic {
        category: "validation".to_string(),
        code: "blocking_validation_errors".to_string(),
        message,
        paths,
        remediation: Some("qdev validate".to_string()),
    }))
}

/// Runs full preflight inspection.
pub fn run_preflight(
    workspace_root: &Path,
    config: &Config,
    options: &PreflightOptions,
    opt_store: Option<&dyn Store>,
) -> Result<PreflightOutcome, QdevError> {
    // 0. Verify git repository existence
    let is_git_repo = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace_root)
        .output();

    match is_git_repo {
        Ok(output) => {
            if !output.status.success() || String::from_utf8_lossy(&output.stdout).trim() != "true" {
                return Err(QdevError::infrastructure_failure(
                    "not_a_git_repository",
                    "Workspace is not a git repository",
                ));
            }
        }
        Err(e) => {
            return Err(QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to execute git: {}", e),
            ));
        }
    }

    let mut diagnostics = Vec::new();
    let mut remediation_commands = Vec::new();

    // 1. Working tree scope check
    if let Some(scope_diag) = check_working_tree_scope(workspace_root, config, opt_store, options)? {
        if let Some(ref cmd) = scope_diag.remediation {
            if !remediation_commands.contains(cmd) {
                remediation_commands.push(cmd.clone());
            }
        }
        diagnostics.push(scope_diag);
    }

    // Determine active story id for diagnostics
    let lease_story = if options.story.is_none() {
        find_active_lease_with_storage(workspace_root, Some(&config.storage))
            .ok()
            .map(|l| l.story_id)
    } else {
        None
    };
    let effective_story_id = options.story.as_deref().or(lease_story.as_deref());

    // 2. Branch freshness check
    let freshness_diags = check_branch_freshness(workspace_root, config, effective_story_id)?;
    for diag in freshness_diags {
        if let Some(ref cmd) = diag.remediation {
            if !remediation_commands.contains(cmd) {
                remediation_commands.push(cmd.clone());
            }
        }
        diagnostics.push(diag);
    }

    // 3. Validation health check
    if let Some(val_diag) = check_validation_health(workspace_root, config, opt_store)? {
        if let Some(ref cmd) = val_diag.remediation {
            if !remediation_commands.contains(cmd) {
                remediation_commands.push(cmd.clone());
            }
        }
        diagnostics.push(val_diag);
    }

    if diagnostics.is_empty() {
        Ok(PreflightOutcome {
            status: PreflightStatus::Pass,
            summary: "working tree in scope, integration branch fresh, zero blocking findings".to_string(),
            diagnostics: Vec::new(),
            remediation_commands: Vec::new(),
        })
    } else {
        Ok(PreflightOutcome {
            status: PreflightStatus::Refusal,
            summary: format!("preflight checks failed with {} issue(s)", diagnostics.len()),
            diagnostics,
            remediation_commands,
        })
    }
}

/// Formats the human-readable text output for preflight.
pub fn format_preflight_text(outcome: &PreflightOutcome) -> String {
    if outcome.status == PreflightStatus::Pass {
        format!("✓ preflight: {}", outcome.summary)
    } else {
        let mut lines = Vec::new();
        for diag in &outcome.diagnostics {
            lines.push(format!("✖ preflight: {}", diag.message));
            if let Some(ref cmd) = diag.remediation {
                lines.push(format!("  Fix: {}", cmd));
            }
        }
        lines.join("\n")
    }
}
