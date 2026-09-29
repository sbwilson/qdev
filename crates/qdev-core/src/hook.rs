//! Git Hook Shims & qdev hook runner per Story 3.8.
//!
//! Provides two-line POSIX hook shims (`pre-commit`, `pre-push`, `prepare-commit-msg`)
//! delegating to `qdev hook <name>`, preserving and chaining existing non-qdev hooks,
//! staged secret scanning for NFR-404, and integration with `qdev doctor [--fix]`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::gate::git::{diff_cached_name_only, run_git};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::errors::QdevError;
use crate::lease::find_active_lease_with_storage;
use crate::preflight::{run_preflight, PreflightOptions, PreflightStatus};
use crate::schema::extract_frontmatter;
use crate::schema::EntityKind;
use crate::scratch::read_scratch_entries;
use crate::store::Store;
use crate::write::{resolve_entity_file, write_file_atomic};

/// The canonical expected git hook names managed by qdev.
pub const EXPECTED_HOOKS: &[&str] = &["pre-commit", "pre-push", "prepare-commit-msg"];

/// Returns the standard POSIX two-line shim content for a given hook.
///
/// Always uses standard LF newlines (`\n`) for cross-platform compatibility
/// with Git for Windows MSYS bash.
pub fn shim_content(hook_name: &str) -> String {
    format!("#!/bin/sh\nexec qdev hook {} \"$@\"\n", hook_name)
}

/// Discovers the Git hooks directory for a workspace.
///
/// Executes `git rev-parse --git-path hooks` to properly handle both standard
/// repositories and linked git worktrees. Falls back to `.git/hooks` if the directory exists.
pub fn resolve_hooks_dir(workspace_root: &Path) -> Result<PathBuf, QdevError> {
    let output = run_git(workspace_root, &["rev-parse", "--git-path", "hooks"]);

    match output {
        Ok(out) if out.status.success() => {
            let rel = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if rel.is_empty() {
                return Err(QdevError::infrastructure_failure(
                    "not_a_git_repository",
                    "Workspace is not a git repository",
                ));
            }
            let p = PathBuf::from(rel);
            if p.is_absolute() {
                Ok(p)
            } else {
                Ok(workspace_root.join(p))
            }
        }
        _ => {
            let git_dir = workspace_root.join(".git");
            if git_dir.is_dir() {
                Ok(git_dir.join("hooks"))
            } else {
                Err(QdevError::infrastructure_failure(
                    "not_a_git_repository",
                    "Workspace is not a git repository",
                ))
            }
        }
    }
}

/// Status of Git hook shims in a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookStatus {
    pub all_installed: bool,
    pub missing_hooks: Vec<String>,
    pub outdated_hooks: Vec<String>,
}

/// Inspects the current installation state of Git hook shims.
pub fn inspect_hooks(workspace_root: &Path) -> Result<HookStatus, QdevError> {
    let hooks_dir = resolve_hooks_dir(workspace_root)?;

    let mut missing_hooks = Vec::new();
    let mut outdated_hooks = Vec::new();

    for hook in EXPECTED_HOOKS {
        let hook_path = hooks_dir.join(hook);
        if !hook_path.exists() {
            missing_hooks.push(hook.to_string());
            continue;
        }

        if !is_executable(&hook_path) {
            missing_hooks.push(hook.to_string());
            continue;
        }

        let content = match fs::read_to_string(&hook_path) {
            Ok(c) => c,
            Err(_) => {
                outdated_hooks.push(hook.to_string());
                continue;
            }
        };

        // Outdated if CRLF or contents do not match expected shim or invocation
        let expected = shim_content(hook);
        if content.contains('\r')
            || content.trim() != expected.trim()
            || !content.contains(&format!("qdev hook {}", hook))
        {
            outdated_hooks.push(hook.to_string());
        }
    }

    let all_installed = missing_hooks.is_empty() && outdated_hooks.is_empty();

    Ok(HookStatus {
        all_installed,
        missing_hooks,
        outdated_hooks,
    })
}

/// Result of installing or updating Git hook shims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookInstallReport {
    pub hooks_dir: String,
    pub installed_hooks: Vec<String>,
    pub preserved_legacy: Vec<String>,
}

