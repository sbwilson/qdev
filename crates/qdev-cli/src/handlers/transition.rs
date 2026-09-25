use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::{check_governance_gate, GovernanceOutcome};
use qdev_core::{resolve_author, ExitCode, Interactivity, JsonEnvelope, QdevError};

pub fn handle_transition(
    transition_args: &cli::TransitionArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // Kind validation
    if transition_args.kind != "story" {
        let err = QdevError::usage_error(format!(
            "Transition command only supports 'story' entities, got '{}'",
            transition_args.kind
        ));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // Target status parsing
    if let Err(e) = qdev_core::StoryState::parse(&transition_args.target_status) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    // Resolve active author attribution
    let author = match resolve_author(
        transition_args.author_type.as_deref(),
        transition_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut if_version = transition_args.if_version;
    let (gov_just, override_decision) = match check_governance_gate(
        &root,
        &transition_args.id,
        Some(qdev_core::EntityKind::Story),
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

    let effective_justification = transition_args
        .justification
        .clone()
        .or_else(|| cli.justification.clone())
        .or(gov_just);

    let options = qdev_core::TransitionOptions {
        workspace_root: root.clone(),
        storage: Some(annotated_config.config.storage.clone()),
        entity_kind: transition_args.kind.clone(),
        story_id: transition_args.id.clone(),
        target_status: transition_args.target_status.clone(),
        justification: effective_justification,
        author: author.clone(),
        if_version,
        skip_gates: transition_args.skip_gates,
        interactivity,
    };

    let mut engine = qdev_core::TransitionEngine::new();
    engine.add_pre_hook(qdev_core::TransitionGateHook::new(
        annotated_config.config.clone(),
    ));
    let res = match engine.transition(&options) {
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
            &transition_args.id,
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
        let envelope = JsonEnvelope::new(res);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit transition envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let mut msg = format!(
            "Transitioned story {} {} -> {} (version {})\n",
            res.id, res.from_status, res.to_status, res.version
        );
        if let Some(ref dec_id) = res.decision_id {
            msg.push_str(&format!("Recorded decision: {}\n", dec_id));
        }
        if !res.closed_dw.is_empty() {
            msg.push_str(&format!(
                "Closed deferred work: {}\n",
                res.closed_dw.join(", ")
            ));
        }
        if let Err(e) = output.emit_text(&msg) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit transition output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
