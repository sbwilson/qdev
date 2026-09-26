//! Hygiene module root, types, and check_hygiene orchestrator.
//!
//! Provides language-aware comment linting and reporting for Rust, Swift, and Python.

pub mod linter;
pub mod tokenizer;

use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::errors::QdevError;

pub use linter::{
    lint_comments, HygieneFinding, HygieneLinter, RULE_FORBID_PATTERNS,
    RULE_MAX_INLINE_COMMENT_LINES, RULE_REVIEW_ROUND, RULE_STORY_BANNER,
};
pub use tokenizer::{
    tokenize_comments, CommentKind, CommentLine, CommentToken, SupportedLanguage,
};

/// Structured output of a hygiene check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HygieneCheckOutcome {
    pub status: String,
    pub summary: String,
    pub findings: Vec<HygieneFinding>,
}

/// Execution options for hygiene check.
#[derive(Debug, Clone, Default)]
pub struct HygieneCheckOptions {
    pub diff: bool,
    pub paths: Vec<String>,
}

/// Runs comment hygiene inspection across target files or git diff changes.
pub fn check_hygiene(
    workspace_root: &Path,
    config: &Config,
    options: &HygieneCheckOptions,
) -> Result<HygieneCheckOutcome, QdevError> {
    // If hygiene is disabled, exit 0 with 0 findings immediately
    if !config.hygiene.enabled {
        return Ok(HygieneCheckOutcome {
            status: "pass".to_string(),
            summary: "Hygiene check: 0 findings".to_string(),
            findings: Vec::new(),
        });
    }

    let linter = HygieneLinter::new(&config.hygiene)?;

    // Resolve target candidate files
    let target_files = if options.diff {
        let diff_files = resolve_diff_files(workspace_root, &config.git.integration_branch)?;
        if !options.paths.is_empty() {
            let filter_paths = resolve_explicit_paths(workspace_root, &options.paths)?;
            diff_files
                .into_iter()
                .filter(|df| {
                    filter_paths
                        .iter()
                        .any(|fp| df == fp || df.starts_with(fp))
                })
                .collect()
        } else {
            diff_files
        }
    } else if !options.paths.is_empty() {
        resolve_explicit_paths(workspace_root, &options.paths)?
    } else {
        collect_workspace_source_files(workspace_root)?
    };

    let mut all_findings = Vec::new();

    for file_path in target_files {
        if !file_path.is_file() {
            continue;
        }

        // Check if file language is enabled in config
        let Some(lang) = SupportedLanguage::from_path(&file_path) else {
            continue;
        };

        let is_lang_enabled = config
            .hygiene
            .languages
            .iter()
            .any(|l| SupportedLanguage::from_name(l) == Some(lang));

        if !is_lang_enabled {
            continue;
        }

        let content = match fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => continue, // Skip unreadable/binary files
        };

        let rel_path = file_path
            .strip_prefix(workspace_root)
            .unwrap_or(&file_path)
            .to_string_lossy()
            .replace('\\', "/");

        let tokens = tokenize_comments(&content, lang);
        let findings = linter.lint(&rel_path, &tokens);
        all_findings.extend(findings);
    }

    // Sort findings deterministically: file, line, rule_id
    all_findings.sort_by(|a, b| {
        (&a.file, a.line, &a.rule_id).cmp(&(&b.file, b.line, &b.rule_id))
    });

    let status = if all_findings.is_empty() {
        "pass"
    } else {
        "fail"
    };

    let count_str = match all_findings.len() {
        0 => "0 findings".to_string(),
        1 => "1 finding".to_string(),
        n => format!("{} findings", n),
    };
    let summary = format!("Hygiene check: {}", count_str);

    Ok(HygieneCheckOutcome {
        status: status.to_string(),
        summary,
        findings: all_findings,
    })
}