/// Writes or updates all expected Git hook shims, preserving non-qdev hooks by chaining.
pub fn install_hooks(workspace_root: &Path) -> Result<HookInstallReport, QdevError> {
    let hooks_dir = resolve_hooks_dir(workspace_root)?;
    fs::create_dir_all(&hooks_dir).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to create hooks directory: {}", e),
        )
    })?;

    let mut installed_hooks = Vec::new();
    let mut preserved_legacy = Vec::new();

    for hook in EXPECTED_HOOKS {
        let hook_path = hooks_dir.join(hook);
        if hook_path.exists() {
            let content = fs::read_to_string(&hook_path).unwrap_or_default();
            if content.contains(&format!("qdev hook {}", hook)) {
                // Already a qdev shim; update in-place without renaming
                write_shim(&hook_path, hook)?;
                installed_hooks.push(hook.to_string());
            } else {
                // Non-qdev hook; preserve by renaming to <name>.legacy
                let legacy_path = hooks_dir.join(format!("{}.legacy", hook));
                if !legacy_path.exists() {
                    fs::rename(&hook_path, &legacy_path).map_err(|e| {
                        QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to rename legacy hook '{}': {}", hook, e),
                        )
                    })?;
                    preserved_legacy.push(hook.to_string());
                }
                write_shim(&hook_path, hook)?;
                installed_hooks.push(hook.to_string());
            }
        } else {
            write_shim(&hook_path, hook)?;
            installed_hooks.push(hook.to_string());
        }
    }

    Ok(HookInstallReport {
        hooks_dir: hooks_dir.to_string_lossy().to_string(),
        installed_hooks,
        preserved_legacy,
    })
}

fn write_shim(hook_path: &Path, hook: &str) -> Result<(), QdevError> {
    let content = shim_content(hook);
    fs::write(hook_path, content.as_bytes()).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to write hook shim '{}': {}", hook_path.display(), e),
        )
    })?;
    set_executable(hook_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to set permissions on '{}': {}",
                hook_path.display(),
                e
            ),
        )
    })?;
    Ok(())
}

fn set_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)?;
    }
    let _ = path;
    Ok(())
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            return meta.permissions().mode() & 0o111 != 0;
        }
        false
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// A violation detected by the staged secret scanner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretViolation {
    pub file: String,
    pub pattern: String,
}

/// Scans staged scratchpad and evidence files for patterns configured in `config.hygiene.secret_patterns`.
///
/// Satisfies NFR-404: Never scans unstaged files; only inspects staged changes via `git diff --cached`.
pub fn scan_staged_secrets(
    workspace_root: &Path,
    config: &Config,
) -> Result<Vec<SecretViolation>, QdevError> {
    if config.hygiene.secret_patterns.is_empty() {
        return Ok(Vec::new());
    }

    let mut compiled = Vec::new();
    for pat in &config.hygiene.secret_patterns {
        match regex::Regex::new(pat) {
            Ok(re) => compiled.push((pat.clone(), re)),
            Err(e) => {
                return Err(QdevError::usage_error(format!(
                    "Invalid regex in hygiene.secret_patterns '{}': {}",
                    pat, e
                )));
            }
        }
    }

    let output = diff_cached_name_only(workspace_root).map_err(|e| {
        QdevError::infrastructure_failure(
            "git_unavailable",
            format!("Failed to execute git diff --cached: {}", e),
        )
    })?;

    if !output.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_error",
            format!(
                "git diff --cached failed: {}",
                String::from_utf8_lossy(&output.stderr)
            ),
        ));
    }

    let files_str = String::from_utf8_lossy(&output.stdout);
    let mut violations = Vec::new();

    let state_dir = config.storage.state_dir.replace('\\', "/");
    let trimmed_state_dir = state_dir.trim_end_matches('/');

    for raw_line in files_str.lines() {
        let line = raw_line
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .replace('\\', "/");
        if line.is_empty() {
            continue;
        }

        let in_scratch = line.starts_with("scratch/")
            || line.contains("/scratch/")
            || line.starts_with("docs/state/scratch/")
            || line.starts_with(&format!("{}/scratch/", trimmed_state_dir));
        let is_scratch = in_scratch && line.ends_with(".jsonl");

        let in_evidence = line.starts_with("evidence/")
            || line.contains("/evidence/")
            || line.starts_with("docs/state/evidence/")
            || line.starts_with(&format!("{}/evidence/", trimmed_state_dir));
        let is_evidence = in_evidence && line.ends_with(".json");

        if !is_scratch && !is_evidence {
            continue;
        }

        let show_out = run_git(workspace_root, &["show", &format!(":{}", line)]);

        let content = match show_out {
            Ok(res) if res.status.success() => String::from_utf8_lossy(&res.stdout).to_string(),
            _ => continue,
        };

        for (pat_str, re) in &compiled {
            if re.is_match(&content) {
                violations.push(SecretViolation {
                    file: line.clone(),
                    pattern: pat_str.clone(),
                });
            }
        }
    }

    Ok(violations)
}

