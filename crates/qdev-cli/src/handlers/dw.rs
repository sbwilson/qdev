use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::{
    check_governance_gate, open_query_store, reject_empty_filter_values, GovernanceOutcome,
};
use qdev_core::{resolve_author, ExitCode, Interactivity, JsonEnvelope, QdevError};

pub fn handle_dw(
    dw_args: &cli::DwArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    match dw_args.command {
        cli::DwCommands::Add(ref add_args) => handle_dw_add(
            add_args,
            annotated_config,
            cli,
            output,
            current_dir,
            interactivity,
        ),
        cli::DwCommands::List(ref list_args) => {
            handle_dw_list(list_args, annotated_config, cli, output, current_dir)
        }
        cli::DwCommands::Close(ref close_args) => {
            handle_dw_close(close_args, annotated_config, cli, output, current_dir)
        }
    }
}

pub fn handle_dw_add(
    add_args: &cli::DwAddArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // 1. Validate title is non-empty
    if add_args.title.trim().is_empty() {
        let err = QdevError::usage_error("Title cannot be empty");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 2. Validate safety risk
    if let Err(e) = qdev_core::validate_safety_risk(&add_args.risk) {
        let _ = output.emit_error(&e);
        return ExitCode::UsageError;
    }

    // 3. Validate target module against registered modules
    let trimmed_mod = add_args.module.trim();
    if trimmed_mod.is_empty() {
        let err = QdevError::usage_error("Target module cannot be empty");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }
    if !annotated_config
        .config
        .modules
        .iter()
        .any(|m| m.id == trimmed_mod)
    {
        let err = QdevError::usage_error(format!(
            "Module '{}' is not registered in configuration",
            trimmed_mod
        ));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 4. Resolve author
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

    // 5. Origin story resolution and governance check if specified
    let origin_story_id = if let Some(ref s) = add_args.story {
        let trimmed_s = s.trim();
        if trimmed_s.is_empty() {
            let err = QdevError::usage_error("Origin story ID cannot be empty");
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }

        let story_id = match qdev_core::resolve_entity_file(
            &root,
            Some(qdev_core::EntityKind::Story),
            trimmed_s,
            Some(&annotated_config.config.storage),
        ) {
            Ok((_, id, _)) => id,
            Err(_) => {
                let err = QdevError::usage_error_with_code(
                    "entity_not_found",
                    format!("Origin story '{}' not found in workspace", trimmed_s),
                );
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }
        };

        let (_gov_just, override_decision) = match check_governance_gate(
            &root,
            &story_id,
            Some(qdev_core::EntityKind::Story),
            &author,
            annotated_config,
            interactivity,
            cli,
            output,
            &mut None,
        ) {
            Ok(GovernanceOutcome::Proceed {
                justification,
                override_decision,
            }) => (justification, override_decision),
            Ok(GovernanceOutcome::Aborted) => return ExitCode::Success,
            Err(code) => return code,
        };

        if let Some((dec_type, dec_just)) = override_decision {
            if let Err(e) = qdev_core::create_governance_override_decision(
                &root,
                Some(&annotated_config.config.storage),
                &story_id,
                &dec_type,
                &dec_just,
                &author,
                None,
            ) {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        }

        Some(story_id)
    } else {
        None
    };

    // 6. Build input and add deferred work
    let input = qdev_core::AddDeferredWorkInput {
        title: add_args.title.trim().to_string(),
        target_module: trimmed_mod.to_string(),
        safety_risk: add_args.risk.trim().to_string(),
        origin_story_id,
        rationale: add_args
            .rationale
            .as_ref()
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty()),
        gate: add_args
            .gate
            .as_ref()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty()),
        owners: add_args.owners.clone(),
        author,
    };

    let payload = match qdev_core::add_deferred_work(&root, &annotated_config.config, &input) {
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
                format!("Failed to emit deferred work envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = format!(
            "Created deferred work {} [{}] in module '{}' (risk: {}) -> {}\n",
            payload.id, payload.status, payload.target_module, payload.safety_risk, payload.path
        );
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit deferred work output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

pub fn handle_dw_close(
    close_args: &cli::DwCloseArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // 1. Validate ID
    let trimmed_id = close_args.id.trim();
    if trimmed_id.is_empty() {
        let err = QdevError::usage_error("Deferred work ID cannot be empty");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 2. Validate close status (defaults to "done")
    let target_status = close_args
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("done");

    if target_status != "done" && target_status != "wont_fix" {
        let err = QdevError::usage_error(format!(
            "Invalid close status '{}'; allowed statuses are 'done' or 'wont_fix'",
            target_status
        ));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 3. Validate justification / resolution for wont_fix
    let justification = close_args
        .justification
        .as_deref()
        .or(cli.justification.as_deref())
        .map(str::trim)
        .filter(|j| !j.is_empty())
        .map(str::to_string);

    let has_justification_or_res = justification.is_some()
        || close_args
            .resolution
            .as_deref()
            .map(str::trim)
            .is_some_and(|r| !r.is_empty());

    if target_status == "wont_fix" && !has_justification_or_res {
        let err = QdevError::usage_error(
            "Closing deferred work as 'wont_fix' requires a non-empty --justification (or --resolution)",
        );
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 4. Resolve author
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

    // 5. Close deferred work
    let input = qdev_core::CloseDeferredWorkInput {
        id: trimmed_id.to_string(),
        status: Some(target_status.to_string()),
        resolution: close_args.resolution.clone(),
        justification,
        author,
    };

    let payload =
        match qdev_core::close_deferred_work(&root, Some(&annotated_config.config.storage), &input)
        {
            Ok(p) => p,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };

    // 6. Emit output
    if cli.json {
        let envelope = JsonEnvelope::new(payload);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit close envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = format!(
            "Closed deferred work {} [{}] -> {}\n",
            payload.id, payload.status, payload.path
        );
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit close output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

pub fn handle_dw_list(
    list_args: &cli::DwListArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    if let Err(e) = reject_empty_filter_values(&[
        ("--module", list_args.module.as_deref()),
        ("--risk", list_args.risk.as_deref()),
        ("--status", list_args.status.as_deref()),
        ("--story", list_args.story.as_deref()),
    ]) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    if let Some(ref r) = list_args.risk {
        if let Err(e) = qdev_core::validate_safety_risk(r) {
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    }

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let filter = qdev_core::ListDeferredWorkFilter {
        module: list_args.module.clone(),
        risk: list_args.risk.clone(),
        status: list_args.status.clone(),
        story: list_args.story.clone(),
    };

    let records = match qdev_core::list_deferred_work_records(&store, &filter) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(qdev_core::ListDeferredWorkPayload { items: records });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit dw list envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_dw_list_text(&records);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit dw list output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

fn render_dw_list_text(records: &[qdev_core::DeferredWorkItem]) -> String {
    const ID_WIDTH: usize = 12;
    const MODULE_WIDTH: usize = 16;
    const RISK_WIDTH: usize = 28;
    const STATUS_WIDTH: usize = 10;
    const STORY_WIDTH: usize = 10;

    let mut out = String::new();
    out.push_str(&format!(
        "{:<id_w$}  {:<mod_w$}  {:<risk_w$}  {:<status_w$}  {:<story_w$}  TITLE\n",
        "ID",
        "MODULE",
        "RISK",
        "STATUS",
        "STORY",
        id_w = ID_WIDTH,
        mod_w = MODULE_WIDTH,
        risk_w = RISK_WIDTH,
        status_w = STATUS_WIDTH,
        story_w = STORY_WIDTH,
    ));

    if records.is_empty() {
        out.push_str("(no matching deferred work records)\n");
        return out;
    }

    for r in records {
        let title = r.title.as_deref().unwrap_or("");
        let status = r.status.as_deref().unwrap_or("");
        let risk = r.safety_risk.as_deref().unwrap_or("");
        let story = r.origin_story_id.as_deref().unwrap_or("");
        out.push_str(&format!(
            "{:<id_w$}  {:<mod_w$}  {:<risk_w$}  {:<status_w$}  {:<story_w$}  {}\n",
            r.id,
            r.target_module,
            risk,
            status,
            story,
            title,
            id_w = ID_WIDTH,
            mod_w = MODULE_WIDTH,
            risk_w = RISK_WIDTH,
            status_w = STATUS_WIDTH,
            story_w = STORY_WIDTH,
        ));
    }
    out
}
