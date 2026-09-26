//! CLI handler for `qdev install hooks`.

use std::path::Path;

use qdev_core::{find_workspace_root, install_hooks, ExitCode, JsonEnvelope, QdevError};

use crate::cli;
use crate::output::OutputEmitter;
use crate::Cli;

pub fn handle_install(
    install_args: &cli::InstallArgs,
    _annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &Path,
) -> ExitCode {
    match install_args.command {
        cli::InstallCommands::Hooks(_) => {
            let root = find_workspace_root(current_dir);
            match install_hooks(&root) {
                Ok(report) => {
                    if cli.json {
                        let envelope = JsonEnvelope::new(report);
                        if let Err(e) = output.emit_envelope(&envelope) {
                            let err = QdevError::infrastructure_failure(
                                "io_error",
                                format!("Failed to emit install envelope: {}", e),
                            );
                            let _ = output.emit_error(&err);
                            return ExitCode::InfrastructureFailure;
                        }
                    } else {
                        let mut text = format!(
                            "Installed {} git hook(s) in {}: {}\n",
                            report.installed_hooks.len(),
                            report.hooks_dir,
                            report.installed_hooks.join(", ")
                        );
                        for legacy in &report.preserved_legacy {
                            text.push_str(&format!(
                                "Preserved non-qdev hook: {} -> {}.legacy\n",
                                legacy, legacy
                            ));
                        }
                        if let Err(e) = output.emit_text(&text) {
                            let err = QdevError::infrastructure_failure(
                                "io_error",
                                format!("Failed to emit install output: {}", e),
                            );
                            let _ = output.emit_error(&err);
                            return ExitCode::InfrastructureFailure;
                        }
                    }
                    ExitCode::Success
                }
                Err(e) => {
                    let _ = output.emit_error(&e);
                    e.exit_code()
                }
            }
        }
    }
}
