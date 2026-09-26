//! CLI handler for `qdev hygiene`.
//!
//! Provides comment hygiene linting, JSON/text reporting, and diff support.

use std::path::Path;

use qdev_core::{
    check_hygiene, find_workspace_root, ExitCode, HygieneCheckOptions, JsonEnvelope, QdevError,
};

use crate::cli;
use crate::output::OutputEmitter;
use crate::Cli;

pub fn handle_hygiene(
    hygiene_args: &cli::HygieneArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &Path,
) -> ExitCode {
    match hygiene_args.command {
        cli::HygieneCommands::Check(ref check_args) => {
            // --fix exits 2 (ExitCode::UsageError) with a message that auto-fix is deferred
            if check_args.fix {
                let err = QdevError::usage_error("Automatic fixing with --fix is deferred to v2");
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }

            let workspace_root = find_workspace_root(current_dir);

            let options = HygieneCheckOptions {
                diff: check_args.diff,
                paths: check_args.paths.clone(),
            };

            let outcome = match check_hygiene(&workspace_root, &annotated_config.config, &options) {
                Ok(o) => o,
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
            };

            if cli.json {
                let envelope = JsonEnvelope::new(outcome.clone());
                let _ = output.emit_envelope(&envelope);
            } else {
                if outcome.findings.is_empty() {
                    let _ = output.emit_text("Hygiene check: 0 findings");
                } else {
                    let mut text = String::new();
                    for finding in &outcome.findings {
                        text.push_str(&format!(
                            "{}: [{}] {}\n",
                            finding.location, finding.rule_id, finding.excerpt
                        ));
                    }
                    let count_str = match outcome.findings.len() {
                        1 => "1 finding".to_string(),
                        n => format!("{} findings", n),
                    };
                    text.push_str(&format!("\nHygiene check: {}", count_str));
                    let _ = output.emit_text(text.trim_end_matches('\n'));
                }
            }

            if outcome.findings.is_empty() {
                ExitCode::Success
            } else {
                ExitCode::LogicalFailure
            }
        }
    }
}
