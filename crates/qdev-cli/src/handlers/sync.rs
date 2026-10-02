//! Cache synchronization and rebuild handler: coordinates hydration sweeps and rebuilds.

use crate::cli::{self, Cli};
use crate::open_query_store;
use crate::output::OutputEmitter;
use qdev_core::{ExitCode, JsonEnvelope, QdevError, Store};

pub fn handle_sync(
    sync_args: &cli::SyncArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    boot_summary: Option<&qdev_core::SweepSummary>,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let storage = &annotated_config.config.storage;

    // `sweep_workspace` takes the advisory write lock itself, so the plain branch must not (it
    // would deadlock). `reset_and_rebuild` does not — `ensure_cache` calls it while already
    // holding the lock — so this, its only other caller, takes it here: a rebuild drops and
    // repopulates every table, and a concurrent `qdev update` landing in the middle of that
    // would be rebuilt away or read mid-write.
    let summary = if sync_args.rebuild {
        let lock_path = root.join(&storage.cache_dir).join("write.lock");
        match qdev_core::acquire_write_lock(&lock_path, std::time::Duration::from_millis(5000)) {
            Ok(_guard) => store.reset_and_rebuild(&root, storage),
            Err(e) => Err(e),
        }
    } else if let Some(boot) = boot_summary {
        // The boot-time sweep already ran against this workspace in this process. Sweeping a
        // second time here would find everything settled and report `parsed: 0`, making sync
        // look like a no-op in exactly the invocation where real hydration happened (at
        // boot). Report the boot pass instead.
        Ok(*boot)
    } else {
        store.sweep_workspace(&root, storage)
    };

    let summary = match summary {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(summary);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sync envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_sync_text(&summary);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sync output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

pub fn render_sync_text(summary: &qdev_core::SweepSummary) -> String {
    format!(
        "parsed={}, unchanged={}, retained={}, hashed={}, purged={}, findings={}\n",
        summary.parsed,
        summary.unchanged,
        summary.retained,
        summary.hashed,
        summary.purged,
        summary.findings
    )
}
