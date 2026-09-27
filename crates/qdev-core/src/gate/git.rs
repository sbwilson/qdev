//! Shared git plumbing for gate execution and workspace inspection.
//!
//! Every helper spawns `git` with the workspace root as current directory, so
//! callers stop re-deriving the same `Command::new("git").current_dir(...).output()`
//! sequence. Helpers return the raw `std::io::Result`/`Output` or a conservative
//! scalar, so each call site keeps its own error handling and exit-status
//! semantics. The path-listing helpers pass `-c core.quotePath=false` exactly
//! where the original call sites already did, so path output is never
//! C-style-quoted regardless of repository or locale configuration; helpers
//! that only read SHAs, refs, or counts pass the subcommand arguments
//! verbatim.

use std::path::Path;
use std::process::{Command, Output};

/// Runs `git <args>` in `workspace_root`.
///
/// A failed spawn (e.g. no git on PATH) is an `std::io::Error`; a non-zero git
/// exit is **not** an error here — callers inspect `Output::status`.
pub fn run_git(workspace_root: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
}

/// Runs `git <args>` in `workspace_root` with `-c core.quotePath=false` prepended,
/// so path output is never C-style quoted.
pub fn run_git_unquoted(workspace_root: &Path, args: &[&str]) -> std::io::Result<Output> {
    let mut git_args: Vec<&str> = Vec::with_capacity(args.len() + 2);
    git_args.push("-c");
    git_args.push("core.quotePath=false");
    git_args.extend_from_slice(args);
    Command::new("git")
        .args(&git_args)
        .current_dir(workspace_root)
        .output()
}

/// `git rev-parse HEAD`.
pub fn rev_parse_head(workspace_root: &Path) -> std::io::Result<Output> {
    run_git(workspace_root, &["rev-parse", "HEAD"])
}

/// `git rev-parse HEAD`: the current commit SHA, or `None` when the command
/// cannot run, exits non-zero, or prints nothing.
pub fn head_sha(workspace_root: &Path) -> Option<String> {
    let output = rev_parse_head(workspace_root).ok()?;
    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return Some(sha);
        }
    }
    None
}

/// `git rev-parse --is-inside-work-tree`: `true` when the command succeeds.
pub fn is_inside_work_tree(workspace_root: &Path) -> bool {
    run_git(workspace_root, &["rev-parse", "--is-inside-work-tree"])
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `git merge-base HEAD <refname>`.
pub fn merge_base(workspace_root: &Path, refname: &str) -> std::io::Result<Output> {
    run_git(workspace_root, &["merge-base", "HEAD", refname])
}

/// The merge-base commit of `HEAD` and `refname`, or `None` when the ref cannot
/// be resolved (command failure, non-zero exit, or empty output).
pub fn merge_base_commit(workspace_root: &Path, refname: &str) -> Option<String> {
    let output = merge_base(workspace_root, refname).ok()?;
    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return Some(sha);
        }
    }
    None
}

/// `git rev-parse --verify --quiet <refname>`: succeeds when the ref resolves.
pub fn verify_ref(workspace_root: &Path, refname: &str) -> std::io::Result<Output> {
    run_git(
        workspace_root,
        &["rev-parse", "--verify", "--quiet", refname],
    )
}

/// `true` when `refname` resolves in the workspace repository.
pub fn ref_exists(workspace_root: &Path, refname: &str) -> bool {
    verify_ref(workspace_root, refname)
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `git diff --name-only --relative <target>` (paths unquoted): changed files relative
/// to the workspace root.
pub fn diff_name_only_relative(workspace_root: &Path, target: &str) -> std::io::Result<Output> {
    run_git_unquoted(
        workspace_root,
        &["diff", "--name-only", "--relative", target],
    )
}

/// `git diff --cached --name-only --relative` (paths unquoted): staged files relative
/// to the workspace root.
pub fn diff_cached_name_only(workspace_root: &Path) -> std::io::Result<Output> {
    run_git_unquoted(
        workspace_root,
        &["diff", "--cached", "--name-only", "--relative"],
    )
}

/// `git status --porcelain=v1 -uall` (paths unquoted): every changed, staged, or
/// untracked path, one per line.
pub fn status_porcelain_untracked_all(workspace_root: &Path) -> std::io::Result<Output> {
    run_git_unquoted(workspace_root, &["status", "--porcelain=v1", "-uall"])
}

/// The file paths in name-only git output: each line trimmed, empty lines dropped.
pub fn name_only_paths(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}
