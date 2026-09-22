use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::{open_query_store, reject_empty_filter_values};
use qdev_core::{resolve_author, ExitCode, JsonEnvelope, QdevError};

pub fn handle_next(
    next_args: &cli::NextArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    if let Err(e) = reject_empty_filter_values(&[("--owner", next_args.owner.as_deref())]) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    // `--owner me` resolves through the current identity (resolve_author chain);
    // any other value is a literal owner string.
    let owner = next_args.owner.as_deref().map(|o| {
        if o.trim() == "me" {
            qdev_core::NextOwnerFilter::Current
        } else {
            qdev_core::NextOwnerFilter::Literal(o.trim().to_string())
        }
    });

    let author = match resolve_author(None, None, annotated_config, &root) {
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

    let options = qdev_core::NextOptions {
        workspace_root: &root,
        store: &store,
        config: &annotated_config.config,
        sprint: next_args.sprint.map(i64::from),
        owner,
        author,
    };

    let selection = match qdev_core::select_next(&options) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(selection);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit next envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_next_text(&selection);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit next output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

/// Human summary for `qdev next` without `--json`: the selected story's reason line, the
/// per-filter notes, and the blockers section naming each nearest cause.
pub fn render_next_text(selection: &qdev_core::NextSelection) -> String {
    let mut text = String::from("Next\n");
    if let Some(record) = &selection.next {
        text.push_str(&format!("  {}\n", selection.reason.summary));
        for note in &selection.reason.notes {
            text.push_str(&format!("  {note}\n"));
        }
        if let Some(lease) = &record.lease {
            text.push_str(&format!(
                "  Leased by {} in {} — finish it before picking anything else.\n",
                lease.holder, lease.worktree_path
            ));
        }
    } else {
        text.push_str(&format!(
            "  Nothing eligible: {}\n",
            selection.reason.summary
        ));
        for note in &selection.reason.notes {
            text.push_str(&format!("  {note}\n"));
        }
    }
    if !selection.blockers.is_empty() {
        text.push_str("Blockers\n");
        for blocker in &selection.blockers {
            match &blocker.story_id {
                Some(story_id) => text.push_str(&format!(
                    "  {story_id} ({}): {}\n",
                    blocker.kind, blocker.detail
                )),
                None => text.push_str(&format!("  ({}): {}\n", blocker.kind, blocker.detail)),
            }
        }
    }
    text
}
