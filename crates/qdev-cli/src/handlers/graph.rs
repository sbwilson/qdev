//! Graph rendering handler: emits dependency DAG as Graphviz DOT or structured JSON.

use crate::cli::{self, Cli};
use crate::open_query_store;
use crate::output::OutputEmitter;
use crate::reject_empty_filter_values;
use qdev_core::{ExitCode, JsonEnvelope, QdevError};

pub fn handle_graph(
    graph_args: &cli::GraphArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    if graph_args.dot && cli.json {
        let err = QdevError::usage_error("Cannot combine '--dot' and '--json' output flags")
            .with_details(serde_json::json!({ "flags": ["--dot", "--json"] }));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    if !graph_args.dot && !cli.json {
        let err =
            QdevError::usage_error("qdev graph requires either '--dot' or '--json' output format")
                .with_details(serde_json::json!({ "flag": "output_format" }));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    let root = qdev_core::find_workspace_root(current_dir);

    if let Err(e) = reject_empty_filter_values(&[
        ("--epic", graph_args.epic.as_deref()),
        ("--sprint", graph_args.sprint.as_deref()),
    ]) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    let normalized_sprint = match graph_args.sprint.as_deref() {
        Some(s) => match qdev_core::normalize_sprint_id(s) {
            Ok(num) => Some(num),
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        },
        None => None,
    };

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::StoryGraphOptions {
        epic: graph_args.epic.clone(),
        sprint: normalized_sprint,
        highlight_critical_path: graph_args.highlight_critical_path,
    };

    let payload = match qdev_core::build_story_graph(
        &store,
        &root,
        Some(&annotated_config.config.storage),
        &options,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(payload);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit graph JSON envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let dot = qdev_core::render_graph_dot(&payload);
        if let Err(e) = output.emit_text(&dot) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit graph output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
