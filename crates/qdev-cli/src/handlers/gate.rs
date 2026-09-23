use std::path::Path;

use qdev_core::config::AnnotatedConfig;
use qdev_core::{
    execute_gate_set, get_gate_list, resolve_gate_execution_order, ExitCode, GateListPayload,
    GateRunOptions, JsonEnvelope, QdevError,
};

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
        GateCommands::List(_) => {
            let items = match get_gate_list(workspace_root, &annotated_config.config) {
                Ok(items) => items,
                Err(err) => {
                    let _ = output.emit_error(&err);
                    return err.exit_code();
                }
            };

            if cli.json {
                let payload = GateListPayload { gates: items };
                let envelope = JsonEnvelope::new(payload);
                let _ = output.emit_envelope(&envelope);
            } else {
                let mut lines = Vec::new();
                for gate in &items {
                    let trans = gate.transitions.join(", ");
                    let deps = gate.dependencies.join(", ");
                    let last = gate.last_status.as_deref().unwrap_or("none");
                    lines.push(format!(
                        "{} (kind: {}, transitions: [{}], depends_on: [{}], last_status: {})",
                        gate.id, gate.kind, trans, deps, last
                    ));
                }
                let _ = output.emit_text(&lines.join("\n"));
            }

            ExitCode::Success
        }
        GateCommands::Run(run_args) => {
            let id_present = run_args.id.is_some();
            let all_present = run_args.all;
            let trans_present = run_args.for_transition.is_some();

            let count = (id_present as usize) + (all_present as usize) + (trans_present as usize);
            if count > 1 {
                let err = QdevError::usage_error(
                    "conflicting invocation: specify only one of gate ID, --all, or --for-transition",
                );
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }
            if count == 0 {
                let err = QdevError::usage_error(
                    "must specify a gate ID, --all, or --for-transition",
                );
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }

            let options = GateRunOptions {
                story: run_args.story.clone(),
                timeout_ms: None,
            };

            let target_ids: Option<Vec<String>> = if let Some(ref gate_id) = run_args.id {
                Some(vec![gate_id.clone()])
            } else if let Some(ref trans) = run_args.for_transition {
                let matching: Vec<String> = annotated_config
                    .config
                    .gates
                    .iter()
                    .filter(|g| g.on_transition.contains(trans))
                    .map(|g| g.id.clone())
                    .collect();
                if matching.is_empty() {
                    let err = QdevError::usage_error(format!(
                        "no gates configured for transition '{}'",
                        trans
                    ));
                    let _ = output.emit_error(&err);
                    return ExitCode::UsageError;
                }
                Some(matching)
            } else {
                None
            };

            let execution_order = match resolve_gate_execution_order(
                &annotated_config.config.gates,
                target_ids.as_deref(),
            ) {
                Ok(order) => order,
                Err(err) => {
                    let _ = output.emit_error(&err);
                    return err.exit_code();
                }
            };

            let set_outcome = match execute_gate_set(
                workspace_root,
                &annotated_config.config,
                &execution_order,
                &options,
            ) {
                Ok(outcome) => outcome,
                Err(err) => {
                    let _ = output.emit_error(&err);
                    return err.exit_code();
                }
            };

            if cli.json {
                if let Some(ref target_id) = run_args.id {
                    if let Some(target_outcome) = set_outcome.outcomes.iter().find(|o| &o.gate_id == target_id) {
                        let envelope = JsonEnvelope::new(target_outcome.to_payload());
                        let _ = output.emit_envelope(&envelope);
                    } else {
                        let envelope = JsonEnvelope::new(set_outcome.to_payload());
                        let _ = output.emit_envelope(&envelope);
                    }
                } else {
                    let envelope = JsonEnvelope::new(set_outcome.to_payload());
                    let _ = output.emit_envelope(&envelope);
                }
            } else {
                let text = set_outcome.receipts();
                if !text.is_empty() {
                    let _ = output.emit_text(&text);
                }
            }

            set_outcome.aggregate_exit_code()
        }
    }
}
