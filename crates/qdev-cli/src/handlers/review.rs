use crate::cli::{self, Cli};
use crate::open_query_store;
use crate::output::OutputEmitter;
use qdev_core::{ExitCode, JsonEnvelope};

pub fn handle_review(
    args: &cli::ReviewArgs,
    config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);
    let store = match open_query_store(&root, config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };
    match &args.command {
        cli::ReviewCommands::Epic(a) => {
            let required = config
                .config
                .gates
                .iter()
                .filter(|g| g.on_transition.iter().any(|t| t == "review"))
                .map(|g| g.id.clone())
                .collect::<Vec<_>>();
            match qdev_core::review_epic_with_gates(&store, &a.id, &required) {
                Ok(result) => emit(
                    &result,
                    format!(
                        "Epic {}: {} stories, {} open deferred-work items\n",
                        result.epic_id,
                        result.stories.len(),
                        result.open_deferred_work.len()
                    ),
                    cli,
                    output,
                ),
                Err(e) => {
                    let _ = output.emit_error(&e);
                    e.exit_code()
                }
            }
        }
        cli::ReviewCommands::Sprint(a) => {
            let explicit = a.id.as_deref().or(a.sprint.as_deref());
            let sprint = match qdev_core::resolve_sprint_selection(&store, &config.config, explicit)
            {
                Ok(s) => s,
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
            };
            match qdev_core::review_sprint(
                &root,
                &config.config.storage,
                &store,
                &config.config,
                sprint,
            ) {
                Ok(result) => emit(
                    &result,
                    format!(
                        "Sprint {} (release {}) review: {} and {}\n",
                        result.sprint_id, result.release, result.rtm_path, result.anomalies_path
                    ),
                    cli,
                    output,
                ),
                Err(e) => {
                    let _ = output.emit_error(&e);
                    e.exit_code()
                }
            }
        }
    }
}
fn emit<T: serde::Serialize>(
    result: &T,
    text: String,
    cli: &Cli,
    output: &OutputEmitter,
) -> ExitCode {
    let written = if cli.json {
        output.emit_envelope(&JsonEnvelope::new(result))
    } else {
        output.emit_text(&text)
    };
    match written {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            let err = qdev_core::QdevError::infrastructure_failure("io_error", e.to_string());
            let _ = output.emit_error(&err);
            ExitCode::InfrastructureFailure
        }
    }
}
