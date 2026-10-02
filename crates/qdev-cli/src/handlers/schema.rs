//! Schema inspection handler: prints entity or output payload JSON schemas.

use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use qdev_core::{ExitCode, JsonEnvelope, QdevError};

pub fn handle_schema(schema_args: &cli::SchemaArgs, cli: &Cli, output: &OutputEmitter) -> ExitCode {
    if schema_args.kind.trim().eq_ignore_ascii_case("payload") {
        return handle_schema_payload(schema_args, cli, output);
    }

    if let Some(extra) = &schema_args.name {
        let e = QdevError::usage_error(format!(
            "Unexpected extra argument '{}'. Usage: qdev schema <entity-kind>",
            extra
        ));
        let _ = output.emit_error(&e);
        return ExitCode::UsageError;
    }

    let kind = match qdev_core::EntityKind::from_str_loose(&schema_args.kind) {
        Ok(k) => k,
        Err(e) => {
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(kind.schema_json());
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit schema envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let schema_text = kind.pretty_schema_str();
        if let Err(e) = output.emit_text(&schema_text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit schema: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

/// Handles `qdev schema payload <name>`: prints a hand-authored JSON Schema for a CLI output
/// payload shape, resolved via `PayloadKind` — entirely separate from the `EntityKind` path above.
pub fn handle_schema_payload(
    schema_args: &cli::SchemaArgs,
    cli: &Cli,
    output: &OutputEmitter,
) -> ExitCode {
    let name = match &schema_args.name {
        Some(n) => n,
        None => {
            let e = QdevError::usage_error(format!(
                "Missing payload name. Usage: qdev schema payload <name>. Valid payload names: {}",
                qdev_core::PayloadKind::valid_names()
            ));
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    };

    let payload = match qdev_core::PayloadKind::from_str_loose(name) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(payload.schema_json());
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit payload schema envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let schema_text = payload.pretty_schema_str();
        if let Err(e) = output.emit_text(&schema_text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit payload schema: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
