use std::path::Path;

use qdev_core::{
    execute_configured_command, parse_cargo_audit_json_reported, persist_soup_records,
    record_sbom_artifact, ExitCode, GateRunOptions, GateStatus, JsonEnvelope, QdevError,
};
use serde::Serialize;

use crate::cli::{Cli, SoupArgs, SoupCommands};
use crate::output::OutputEmitter;

#[derive(Serialize)]
struct SoupPayload {
    runs: Vec<qdev_core::GateRunPayload>,
    findings: usize,
    warnings: Vec<String>,
}

pub fn handle_soup(
    args: &SoupArgs,
    config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    workspace: &Path,
) -> ExitCode {
    match &args.command {
        SoupCommands::Audit(audit) => {
            let audit_command = match config
                .config
                .soup
                .audit_command
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                Some(command) => command,
                None => {
                    return emit_error(
                        output,
                        QdevError::usage_error("missing [soup].audit_command configuration"),
                    )
                }
            };
            if let Some(release) = &audit.release {
                if let Err(err) = qdev_core::resolve_entity_file(
                    workspace,
                    Some(qdev_core::EntityKind::Release),
                    release,
                    Some(&config.config.storage),
                ) {
                    return emit_error(output, err);
                }
            }
            let mut runs = vec![match execute_configured_command(
                workspace,
                &config.config,
                "qdev-soup-audit",
                audit_command,
                // Workspace-level command: the run must never be attributed to an unrelated
                // story that happens to hold a lease in this workspace.
                &GateRunOptions {
                    story: None,
                    workspace_level: true,
                    timeout_ms: None,
                    ..Default::default()
                },
            ) {
                Ok(run) => run,
                Err(err) => return emit_error(output, err),
            }];
            if let Some(command) = config
                .config
                .soup
                .deny_command
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                match execute_configured_command(
                    workspace,
                    &config.config,
                    "qdev-soup-deny",
                    command,
                    &GateRunOptions {
                        workspace_level: true,
                        ..Default::default()
                    },
                ) {
                    Ok(run) => runs.push(run),
                    Err(err) => return emit_error(output, err),
                }
            }
            let exit = aggregate(&runs);
            let (findings, warnings) = if exit == ExitCode::Success {
                let parsed =
                    parse_cargo_audit_json_reported(runs[0].stdout.as_deref().unwrap_or(""));
                let mut warnings = Vec::new();
                if let Some(w) = parsed.warning {
                    warnings.push(w);
                }
                (parsed.findings, warnings)
            } else {
                (Vec::new(), Vec::new())
            };
            if exit == ExitCode::Success {
                if let Err(err) = persist_soup_records(
                    workspace,
                    &config.config,
                    &findings,
                    audit.release.as_deref(),
                ) {
                    return emit_error(output, err);
                }
            }
            emit_runs(cli, output, runs, findings.len(), warnings);
            exit
        }
        SoupCommands::Sbom(sbom) => {
            let command = match config
                .config
                .soup
                .sbom_command
                .as_deref()
                .filter(|s| !s.trim().is_empty())
            {
                Some(command) => command,
                None => {
                    return emit_error(
                        output,
                        QdevError::usage_error("missing [soup].sbom_command configuration"),
                    )
                }
            };
            // Resolve the release before launching the command so a bad selector cannot produce side effects.
            if let Err(err) = qdev_core::resolve_entity_file(
                workspace,
                Some(qdev_core::EntityKind::Release),
                &sbom.release,
                Some(&config.config.storage),
            ) {
                return emit_error(output, err);
            }
            let run = match execute_configured_command(
                workspace,
                &config.config,
                "qdev-soup-sbom",
                command,
                &GateRunOptions {
                    workspace_level: true,
                    ..Default::default()
                },
            ) {
                Ok(run) => run,
                Err(err) => return emit_error(output, err),
            };
            let exit = run.cli_exit_code();
            if exit == ExitCode::Success {
                let artifact = run
                    .stdout
                    .as_deref()
                    .and_then(|s| s.lines().rev().find(|line| !line.trim().is_empty()))
                    .map(str::trim);
                match artifact {
                    Some(path) => {
                        if let Err(err) =
                            record_sbom_artifact(workspace, &config.config, &sbom.release, path)
                        {
                            return emit_error(output, err);
                        }
                    }
                    None => return emit_error(
                        output,
                        QdevError::usage_error(
                            "SBOM command succeeded but did not report an artifact path on stdout",
                        ),
                    ),
                }
            }
            emit_runs(cli, output, vec![run], 0, Vec::new());
            exit
        }
    }
}

fn aggregate(runs: &[qdev_core::GateRunOutcome]) -> ExitCode {
    if runs.iter().any(|r| r.status == GateStatus::Fail) {
        ExitCode::LogicalFailure
    } else if runs.iter().any(|r| r.status == GateStatus::Infra) {
        ExitCode::InfrastructureFailure
    } else {
        ExitCode::Success
    }
}

fn emit_runs(
    cli: &Cli,
    output: &OutputEmitter,
    runs: Vec<qdev_core::GateRunOutcome>,
    findings: usize,
    warnings: Vec<String>,
) {
    if cli.json {
        let _ = output.emit_envelope(&JsonEnvelope::new(SoupPayload {
            runs: runs.into_iter().map(|r| r.to_payload()).collect(),
            findings,
            warnings,
        }));
    } else {
        let mut lines = runs.iter().map(|r| r.receipt()).collect::<Vec<_>>();
        lines.extend(warnings.iter().map(|w| format!("warning: {w}")));
        let _ = output.emit_text(&lines.join("\n"));
    }
}

fn emit_error(output: &OutputEmitter, err: QdevError) -> ExitCode {
    let code = err.exit_code();
    let _ = output.emit_error(&err);
    code
}
