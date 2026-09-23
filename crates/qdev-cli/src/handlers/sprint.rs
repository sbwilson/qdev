use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::open_query_store;
use qdev_core::{resolve_author, ExitCode, JsonEnvelope};

pub fn handle_sprint(
    sprint_args: &cli::SprintArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    match sprint_args.command {
        cli::SprintCommands::Open(ref open_args) => {
            handle_sprint_open(open_args, annotated_config, cli, output, current_dir)
        }
        cli::SprintCommands::Assign(ref assign_args) => {
            handle_sprint_assign(assign_args, annotated_config, cli, output, current_dir)
        }
        cli::SprintCommands::Close(ref close_args) => {
            handle_sprint_close(close_args, annotated_config, cli, output, current_dir)
        }
    }
}

pub fn handle_sprint_open(
    open_args: &cli::SprintOpenArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // Validate title is non-empty
    if open_args.title.trim().is_empty() {
        let err =
            qdev_core::QdevError::logical_failure("empty_title", "Sprint title cannot be empty");
        let _ = output.emit_error(&err);
        return ExitCode::LogicalFailure;
    }

    // Parse sprint id
    let sprint_num = match qdev_core::normalize_sprint_id(&open_args.id) {
        Ok(n) => n,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let author = match resolve_author(
        open_args.author_type.as_deref(),
        open_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::SprintOpenOptions {
        workspace_root: &root,
        storage: &annotated_config.config.storage,
        store: &store,
        sprint: sprint_num,
        title: &open_args.title,
        release: open_args.release.as_deref(),
        author: &author,
    };

    let result = match qdev_core::open_sprint(&options) {
        Ok(res) => res,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(result);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sprint open envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = format!(
            "Opened sprint {} ({}) [{}] -> {}\n",
            result.sprint_id, result.title, result.status, result.rel_path
        );
        if let Err(e) = output.emit_text(&text) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sprint open text: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

pub fn handle_sprint_assign(
    assign_args: &cli::SprintAssignArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Determine explicit sprint and stories
    let (explicit_sprint, stories) = match (&assign_args.sprint, &assign_args.first) {
        (Some(s), Some(f)) => {
            let mut stories = vec![f.clone()];
            stories.extend(assign_args.rest.clone());
            (Some(s.clone()), stories)
        }
        (Some(s), None) => (Some(s.clone()), assign_args.rest.clone()),
        (None, Some(f)) => {
            if qdev_core::normalize_sprint_id(f).is_ok() {
                (Some(f.clone()), assign_args.rest.clone())
            } else {
                let mut stories = vec![f.clone()];
                stories.extend(assign_args.rest.clone());
                (None, stories)
            }
        }
        (None, None) => (None, assign_args.rest.clone()),
    };

    if stories.is_empty() {
        let err = qdev_core::QdevError::usage_error(
            "At least one story must be specified for sprint assignment",
        );
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    let sprint_num = match qdev_core::resolve_sprint_selection(
        &store,
        &annotated_config.config,
        explicit_sprint.as_deref(),
    ) {
        Ok(n) => n,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let author = match resolve_author(
        assign_args.author_type.as_deref(),
        assign_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::SprintAssignOptions {
        workspace_root: &root,
        storage: &annotated_config.config.storage,
        store: &store,
        sprint: sprint_num,
        stories: &stories,
        author: &author,
    };

    let result = match qdev_core::assign_to_sprint(&options) {
        Ok(res) => res,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(result);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sprint assign envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = format!(
            "Assigned {} stories to sprint {} ({})\n",
            result.assigned_stories.len(),
            result.sprint_id,
            result.assigned_stories.join(", ")
        );
        if let Err(e) = output.emit_text(&text) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sprint assign text: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

pub fn handle_sprint_close(
    close_args: &cli::SprintCloseArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let explicit_sprint = close_args.id.as_deref().or(close_args.sprint.as_deref());
    let sprint_num = match qdev_core::resolve_sprint_selection(
        &store,
        &annotated_config.config,
        explicit_sprint,
    ) {
        Ok(n) => n,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let carry_over_target = match close_args.carry_over.as_deref() {
        Some(target_str) => match qdev_core::normalize_sprint_id(target_str) {
            Ok(n) => Some(n),
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        },
        None => None,
    };

    let author = match resolve_author(
        close_args.author_type.as_deref(),
        close_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::SprintCloseOptions {
        workspace_root: &root,
        storage: &annotated_config.config.storage,
        store: &store,
        sprint: sprint_num,
        carry_over_target,
        author: &author,
        gates: Some(&annotated_config.config.gates),
        integration_branch: Some(&annotated_config.config.git.integration_branch),
    };

    let result = match qdev_core::close_sprint(&options) {
        Ok(res) => res,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(result);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sprint close envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = if result.carried_stories.is_empty() {
            format!(
                "Closed sprint {} [{}] at {}\n",
                result.sprint_id, result.status, result.completed_at
            )
        } else {
            format!(
                "Closed sprint {} [{}] at {}, carried over {} stories ({}) to sprint {}\n",
                result.sprint_id,
                result.status,
                result.completed_at,
                result.carried_stories.len(),
                result.carried_stories.join(", "),
                result.carry_over_target.unwrap_or_default()
            )
        };
        if let Err(e) = output.emit_text(&text) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sprint close text: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
