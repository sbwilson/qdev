use std::path::Path;

use qdev_core::config::AnnotatedConfig;
use qdev_core::{
    current_iso8601, execute_gate, execute_gate_set, format_metric_number, get_gate_list,
    read_baseline, resolve_author, resolve_commit_sha, resolve_gate_execution_order,
    write_baseline, ExitCode, GateBaselinePayload, GateListPayload, GateRunOptions, GateStatus,
    JsonEnvelope, QdevError, RatchetBaseline,
};

use crate::cli::{Cli, GateArgs, GateBaselineArgs, GateCommands};
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
        GateCommands::Baseline(baseline_args) => {
            handle_gate_baseline(baseline_args, annotated_config, cli, output, workspace_root)
        }
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
                let err =
                    QdevError::usage_error("must specify a gate ID, --all, or --for-transition");
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }

            let options = GateRunOptions {
                story: run_args.story.clone(),
                timeout_ms: None,
                ..Default::default()
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
                    if let Some(target_outcome) = set_outcome
                        .outcomes
                        .iter()
                        .find(|o| &o.gate_id == target_id)
                    {
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

/// Handles `qdev gate baseline <id>` commands for inspection and recording.
pub fn handle_gate_baseline(
    args: &GateBaselineArgs,
    annotated_config: &AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    workspace_root: &Path,
) -> ExitCode {
    let gate_config = match annotated_config
        .config
        .gates
        .iter()
        .find(|g| g.id == args.id)
    {
        Some(g) => g,
        None => {
            let err =
                QdevError::usage_error(format!("gate '{}' not found in configuration", args.id));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    };

    if gate_config.kind.as_deref() != Some("ratchet") {
        let err = QdevError::usage_error(format!(
            "gate '{}' is not a ratchet gate (kind is not 'ratchet')",
            args.id
        ));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    let direction_str = gate_config.direction.as_deref().unwrap_or("");
    if direction_str != "must_not_increase" && direction_str != "must_not_decrease" {
        let err = QdevError::usage_error(format!(
            "ratchet gate '{}' requires direction 'must_not_increase' or 'must_not_decrease'",
            args.id
        ));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    if args.value.is_some() && !args.set {
        let err = QdevError::usage_error("--value requires --set flag");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    if let Some(val) = args.value {
        if !val.is_finite() {
            let err = QdevError::usage_error(format!(
                "baseline value must be a finite number, got {}",
                val
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    }

    let branch = &annotated_config.config.git.integration_branch;

    if args.set {
        let metric_value = if let Some(val) = args.value {
            val
        } else {
            let options = GateRunOptions::default();
            let outcome =
                match execute_gate(workspace_root, &annotated_config.config, &args.id, &options) {
                    Ok(o) => o,
                    Err(err) => {
                        let _ = output.emit_error(&err);
                        return err.exit_code();
                    }
                };
            if outcome.status != GateStatus::Pass {
                let err = QdevError::logical_failure(
                    "gate_failed",
                    format!(
                        "gate '{}' failed during baseline execution: {}",
                        args.id, outcome.summary
                    ),
                );
                let _ = output.emit_error(&err);
                return ExitCode::LogicalFailure;
            }
            match outcome.metric {
                Some(m) => m,
                None => {
                    let err = QdevError::logical_failure(
                        "no_metric",
                        format!("gate '{}' did not produce a numeric metric", args.id),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::LogicalFailure;
                }
            }
        };

        let author = match resolve_author(
            args.author_type.as_deref(),
            args.author_id.as_deref(),
            annotated_config,
            workspace_root,
        ) {
            Ok(a) => a,
            Err(err) => {
                let _ = output.emit_error(&err);
                return err.exit_code();
            }
        };

        let commit_sha =
            resolve_commit_sha(workspace_root).unwrap_or_else(|| "unknown".to_string());
        let timestamp = current_iso8601();

        let baseline = RatchetBaseline {
            gate: args.id.clone(),
            metric: gate_config
                .metric
                .clone()
                .unwrap_or_else(|| "metric".to_string()),
            direction: direction_str.to_string(),
            value: metric_value,
            commit: commit_sha,
            author,
            timestamp,
        };

        if let Err(err) = write_baseline(
            workspace_root,
            &annotated_config.config.storage,
            branch,
            &baseline,
        ) {
            let _ = output.emit_error(&err);
            return err.exit_code();
        }

        let payload = GateBaselinePayload::from_baseline(branch.clone(), baseline);
        if cli.json {
            let envelope = JsonEnvelope::new(payload);
            let _ = output.emit_envelope(&envelope);
        } else {
            let _ = output.emit_text(&format!(
                "Recorded baseline for gate '{}' on branch '{}': value {} (metric: {}, direction: {}, commit: {}, author: {}:{})",
                payload.gate,
                payload.branch,
                format_metric_number(payload.value.unwrap()),
                payload.metric.as_deref().unwrap_or(""),
                payload.direction.as_deref().unwrap_or(""),
                payload.commit.as_deref().unwrap_or(""),
                payload.author.as_ref().map(|a| a.author_type.as_str()).unwrap_or(""),
                payload.author.as_ref().map(|a| a.id.as_str()).unwrap_or(""),
            ));
        }

        ExitCode::Success
    } else {
        // Inspect baseline
        let maybe_baseline = match read_baseline(
            workspace_root,
            &annotated_config.config.storage.state_dir,
            branch,
            &args.id,
        ) {
            Ok(b) => b,
            Err(err) => {
                let _ = output.emit_error(&err);
                return err.exit_code();
            }
        };

        match maybe_baseline {
            Some(baseline) => {
                let payload = GateBaselinePayload::from_baseline(branch.clone(), baseline);
                if cli.json {
                    let envelope = JsonEnvelope::new(payload);
                    let _ = output.emit_envelope(&envelope);
                } else {
                    let text = format!(
                        "Baseline for gate '{}' (branch '{}'):\n  metric: {}\n  direction: {}\n  value: {}\n  commit: {}\n  author: {}:{}\n  timestamp: {}",
                        payload.gate,
                        payload.branch,
                        payload.metric.as_deref().unwrap_or(""),
                        payload.direction.as_deref().unwrap_or(""),
                        format_metric_number(payload.value.unwrap()),
                        payload.commit.as_deref().unwrap_or(""),
                        payload
                            .author
                            .as_ref()
                            .map(|a| a.author_type.as_str())
                            .unwrap_or(""),
                        payload
                            .author
                            .as_ref()
                            .map(|a| a.id.as_str())
                            .unwrap_or(""),
                        payload.timestamp.as_deref().unwrap_or(""),
                    );
                    let _ = output.emit_text(&text);
                }
                ExitCode::Success
            }
            None => {
                let payload = GateBaselinePayload::missing(args.id.clone(), branch.clone());
                if cli.json {
                    let envelope = JsonEnvelope::new(payload);
                    let _ = output.emit_envelope(&envelope);
                } else {
                    let _ = output.emit_text(&format!(
                        "No baseline recorded for gate '{}' on branch '{}'.",
                        args.id, branch
                    ));
                }
                ExitCode::Success
            }
        }
    }
}
