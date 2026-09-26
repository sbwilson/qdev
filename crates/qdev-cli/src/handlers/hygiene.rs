//! CLI handler for `qdev hygiene`.
//!
//! Provides a stub implementation for Story 3.8 until full comment linting
//! is introduced in Story 3.9.

use std::path::Path;

use qdev_core::{ExitCode, JsonEnvelope};
use serde::{Deserialize, Serialize};

use crate::cli;
use crate::output::OutputEmitter;
use crate::Cli;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HygienePayload {
    pub status: String,
    pub findings: Vec<String>,
}

pub fn handle_hygiene(
    hygiene_args: &cli::HygieneArgs,
    _annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    _current_dir: &Path,
) -> ExitCode {
    match hygiene_args.command {
        cli::HygieneCommands::Check(ref _check_args) => {
            if cli.json {
                let payload = HygienePayload {
                    status: "pass".to_string(),
                    findings: Vec::new(),
                };
                let envelope = JsonEnvelope::new(payload);
                let _ = output.emit_envelope(&envelope);
            } else {
                let _ = output.emit_text("Hygiene check: 0 findings\n");
            }
            ExitCode::Success
        }
    }
}