fn resolve_diff_files(
    workspace_root: &Path,
    integration_branch: &str,
) -> Result<Vec<PathBuf>, QdevError> {
    let is_git = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git {
        return Err(QdevError::infrastructure_failure(
            "not_in_git_worktree",
            "Cannot run hygiene check --diff: not inside a git work tree",
        ));
    }

    let mut changed = std::collections::HashSet::new();

    // 1. Try merge-base with integration_branch
    let mb_output = std::process::Command::new("git")
        .args(["merge-base", "HEAD", integration_branch])
        .current_dir(workspace_root)
        .output();

    let mut diff_target = None;
    if let Ok(ref out) = mb_output {
        if out.status.success() {
            let mb = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !mb.is_empty() {
                diff_target = Some(mb);
            }
        }
    }

    // Fallback: integration_branch ref directly
    if diff_target.is_none() {
        let verify = std::process::Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", integration_branch])
            .current_dir(workspace_root)
            .output();
        if let Ok(ref out) = verify {
            if out.status.success() {
                diff_target = Some(integration_branch.to_string());
            }
        }
    }

    if let Some(target) = diff_target {
        let diff_out = std::process::Command::new("git")
            .args(["-c", "core.quotePath=false", "diff", "--name-only", "--relative", &target])
            .current_dir(workspace_root)
            .output()
            .map_err(|e| {
                QdevError::infrastructure_failure(
                    "git_diff_failed",
                    format!("Failed to execute git diff: {}", e),
                )
            })?;

        if !diff_out.status.success() {
            return Err(QdevError::infrastructure_failure(
                "git_diff_failed",
                format!(
                    "git diff failed: {}",
                    String::from_utf8_lossy(&diff_out.stderr).trim()
                ),
            ));
        }

        for line in String::from_utf8_lossy(&diff_out.stdout).lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                changed.insert(trimmed.to_string());
            }
        }
    }

    // Include uncommitted changes (staged and unstaged + untracked)
    let status_out = std::process::Command::new("git")
        .args(["-c", "core.quotePath=false", "status", "--porcelain=v1", "-uall"])
        .current_dir(workspace_root)
        .output()
        .map_err(|e| {
            QdevError::infrastructure_failure(
                "git_status_failed",
                format!("Failed to execute git status: {}", e),
            )
        })?;

    if !status_out.status.success() {
        return Err(QdevError::infrastructure_failure(
            "git_status_failed",
            format!(
                "git status failed: {}",
                String::from_utf8_lossy(&status_out.stderr).trim()
            ),
        ));
    }

    for line in String::from_utf8_lossy(&status_out.stdout).lines() {
        if line.len() >= 3 {
            let path = line[3..].trim();
            let path = if let Some(idx) = path.find(" -> ") {
                &path[idx + 4..]
            } else {
                path
            };
            if !path.is_empty() {
                changed.insert(path.to_string());
            }
        }
    }

    let result: Vec<PathBuf> = changed
        .into_iter()
        .map(|p| workspace_root.join(p))
        .collect();

    Ok(result)
}

fn resolve_explicit_paths(
    workspace_root: &Path,
    paths: &[String],
) -> Result<Vec<PathBuf>, QdevError> {
    let mut files = Vec::new();

    for p in paths {
        let path = if Path::new(p).is_absolute() {
            PathBuf::from(p)
        } else {
            workspace_root.join(p)
        };

        if !path.exists() {
            return Err(QdevError::usage_error(format!("Path not found: {}", p)));
        }

        if path.is_file() {
            files.push(path);
        } else if path.is_dir() {
            collect_dir_files(&path, &mut files)?;
        }
    }

    Ok(files)
}

fn collect_dir_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), QdevError> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries.flatten() {
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        // Skip directory symlinks to prevent cycles
        if file_type.is_symlink() {
            continue;
        }

        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if name_str.starts_with('.') || name_str == "target" || name_str == "node_modules" {
            continue;
        }

        if file_type.is_dir() {
            collect_dir_files(&path, out)?;
        } else if file_type.is_file() && SupportedLanguage::from_path(&path).is_some() {
            out.push(path);
        }
    }

    Ok(())
}

fn collect_workspace_source_files(workspace_root: &Path) -> Result<Vec<PathBuf>, QdevError> {
    let mut files = Vec::new();
    collect_dir_files(workspace_root, &mut files)?;
    Ok(files)
}
