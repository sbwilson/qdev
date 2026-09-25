use std::io::{self, Write};
use std::path::Path;

use qdev_core::config::AnnotatedConfig;
use qdev_core::{
    format_preflight_text, run_preflight, ExitCode, JsonEnvelope, PreflightOptions,
    PreflightPayload, PreflightStatus,
};

use crate::cli::{Cli, PreflightArgs};
use crate::output::OutputEmitter;

/// Handles `qdev preflight` command.
pub fn handle_preflight(
    args: &PreflightArgs,
    annotated_config: &AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    workspace_root: &Path,
) -> ExitCode {
    let opt_store = {
        let cache_db_path = workspace_root
            .join(&annotated_config.config.storage.cache_dir)
            .join("cache.sqlite");
        if cache_db_path.is_file() {
            qdev_core::SqliteStore::open(&cache_db_path).ok()
        } else {
            None
        }
    };

    let options = PreflightOptions {
        story: args.story.clone(),
    };

    let outcome = match run_preflight(
        workspace_root,
        &annotated_config.config,
        &options,
        opt_store.as_ref().map(|s| s as &dyn qdev_core::Store),
    ) {
        Ok(o) => o,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if outcome.status == PreflightStatus::Pass {
        if cli.json {
            let payload = PreflightPayload::from(outcome);
            let envelope = JsonEnvelope::new(payload);
            let _ = output.emit_envelope(&envelope);
        } else {
            let text = format_preflight_text(&outcome);
            let _ = output.emit_text(&text);
        }
        ExitCode::Success
    } else {
        if cli.json {
            let payload = PreflightPayload::from(outcome);
            let envelope = JsonEnvelope::new(payload);
            let _ = output.emit_envelope(&envelope);
        } else {
            let text = format_preflight_text(&outcome);
            eprintln!("{}", text);
            let _ = io::stderr().flush();
        }
        ExitCode::PolicyRefusal
    }
}
