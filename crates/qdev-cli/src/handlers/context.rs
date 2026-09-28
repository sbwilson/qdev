//! `qdev context` handler (Story 4.1): thin CLI wiring around the pure core projection.
//!
//! The handler mirrors `handle_get`'s boot sequence — workspace guard (shared, in `run`),
//! open the query store, call the core, emit through the `OutputEmitter` — and adds the
//! output-mode rules: `--json` emits the envelope, `--format md` emits the deterministic
//! Markdown rendering, and the two together are a usage error (exit 2). The projection
//! itself is read-only: no cache writes, no file writes, no git mutations, no prompts.

use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::open_query_store;
use qdev_core::{ExitCode, JsonEnvelope, QdevError};

pub fn handle_context(
    context_args: &cli::ContextArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    // Conflicting output modes: --json and --format together name two renderings of one
    // payload — refuse rather than guess.
    if cli.json && context_args.format.is_some() {
        let err = QdevError::usage_error(
            "`--json` and `--format` are mutually exclusive — pass one output mode, not both",
        );
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // `--format` accepts exactly one value: `md` (the default text rendering is the fallback).
    if let Some(format) = context_args.format.as_deref() {
        if format.trim().to_lowercase() != "md" {
            let err = QdevError::usage_error(format!(
                "Unknown --format value '{}', expected 'md'",
                format
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    }

    let root = qdev_core::find_workspace_root(current_dir);

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let phase = match qdev_core::ContextPhase::from_str_loose(&context_args.phase) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::ContextOptions {
        phase,
        budget: context_args.budget,
        stats: context_args.stats,
    };

    let payload = match qdev_core::build_context(
        &root,
        &store,
        &annotated_config.config,
        &options,
        &context_args.id,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(payload);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit context envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else if context_args.format.is_some() {
        let text = qdev_core::render_context_markdown(&payload);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit context output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = qdev_core::render_context_text(&payload);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit context output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
