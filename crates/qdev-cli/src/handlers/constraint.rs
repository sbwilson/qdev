use serde::Serialize;

use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::{check_governance_gate, GovernanceOutcome};
use qdev_core::{resolve_author, ExitCode, Interactivity, JsonEnvelope, QdevError};

#[derive(Serialize)]
struct ConstraintAddPayload {
    id: String,
    relative_id: String,
    owner_id: String,
    kind: String,
    text: String,
    version: u64,
    constraints: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct ConstraintRemovePayload {
    id: String,
    owner_id: String,
    version: u64,
    constraints: Vec<serde_json::Value>,
}

pub fn handle_constraint(
    args: &cli::ConstraintArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    match args.command {
        cli::ConstraintCommands::Add(ref add_args) => handle_constraint_add(
            add_args,
            annotated_config,
            cli,
            output,
            current_dir,
            interactivity,
        ),
        cli::ConstraintCommands::Remove(ref remove_args) => handle_constraint_remove(
            remove_args,
            annotated_config,
            cli,
            output,
            current_dir,
            interactivity,
        ),
    }
}

pub fn handle_constraint_add(
    add_args: &cli::ConstraintAddArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let kind = match qdev_core::ConstraintKind::from_kind_str(&add_args.kind) {
        Some(k) => k,
        None => {
            let err = QdevError::usage_error(format!(
                "Invalid constraint kind '{}', expected one of: 'no_go', 'rabbit_hole', 'appetite'",
                add_args.kind
            ));
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    };

    if add_args.text.trim().is_empty() {
        let err = QdevError::usage_error("Constraint text cannot be empty");
        let _ = output.emit_error(&err);
        return err.exit_code();
    }

    let author = match resolve_author(
        add_args.author_type.as_deref(),
        add_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut if_version = add_args.if_version;
    let (_gov_just, override_decision) = match check_governance_gate(
        &root,
        &add_args.target,
        None,
        &author,
        annotated_config,
        interactivity,
        cli,
        output,
        &mut if_version,
    ) {
        Ok(GovernanceOutcome::Proceed {
            justification,
            override_decision,
        }) => (justification, override_decision),
        Ok(GovernanceOutcome::Aborted) => return ExitCode::Success,
        Err(code) => return code,
    };

    let opts = qdev_core::ConstraintAddOptions {
        workspace_root: root.clone(),
        storage: Some(annotated_config.config.storage.clone()),
        entity_id: add_args.target.clone(),
        kind,
        text: add_args.text.clone(),
        if_version,
        author: author.clone(),
    };

    let res = match qdev_core::apply_constraint_add(&opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if let Some((dec_type, dec_just)) = override_decision {
        if let Err(e) = qdev_core::create_governance_override_decision(
            &root,
            Some(&annotated_config.config.storage),
            &add_args.target,
            &dec_type,
            &dec_just,
            &author,
            None,
        ) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(ConstraintAddPayload {
            id: res.id.clone(),
            relative_id: res.relative_id.clone(),
            owner_id: res.owner_id.clone(),
            kind: res.kind.clone(),
            text: res.text.clone(),
            version: res.new_version,
            constraints: res.constraints,
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit constraint envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        println!(
            "Added constraint {} [{}] to {} (version {}) at {}",
            res.id, res.kind, res.owner_id, res.new_version, res.rel_path
        );
    }

    ExitCode::Success
}

pub fn handle_constraint_remove(
    remove_args: &cli::ConstraintRemoveArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let (owner_id, _) = match remove_args.target.trim().split_once('/') {
        Some((o, r)) if !o.is_empty() && !r.is_empty() => (o, r),
        _ => {
            let err = QdevError::usage_error(format!(
                "Invalid constraint identifier '{}', must be of form {{owner}}/{{kind}}-{{n}}",
                remove_args.target
            ));
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    };

    let author = match resolve_author(
        remove_args.author_type.as_deref(),
        remove_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut if_version = remove_args.if_version;
    let (gov_just, override_decision) = match check_governance_gate(
        &root,
        owner_id,
        None,
        &author,
        annotated_config,
        interactivity,
        cli,
        output,
        &mut if_version,
    ) {
        Ok(GovernanceOutcome::Proceed {
            justification,
            override_decision,
        }) => (justification, override_decision),
        Ok(GovernanceOutcome::Aborted) => return ExitCode::Success,
        Err(code) => return code,
    };

    let effective_justification = remove_args
        .justification
        .clone()
        .or_else(|| cli.justification.clone())
        .or(gov_just);

    let opts = qdev_core::ConstraintRemoveOptions {
        workspace_root: root.clone(),
        storage: Some(annotated_config.config.storage.clone()),
        constraint_id: remove_args.target.clone(),
        justification: effective_justification,
        if_version,
        author: author.clone(),
    };

    let res = match qdev_core::apply_constraint_remove(&opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if let Some((dec_type, dec_just)) = override_decision {
        if let Err(e) = qdev_core::create_governance_override_decision(
            &root,
            Some(&annotated_config.config.storage),
            owner_id,
            &dec_type,
            &dec_just,
            &author,
            None,
        ) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(ConstraintRemovePayload {
            id: res.id.clone(),
            owner_id: res.owner_id.clone(),
            version: res.new_version,
            constraints: res.constraints,
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit constraint envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        println!(
            "Removed constraint {} from {} (version {}) at {}",
            res.id, res.owner_id, res.new_version, res.rel_path
        );
    }

    ExitCode::Success
}
