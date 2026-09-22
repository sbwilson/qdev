use std::path::Path;

use qdev_core::config::AnnotatedConfig;
use qdev_core::{execute_gate, ExitCode, GateRunOptions, JsonEnvelope};

use crate::cli::{Cli, GateArgs, GateCommands};
use crate::output::OutputEmitter;

/// Handles `qdev gate` subcommands.
pub fn handle_gate(
    args: &GateArgs,
    annotated_config: &AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    workspace_root: &Path,
) -> ExitCode {
    match &args.command {
        GateCommands::Run(run_args) => {
            let options = GateRunOptions {
                story: run_args.story.clone(),
                timeout_ms: None,
            };

            let outcome = match execute_gate(
                workspace_root,
                &annotated_config.config,
                &run_args.id,
                &options,
            ) {
                Ok(outcome) => outcome,
                Err(err) => {
                    let _ = output.emit_error(&err);
                    return err.exit_code();
                }
            };

            if cli.json {
                let envelope = JsonEnvelope::new(outcome.to_payload());
                let _ = output.emit_envelope(&envelope);
            } else {
                let _ = output.emit_text(&outcome.receipt());
            }

            outcome.cli_exit_code()
        }
    }
}
