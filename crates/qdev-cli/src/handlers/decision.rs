use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use qdev_core::{resolve_author, ExitCode, JsonEnvelope, QdevError};

pub fn handle_decision(
    decision_args: &cli::DecisionArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    match decision_args.command {
        cli::DecisionCommands::Log(ref log_args) => {
            handle_decision_log(log_args, annotated_config, cli, output, current_dir)
        }
    }
}

pub fn handle_decision_log(
    log_args: &cli::DecisionLogArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // 1. Validate topic is non-empty
    if log_args.topic.trim().is_empty() {
        let err = qdev_core::QdevError::usage_error("--topic cannot be empty or whitespace-only");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 2. Validate ruling is non-empty
    if log_args.ruling.trim().is_empty() {
        let err = qdev_core::QdevError::usage_error("--ruling cannot be empty or whitespace-only");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 3. Validate decision_type is recognized
    let trimmed_type = log_args.r#type.trim();
    if !qdev_core::VALID_DECISION_TYPES.contains(&trimmed_type) {
        let err = qdev_core::QdevError::usage_error(format!(
            "Invalid decision type '{}'; allowed types are: {}",
            trimmed_type,
            qdev_core::VALID_DECISION_TYPES.join(", ")
        ));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 4. Resolve author
    let author = match resolve_author(
        log_args.author_type.as_deref(),
        log_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // 5. Build DecisionInput
    let input = qdev_core::DecisionInput {
        subject_id: log_args.subject.trim().to_string(),
        decision_type: trimmed_type.to_string(),
        topic: Some(log_args.topic.trim().to_string()),
        context: log_args.context.as_ref().map(|c| c.trim().to_string()),
        ruling: log_args.ruling.trim().to_string(),
        author,
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    // 6. Log decision
    let payload =
        match qdev_core::log_decision(&root, Some(&annotated_config.config.storage), &input) {
            Ok(p) => p,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };

    // 7. Emit output
    if cli.json {
        let envelope = JsonEnvelope::new(payload);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit decision log envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = format!(
            "Logged decision {} on {} [{}] (topic: \"{}\") -> {}\n",
            payload.id,
            payload.subject_id,
            payload.decision_type,
            payload.topic.as_deref().unwrap_or(""),
            payload.path
        );
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit decision log output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