/// Executes a legacy chained hook if present, propagating its exit code.
pub fn run_legacy_hook(
    workspace_root: &Path,
    hooks_dir: &Path,
    hook_name: &str,
    args: &[String],
    forward_stdin: bool,
) -> Result<Option<i32>, QdevError> {
    let legacy_name = format!("{}.legacy", hook_name);
    let legacy_path = hooks_dir.join(&legacy_name);
    if !legacy_path.exists() {
        return Ok(None);
    }

    #[cfg(windows)]
    let mut cmd = {
        let is_binary = legacy_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                let e = ext.to_ascii_lowercase();
                e == "exe" || e == "cmd" || e == "bat"
            })
            .unwrap_or(false);

        if is_binary {
            let mut c = Command::new(&legacy_path);
            c.args(args);
            c
        } else {
            let mut c = Command::new("sh");
            c.arg(&legacy_path).args(args);
            c
        }
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new(&legacy_path);
        c.args(args);
        c
    };

    cmd.current_dir(workspace_root);

    if forward_stdin {
        cmd.stdin(std::process::Stdio::inherit());
    } else {
        cmd.stdin(std::process::Stdio::null());
    }
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    let status = cmd.status().map_err(|e| {
        QdevError::infrastructure_failure(
            "legacy_hook_execution_failed",
            format!(
                "Failed to execute legacy hook '{}': {}",
                legacy_path.display(),
                e
            ),
        )
    })?;

    Ok(Some(status.code().unwrap_or(1)))
}

/// Executes `qdev hook pre-commit` checks:
/// 1. Hygiene comment linter (stubbed / pass for Story 3.8).
/// 2. Changed validation (`validate --changed`).
/// 3. NFR-404 secret pattern scan over staged scratchpad/evidence files.
/// 4. Chained `.git/hooks/pre-commit.legacy` execution.
pub fn run_pre_commit(
    workspace_root: &Path,
    config: &Config,
    args: &[String],
    opt_store: Option<&dyn Store>,
) -> Result<Option<i32>, QdevError> {
    // 1. Hygiene check --diff
    let qdev_bin = std::env::current_exe()
        .ok()
        .and_then(|current| {
            let name = current.file_name()?.to_string_lossy();
            if name == "qdev" || name == "qdev.exe" {
                Some(current)
            } else {
                let candidate = current.parent()?.parent()?.join("qdev");
                if candidate.exists() {
                    Some(candidate)
                } else {
                    None
                }
            }
        })
        .unwrap_or_else(|| PathBuf::from("qdev"));

    let hygiene_res = Command::new(qdev_bin)
        .args(["hygiene", "check", "--diff"])
        .current_dir(workspace_root)
        .status();

    if let Ok(status) = hygiene_res {
        if status.code() == Some(1) {
            return Err(QdevError::logical_failure(
                "hygiene_failure",
                "Hygiene check failed on staged/diff changes",
            ));
        }
    }

    // 2. Validate --changed
    let local_store;
    let store_ref: &dyn Store = match opt_store {
        Some(s) => s,
        None => {
            let db_path = workspace_root
                .join(&config.storage.cache_dir)
                .join("cache.sqlite");
            if db_path.exists() {
                local_store = crate::store::SqliteStore::open(&db_path)?;
                &local_store
            } else {
                // If cache store is not initialized, run in-memory / skip error check
                local_store = crate::store::SqliteStore::open_in_memory()?;
                &local_store
            }
        }
    };

    let mut findings = crate::validate::run_validation(store_ref, workspace_root, config)?;
    match crate::validate::git_changed_files(workspace_root, &config.git.integration_branch) {
        Ok(changed_paths) => {
            findings = crate::validate::filter_by_changed(findings, &changed_paths);
        }
        Err(_) => {
            // Integration branch might not exist or no commits yet; fall back to staged files
            if let Ok(output) = diff_cached_name_only(workspace_root) {
                if output.status.success() {
                    let files_str = String::from_utf8_lossy(&output.stdout);
                    let staged_paths: std::collections::HashSet<String> = files_str
                        .lines()
                        .map(|l| {
                            l.trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .replace('\\', "/")
                        })
                        .filter(|l| !l.is_empty())
                        .collect();
                    findings = crate::validate::filter_by_changed(findings, &staged_paths);
                }
            }
        }
    }
    crate::validate::sort_findings(&mut findings);
    if crate::validate::has_error_finding(&findings) {
        let mut msg = String::from("Validation failed with error findings on changed files:\n");
        for f in &findings {
            if f.severity == "error" {
                msg.push_str(&format!(
                    "  [{}] {}: {}\n",
                    f.code,
                    f.path,
                    f.message.as_deref().unwrap_or("")
                ));
            }
        }
        return Err(QdevError::logical_failure(
            "validation_error",
            msg.trim_end(),
        ));
    }

    // 3. Secret patterns scan (NFR-404)
    let violations = scan_staged_secrets(workspace_root, config)?;
    if !violations.is_empty() {
        let mut msg = format!(
            "Found {} secret violation(s) in staged files:\n",
            violations.len()
        );
        for v in &violations {
            msg.push_str(&format!(
                "  secret pattern match in staged file '{}': pattern '{}'\n",
                v.file, v.pattern
            ));
        }
        return Err(QdevError::logical_failure(
            "secret_detected",
            msg.trim_end(),
        ));
    }

    // 4. Legacy hook chaining
    let hooks_dir = resolve_hooks_dir(workspace_root)?;
    run_legacy_hook(workspace_root, &hooks_dir, "pre-commit", args, false)
}

