use std::path::Path;

use qdev_core::config::AnnotatedConfig;
use qdev_core::{
    format_impact_text, run_impact, ExitCode, ImpactOptions, ImpactPayload, JsonEnvelope,
};

use crate::cli::{Cli, ImpactArgs};
use crate::output::OutputEmitter;

/// Handles `qdev impact` command.
pub fn handle_impact(
    args: &ImpactArgs,
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

    let options = ImpactOptions {
        story: args.story.clone(),
        paths: args.paths.clone(),
    };

    let outcome = match run_impact(
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

    if cli.json {
        let payload = ImpactPayload::from(outcome);
        let envelope = JsonEnvelope::new(payload);
        let _ = output.emit_envelope(&envelope);
    } else {
        let text = format_impact_text(&outcome);
        let _ = output.emit_text(&text);
    }

    ExitCode::Success
}
