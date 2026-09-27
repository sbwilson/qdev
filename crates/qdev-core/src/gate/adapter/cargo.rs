use std::path::Path;

use crate::gate::{GateFailure, GateStatus};

/// Helper to normalize and relativize source locations to `file:line`.
/// Parses from the right to avoid misidentifying Windows drive letters as colons.
pub fn normalize_location(raw_path_with_line_col: &str, workspace_root: Option<&Path>) -> String {
    let s = raw_path_with_line_col.trim().trim_end_matches(':');
    let parts: Vec<&str> = s.split(':').collect();

    if parts.len() >= 3
        && parts[parts.len() - 1].chars().all(|c| c.is_ascii_digit())
        && !parts[parts.len() - 1].is_empty()
        && parts[parts.len() - 2].chars().all(|c| c.is_ascii_digit())
        && !parts[parts.len() - 2].is_empty()
    {
        // e.g. path:line:col or C:\path\file.rs:line:col
        let path = parts[..parts.len() - 2].join(":");
        let line = parts[parts.len() - 2];
        let rel_path = relativize_path(&path, workspace_root);
        format!("{}:{}", rel_path, line)
    } else if parts.len() >= 2
        && parts[parts.len() - 1].chars().all(|c| c.is_ascii_digit())
        && !parts[parts.len() - 1].is_empty()
    {
        // e.g. path:line or C:\path\file.rs:line
        let path = parts[..parts.len() - 1].join(":");
        let line = parts[parts.len() - 1];
        let rel_path = relativize_path(&path, workspace_root);
        format!("{}:{}", rel_path, line)
    } else {
        relativize_path(s, workspace_root)
    }
}

/// Relativizes an absolute path against `workspace_root` if possible, checking boundary separators.
pub fn relativize_path(path_str: &str, workspace_root: Option<&Path>) -> String {
    if let Some(root) = workspace_root {
        let p = Path::new(path_str);
        if let Ok(rel) = p.strip_prefix(root) {
            return rel.to_string_lossy().to_string();
        }

        let p_str = path_str;
        let root_str = root.to_string_lossy();

        let p_trimmed = p_str.strip_prefix("/private").unwrap_or(p_str);
        let root_trimmed = root_str.strip_prefix("/private").unwrap_or(&root_str);

        if let Some(rel) = p_trimmed.strip_prefix(root_trimmed) {
            if root_trimmed.ends_with('/')
                || root_trimmed.ends_with('\\')
                || rel.starts_with('/')
                || rel.starts_with('\\')
                || rel.is_empty()
            {
                return rel.trim_start_matches(['/', '\\']).to_string();
            }
        }

        if let (Ok(p_canon), Ok(root_canon)) = (p.canonicalize(), root.canonicalize()) {
            if let Ok(rel) = p_canon.strip_prefix(&root_canon) {
                return rel.to_string_lossy().to_string();
            }
        }
    }
    path_str.to_string()
}

/// Parses the target portion following `panicked at ` in Cargo output.
/// Returns (location, optional_inline_message).
fn parse_cargo_panic_target(
    after_panic: &str,
    workspace_root: Option<&Path>,
) -> (String, Option<String>) {
    let trimmed = after_panic.trim();
    if trimmed.starts_with('\'') {
        // e.g. 'assertion failed: left == right', crates/foo/bar.rs:55:13
        // Search for closing delimiter `', ` or `',` to support messages with internal apostrophes
        if let Some(close_idx) = trimmed.rfind("', ") {
            let msg = &trimmed[1..close_idx];
            let loc_part = trimmed[close_idx + 3..].trim();
            let loc = normalize_location(loc_part, workspace_root);
            return (loc, Some(msg.to_string()));
        } else if let Some(close_idx) = trimmed.rfind("',") {
            let msg = &trimmed[1..close_idx];
            let loc_part = trimmed[close_idx + 2..].trim();
            let loc = normalize_location(loc_part, workspace_root);
            return (loc, Some(msg.to_string()));
        }
    }

    // Do not split on whitespace so paths with spaces are preserved
    let loc_token = trimmed.trim_end_matches(':').trim();
    (normalize_location(loc_token, workspace_root), None)
}

/// Parses Cargo test outputs (stdout/stderr) and synthesizes status, summary, and failures.
pub fn parse_cargo_output(
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    workspace_root: Option<&Path>,
) -> (GateStatus, String, Vec<GateFailure>) {
    let combined = format!("{}\n{}", stdout, stderr);
    let mut failures = Vec::new();
    let lines: Vec<&str> = combined.lines().collect();

    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut found_summary = false;
    let mut in_failures_section = false;

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        // Track failures: section header to ignore expected panics from #[should_panic] tests
        if line.trim() == "failures:" {
            in_failures_section = true;
        }

        // Parse summary lines: test result: FAILED. 17 passed; 1 failed; ...
        if let Some(result_idx) = line.find("test result:") {
            let after = &line[result_idx + "test result:".len()..];
            let mut p = 0usize;
            let mut f = 0usize;

            for part in after.split(';') {
                let part_trim = part.trim();
                if let Some(passed_idx) = part_trim.find("passed") {
                    let num_str = part_trim[..passed_idx]
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect::<String>();
                    if let Ok(num) = num_str.parse::<usize>() {
                        p = num;
                    }
                }
                if let Some(failed_idx) = part_trim.find("failed") {
                    let num_str = part_trim[..failed_idx]
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect::<String>();
                    if let Ok(num) = num_str.parse::<usize>() {
                        f = num;
                    }
                }
            }

            total_passed += p;
            total_failed += f;
            found_summary = true;
        }

        // Parse panic lines only inside failures: section
        if in_failures_section {
            if let Some(panic_idx) = line.find("panicked at ") {
                let after_panic = &line[panic_idx + "panicked at ".len()..];
                let (location, inline_msg) = parse_cargo_panic_target(after_panic, workspace_root);

                let mut msg_lines = Vec::new();
                if let Some(msg) = inline_msg {
                    msg_lines.push(msg);
                }

                i += 1;
                while i < lines.len() {
                    let next_line = lines[i];
                    let trimmed = next_line.trim();
                    if trimmed.is_empty()
                        || trimmed.starts_with("note:")
                        || trimmed.starts_with("---- ")
                        || trimmed.starts_with("failures:")
                        || trimmed.starts_with("test result:")
                    {
                        break;
                    }
                    msg_lines.push(next_line.to_string());
                    i += 1;
                }

                let message = msg_lines.join("\n").trim().to_string();
                failures.push(GateFailure { location, message });
                continue;
            }
        }

        i += 1;
    }

    let status = if exit_code == 0 && failures.is_empty() && total_failed == 0 {
        GateStatus::Pass
    } else {
        GateStatus::Fail
    };

    let summary = if exit_code != 0 && total_failed == 0 && failures.is_empty() {
        format!("cargo test exited with code {}", exit_code)
    } else if found_summary {
        if total_failed > 0 {
            format!(
                "{} of {} tests failed",
                total_failed,
                total_passed + total_failed
            )
        } else if total_passed > 0 {
            format!("{} tests passed", total_passed)
        } else {
            "tests passed".to_string()
        }
    } else if !failures.is_empty() {
        format!("{} of {} tests failed", failures.len(), failures.len())
    } else if exit_code == 0 {
        "tests passed".to_string()
    } else {
        format!("cargo test exited with code {}", exit_code)
    };

    (status, summary, failures)
}