/// Executes `qdev hook pre-push` checks:
/// 1. Runs preflight guard (`run_preflight`).
/// 2. Chained `.git/hooks/pre-push.legacy` execution with forwarded stdin.
pub fn run_pre_push(
    workspace_root: &Path,
    config: &Config,
    args: &[String],
    opt_store: Option<&dyn Store>,
) -> Result<Option<i32>, QdevError> {
    // 1. Run preflight
    let outcome = run_preflight(
        workspace_root,
        config,
        &PreflightOptions::default(),
        opt_store,
    )?;

    if outcome.status == PreflightStatus::Refusal {
        let text = crate::preflight::format_preflight_text(&outcome);
        let attribution = crate::errors::RejectionAttribution::new(
            "Working tree modifications must remain within active story scope before push",
        )
        .with_policy("preflight_guard");

        return Err(QdevError::policy_refusal(
            "preflight_refusal",
            text.trim_end(),
        )
        .with_details(serde_json::json!({
            "status": "refusal",
            "summary": outcome.summary,
            "diagnostics": outcome.diagnostics,
            "remediation_commands": outcome.remediation_commands,
        }))
        .with_attribution(attribution));
    }

    // 2. Legacy hook chaining (with stdin forwarded)
    let hooks_dir = resolve_hooks_dir(workspace_root)?;
    run_legacy_hook(workspace_root, &hooks_dir, "pre-push", args, true)
}

/// Executes `qdev hook prepare-commit-msg`:
/// 1. If `config.commit_messages.enabled` is false, no-op.
/// 2. If enabled, draft message (Story 3.11).
/// 3. Chained `.git/hooks/prepare-commit-msg.legacy` execution.
pub fn run_prepare_commit_msg(
    workspace_root: &Path,
    config: &Config,
    args: &[String],
) -> Result<Option<i32>, QdevError> {
    if config.commit_messages.enabled {
        if let Some(target_file) = args.first() {
            let target_path = PathBuf::from(target_file);
            let p = if target_path.is_absolute() {
                target_path
            } else {
                workspace_root.join(target_path)
            };
            if p.exists() {
                let existing = fs::read_to_string(&p).map_err(|e| {
                    QdevError::infrastructure_failure(
                        "commit_message_read_failed",
                        format!(
                            "Failed to read commit message file '{}': {}",
                            p.display(),
                            e
                        ),
                    )
                })?;
                let comment_char = git_comment_char(workspace_root)?;
                let has_message_content = existing
                    .lines()
                    .filter(|l| !l.trim_start().starts_with(comment_char))
                    .any(|l| !l.trim().is_empty());
                if !has_message_content {
                    let draft = build_commit_message_draft(workspace_root, config)?;
                    let current = fs::read_to_string(&p).map_err(|e| {
                        QdevError::infrastructure_failure(
                            "commit_message_read_failed",
                            format!(
                                "Failed to reread commit message file '{}': {}",
                                p.display(),
                                e
                            ),
                        )
                    })?;
                    if current != existing {
                        return Err(QdevError::conflict(
                            "commit_message_changed",
                            format!(
                                "Commit message file '{}' changed while qdev prepared its draft",
                                p.display()
                            ),
                        ));
                    }
                    let mut content = if existing.is_empty() {
                        draft
                    } else {
                        format!("{}\n\n{}", draft, existing)
                    };
                    if !content.ends_with('\n') {
                        content.push('\n');
                    }
                    write_file_atomic(&p, &content)?;
                }
            }
        }
    }

    let hooks_dir = resolve_hooks_dir(workspace_root)?;
    run_legacy_hook(
        workspace_root,
        &hooks_dir,
        "prepare-commit-msg",
        args,
        false,
    )
}

