mod cli;
mod output;

use std::io::IsTerminal;
use std::panic;
use std::process::ExitCode as StdExitCode;

use clap::Parser;
use qdev_core::{ExitCode, Interactivity, JsonEnvelope, JsonErrorEnvelope, QdevError};
use serde::Serialize;

use cli::{is_json_requested, Cli, Commands, ConfigCommands};
use output::OutputEmitter;

#[derive(Serialize)]
struct VersionPayload {
    version: String,
}

fn main() -> StdExitCode {
    let raw_args: Vec<String> = std::env::args().collect();
    let json_mode = is_json_requested(&raw_args);

    if json_mode {
        // Set a panic hook that emits an AD-13 error envelope on stdout and avoids unformatted panic traces
        panic::set_hook(Box::new(|panic_info| {
            let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Internal panic occurred".to_string()
            };

            let location_details = panic_info.location().map(|loc| {
                serde_json::json!({
                    "file": loc.file(),
                    "line": loc.line(),
                    "column": loc.column(),
                })
            });

            let envelope = JsonErrorEnvelope::new(
                "infrastructure_failure",
                format!("Internal error: {}", message),
                location_details,
            );

            let _ = OutputEmitter::emit_error_envelope(&envelope);
        }));
    }

    let result = panic::catch_unwind(|| run(&raw_args));

    let exit_code = match result {
        Ok(code) => code,
        Err(_) => ExitCode::InfrastructureFailure,
    };

    StdExitCode::from(exit_code)
}

fn run(raw_args: &[String]) -> ExitCode {
    let json_mode = is_json_requested(raw_args);
    let output = OutputEmitter::new(json_mode);

    let cli = match Cli::try_parse_from(raw_args) {
        Ok(cli) => cli,
        Err(clap_err) => {
            if clap_err.kind() == clap::error::ErrorKind::DisplayHelp {
                print!("{}", clap_err);
                return ExitCode::Success;
            }

            let err_msg = clap_err.to_string();
            let qdev_err = QdevError::usage_error(err_msg.trim());
            let _ = output.emit_error(&qdev_err);
            return ExitCode::UsageError;
        }
    };

    // Handle --version flag
    if cli.version {
        if cli.json {
            let envelope = JsonEnvelope::new(VersionPayload {
                version: env!("CARGO_PKG_VERSION").to_string(),
            });
            if let Err(e) = output.emit_envelope(&envelope) {
                let err = QdevError::infrastructure_failure(
                    "io_error",
                    format!("Failed to emit version envelope: {}", e),
                );
                let _ = output.emit_error(&err);
                return ExitCode::InfrastructureFailure;
            }
        } else {
            println!("qdev {}", env!("CARGO_PKG_VERSION"));
        }
        return ExitCode::Success;
    }

    // Resolve interactivity per AD-12
    let env_var = std::env::var("QDEV_NONINTERACTIVE").ok();
    let is_stdin_tty = std::io::stdin().is_terminal();
    let interactivity =
        Interactivity::resolve(cli.non_interactive, env_var.as_deref(), is_stdin_tty);

    // Boot-time configuration loading per spec-1-2
    let current_dir = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            let qdev_err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to determine current working directory: {}", e),
            );
            let _ = output.emit_error(&qdev_err);
            return ExitCode::InfrastructureFailure;
        }
    };

    let annotated_config = match qdev_core::load_config(&current_dir) {
        Ok(cfg) => cfg,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Dispatch commands
    match cli.command {
        None | Some(Commands::Status) => {
            let status = qdev_core::get_pulse_status(interactivity);
            if cli.json {
                let envelope = JsonEnvelope::new(status);
                if let Err(e) = output.emit_envelope(&envelope) {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to emit status envelope: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            } else {
                println!("qdev {}", env!("CARGO_PKG_VERSION"));
            }
            ExitCode::Success
        }
        Some(Commands::Config(config_args)) => match config_args.command {
            ConfigCommands::Show => {
                if cli.json {
                    let envelope = JsonEnvelope::new(annotated_config);
                    if let Err(e) = output.emit_envelope(&envelope) {
                        let err = QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to emit config envelope: {}", e),
                        );
                        let _ = output.emit_error(&err);
                        return ExitCode::InfrastructureFailure;
                    }
                } else {
                    let report = annotated_config.to_text_report();
                    if let Err(e) = output.emit_text(&report) {
                        let err = QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to emit config report: {}", e),
                        );
                        let _ = output.emit_error(&err);
                        return ExitCode::InfrastructureFailure;
                    }
                }
                ExitCode::Success
            }
        },
    }
}
