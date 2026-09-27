use std::path::Path;

use crate::gate::adapter::cargo::normalize_location;
use crate::gate::{GateFailure, GateStatus};

/// Parses xcodebuild test outputs (stdout/stderr) and synthesizes status, summary, and failures.
pub fn parse_xcodebuild_output(
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    workspace_root: Option<&Path>,
) -> (GateStatus, String, Vec<GateFailure>) {
    let combined = format!("{}\n{}", stdout, stderr);
    let mut failures = Vec::new();

    let mut total_executed = 0usize;
    let mut total_failures = 0usize;
    let mut found_summary = false;

    for line in combined.lines() {
        // Parse summary line: Executed 18 tests, with 1 failure ...
        // Use the final Executed summary line to avoid double-counting across nested test suites
        if let Some(exec_idx) = line.find("Executed ") {
            let after_exec = &line[exec_idx + "Executed ".len()..];
            if let Some(tests_idx) = after_exec.find(" test") {
                let count_str = after_exec[..tests_idx]
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect::<String>();
                if let Ok(tests) = count_str.parse::<usize>() {
                    let mut fails = 0usize;
                    if let Some(with_idx) = after_exec.find("with ") {
                        let after_with = &after_exec[with_idx + "with ".len()..];
                        if let Some(fail_idx) = after_with.find(" failure") {
                            let fail_count_str = after_with[..fail_idx]
                                .chars()
                                .filter(|c| c.is_ascii_digit())
                                .collect::<String>();
                            if let Ok(f) = fail_count_str.parse::<usize>() {
                                fails = f;
                            }
                        }
                    }
                    total_executed = tests;
                    total_failures = fails;
                    found_summary = true;
                }
            }
        }

        // Parse error line: <path>:<line>: error: <assertion>
        if let Some(err_idx) = line.find(": error:") {
            let loc_part = &line[..err_idx];
            let location = normalize_location(loc_part, workspace_root);

            let raw_msg = line[err_idx + ": error:".len()..].trim();
            // If message contains -[TestClass testMethod] : <assertion>, extract the assertion
            let message = if let Some(bracket_colon_idx) = raw_msg.find("] : ") {
                raw_msg[bracket_colon_idx + "] : ".len()..]
                    .trim()
                    .to_string()
            } else {
                raw_msg.to_string()
            };

            failures.push(GateFailure { location, message });
        }
    }

    let status = if exit_code == 0 && failures.is_empty() && total_failures == 0 {
        GateStatus::Pass
    } else {
        GateStatus::Fail
    };

    let summary = if exit_code != 0 && total_failures == 0 && failures.is_empty() {
        format!("xcodebuild test exited with code {}", exit_code)
    } else if found_summary {
        if total_failures > 0 {
            format!("{} of {} tests failed", total_failures, total_executed)
        } else if total_executed > 0 {
            format!("{} tests passed", total_executed)
        } else {
            "tests passed".to_string()
        }
    } else if !failures.is_empty() {
        format!("{} of {} tests failed", failures.len(), failures.len())
    } else if exit_code == 0 {
        "tests passed".to_string()
    } else {
        format!("xcodebuild test exited with code {}", exit_code)
    };

    (status, summary, failures)
}
