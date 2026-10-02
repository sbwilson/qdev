//! CLI handler for `qdev install hooks`.

use std::path::Path;

use qdev_core::{
    find_workspace_root, install_hooks, install_mcp, install_skills_configured, ExitCode,
    JsonEnvelope, QdevError, SkillInstallOptions,
};

use crate::cli;
use crate::output::OutputEmitter;
use crate::Cli;

pub fn handle_install(
    install_args: &cli::InstallArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
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
        cli::InstallCommands::Skills(ref skills_args) => {
            let root = find_workspace_root(current_dir);
            let options = SkillInstallOptions {
                claude: skills_args.claude,
                cursor: skills_args.cursor,
                agents: skills_args.agents,
            };
            match install_skills_configured(
                &root,
                &options,
                Some(&annotated_config.config.models),
                Some(&annotated_config.config.synthesis),
            ) {
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
                            "Installed {} skill/rule file(s) across target(s) [{}]:\n",
                            report.installed_files.len(),
                            report.targets.join(", ")
                        );
                        for file in &report.installed_files {
                            text.push_str(&format!("  - {}\n", file));
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
        cli::InstallCommands::Mcp(ref mcp_args) => {
            let root = find_workspace_root(current_dir);
            match install_mcp(&root, mcp_args.claude, mcp_args.cursor) {
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
                            "Installed MCP server configuration across target(s) [{}]:\n",
                            report.targets.join(", ")
                        );
                        for file in &report.installed_files {
                            text.push_str(&format!("  - {}\n", file));
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