fn git_comment_char(workspace_root: &Path) -> Result<char, QdevError> {
    let output =
        run_git(workspace_root, &["config", "--get", "core.commentChar"]).map_err(|e| {
            QdevError::infrastructure_failure(
                "git_unavailable",
                format!("Failed to read Git comment character: {}", e),
            )
        })?;
    if !output.status.success() {
        return Ok('#');
    }
    let configured = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if configured.is_empty() || configured == "auto" {
        return Ok('#');
    }
    configured.chars().next().ok_or_else(|| {
        QdevError::usage_error("Git core.commentChar must contain a comment character")
    })
}

fn build_commit_message_draft(workspace_root: &Path, config: &Config) -> Result<String, QdevError> {
    let format = config.commit_messages.format.as_str();
    if !matches!(format, "simple" | "conventional") {
        return Err(QdevError::usage_error(format!(
            "Invalid commit_messages.format '{}'; expected 'simple' or 'conventional'",
            format
        )));
    }

    let lease = find_active_lease_with_storage(workspace_root, Some(&config.storage))?;
    let (_, story_id, story_path) = resolve_entity_file(
        workspace_root,
        Some(EntityKind::Story),
        &lease.story_id,
        Some(&config.storage),
    )?;
    let story = fs::read_to_string(&story_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "leased_story_read_failed",
            format!(
                "Failed to read leased story '{}': {}",
                story_path.display(),
                e
            ),
        )
    })?;
    let frontmatter = extract_frontmatter(&story).map_err(|e| {
        QdevError::logical_failure(
            "leased_story_frontmatter_invalid",
            format!(
                "Failed to parse leased story '{}': {}",
                story_path.display(),
                e
            ),
        )
    })?;
    let title = frontmatter
        .get("title")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            QdevError::logical_failure(
                "leased_story_title_missing",
                format!(
                    "Leased story '{}' has no readable title",
                    story_path.display()
                ),
            )
        })?;
    if title.contains('\r') || title.contains('\n') {
        return Err(QdevError::logical_failure(
            "leased_story_title_multiline",
            format!(
                "Leased story '{}' has a multiline title",
                story_path.display()
            ),
        ));
    }
    let title = title.trim();
    if title.is_empty() {
        return Err(QdevError::logical_failure(
            "leased_story_title_missing",
            format!(
                "Leased story '{}' has no readable title",
                story_path.display()
            ),
        ));
    }

    let header = match format {
        "simple" => format!("{}: {}", story_id, title),
        "conventional" => format!("feat({}): {}", story_id, title),
        _ => unreachable!("format validated above"),
    };
    let entries = read_scratch_entries(workspace_root, Some(&config.storage), &story_id)?;
    let qualifying: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.kind == "decision" || entry.kind == "tradeoff")
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    if qualifying.is_empty() {
        return Ok(header);
    }

    let body = qualifying
        .iter()
        .map(|entry| render_scratchpad_bullet(&entry.text))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!("{}\n\n{}", header, body))
}

fn render_scratchpad_bullet(text: &str) -> String {
    let mut lines = text.lines();
    let first = lines.next().unwrap_or_default();
    let mut rendered = format!("- {}", first);
    for line in lines {
        rendered.push_str("\n  ");
        rendered.push_str(line);
    }
    rendered
}
