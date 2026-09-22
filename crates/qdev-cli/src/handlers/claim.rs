use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::{open_query_store, resolve_kind_and_id};
use qdev_core::{resolve_author, ExitCode, JsonEnvelope, QdevError, Store};

pub fn handle_claim(
    args: &cli::ClaimArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);
    let (kind, story_id) = match resolve_kind_and_id(&args.target, args.id.as_deref()) {
        Ok(res) => res,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let opt_store = open_query_store(&root, annotated_config).ok();

    if let Some(k) = kind {
        if k != qdev_core::EntityKind::Story {
            let err = QdevError::usage_error("Only story entities can be leased");
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    } else if let Some(ref store) = opt_store {
        if let Ok(Some(record)) = store.get_live_entity_for_derivation(&story_id) {
            if record.kind != qdev_core::EntityKind::Story {
                let err = QdevError::usage_error("Only story entities can be leased");
                let _ = output.emit_error(&err);
                return err.exit_code();
            }
        }
    } else if let Ok((k, _, _)) = qdev_core::resolve_entity_file(
        &root,
        None,
        &story_id,
        Some(&annotated_config.config.storage),
    ) {
        if k != qdev_core::EntityKind::Story {
            let err = QdevError::usage_error("Only story entities can be leased");
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    }

    let author = match resolve_author(
        args.author_type.as_deref(),
        args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    match qdev_core::claim_story(
        &root,
        &story_id,
        &author,
        Some(&annotated_config.config.storage),
        opt_store.as_ref().map(|s| s as &dyn qdev_core::Store),
    ) {
        Ok(lease) => {
            if cli.json {
                let envelope = JsonEnvelope::new(lease);
                if let Err(e) = output.emit_envelope(&envelope) {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to emit claim envelope: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            } else {
                let msg = format!(
                    "Claimed lease on story {}\nexport QDEV_SESSION={}\n",
                    story_id, lease.session_token
                );
                if let Err(e) = output.emit_text(&msg) {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to emit claim output: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            }
            ExitCode::Success
        }
        Err(e) => {
            let _ = output.emit_error(&e);
            e.exit_code()
        }
    }
}

pub fn handle_release(
    args: &cli::ReleaseArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let opt_store = open_query_store(&root, annotated_config).ok();

    let story_id = if let Some(ref target) = args.target {
        let (kind, id) = match resolve_kind_and_id(target, args.id.as_deref()) {
            Ok(res) => res,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };

        if let Some(k) = kind {
            if k != qdev_core::EntityKind::Story {
                let err = QdevError::usage_error("Only story entities can be leased");
                let _ = output.emit_error(&err);
                return err.exit_code();
            }
        } else if let Some(ref store) = opt_store {
            if let Ok(Some(record)) = store.get_live_entity_for_derivation(&id) {
                if record.kind != qdev_core::EntityKind::Story {
                    let err = QdevError::usage_error("Only story entities can be leased");
                    let _ = output.emit_error(&err);
                    return err.exit_code();
                }
            }
        } else if let Ok((k, _, _)) =
            qdev_core::resolve_entity_file(&root, None, &id, Some(&annotated_config.config.storage))
        {
            if k != qdev_core::EntityKind::Story {
                let err = QdevError::usage_error("Only story entities can be leased");
                let _ = output.emit_error(&err);
                return err.exit_code();
            }
        }
        id
    } else {
        match qdev_core::find_active_lease(&root) {
            Ok(lease) => lease.story_id,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        }
    };

    let author = match resolve_author(
        args.author_type.as_deref(),
        args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    match qdev_core::release_story(
        &root,
        &story_id,
        &author,
        args.force,
        cli.justification.as_deref(),
        Some(&annotated_config.config.storage),
        opt_store.as_ref().map(|s| s as &dyn qdev_core::Store),
    ) {
        Ok(res) => {
            if cli.json {
                let envelope = JsonEnvelope::new(res);
                if let Err(e) = output.emit_envelope(&envelope) {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to emit release envelope: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            } else {
                let mut msg = format!("Released lease on story {}\n", res.story_id);
                if let Some(ref dec_id) = res.decision_id {
                    msg.push_str(&format!("Recorded decision: {}\n", dec_id));
                }
                if let Err(e) = output.emit_text(&msg) {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to emit release output: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            }
            ExitCode::Success
        }
        Err(e) => {
            let _ = output.emit_error(&e);
            e.exit_code()
        }
    }
}
