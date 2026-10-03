use serde::Serialize;

use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use qdev_core::{resolve_author, ExitCode, JsonEnvelope};

pub fn handle_chore(
    chore_args: &cli::ChoreArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    match chore_args.command {
        cli::ChoreCommands::Start(ref start_args) => {
            handle_chore_start(start_args, annotated_config, cli, output, current_dir)
        }
        cli::ChoreCommands::Commit(ref commit_args) => {
            handle_chore_commit(commit_args, annotated_config, cli, output, current_dir)
        }
        cli::ChoreCommands::List => handle_chore_list(annotated_config, cli, output, current_dir),
        cli::ChoreCommands::Close(ref finish_args) => handle_chore_finish(
            finish_args,
            annotated_config,
            cli,
            output,
            current_dir,
            ChoreOutcome::Closed,
        ),
        cli::ChoreCommands::Abort(ref finish_args) => handle_chore_finish(
            finish_args,
            annotated_config,
            cli,
            output,
            current_dir,
            ChoreOutcome::Abandoned,
        ),
    }
}

/// Which outcome `qdev chore close` / `qdev chore abort` records — one handler, two answers.
#[derive(Debug, Clone, Copy)]
pub enum ChoreOutcome {
    Closed,
    Abandoned,
}

/// Every chore record in the workspace, settled ones included. Without this a record that
/// nothing can commit still blocks `chore start`, and there is no way to see which record that
/// is or what it was for.
pub fn handle_chore_list(
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let records = match qdev_core::list_chore_records(&root, Some(&annotated_config.config.storage))
    {
        Ok(records) => records,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let text = if records.is_empty() {
        "No chore records in this workspace.\n  Start one with `qdev chore start \"<title>\" --paths <glob>`.\n"
            .to_string()
    } else {
        let mut text = String::from("Chore records:\n");
        for record in &records {
            text.push_str(&format!(
                "  {}  [{}]  {}  (allowlist: {})\n",
                record.id,
                record.status,
                record.title,
                record.paths.join(", ")
            ));
            if let Some(commit) = &record.commit {
                text.push_str(&format!("    committed as {}\n", commit));
            }
            if let (Some(id), Some(path)) = (&record.decision_id, &record.decision_path) {
                text.push_str(&format!("    recorded: {} ({})\n", id, path));
            }
            if let Some(reason) = &record.reason {
                text.push_str(&format!("    reason: {}\n", reason));
            }
            if record.status == "open" {
                text.push_str(
                    "    run `qdev chore commit` to commit it, or `qdev chore close`/`abort` to \
                     settle it\n",
                );
            }
        }
        text
    };

    if cli.json {
        let envelope = JsonEnvelope::new(ChoreListPayload {
            chores: records,
            record_dir: qdev_core::chore_dir(&root, Some(&annotated_config.config.storage))
                .strip_prefix(&root)
                .unwrap_or(std::path::Path::new(".qdev/chores"))
                .display()
                .to_string(),
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit chore list envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else if let Err(e) = output.emit_text(&text) {
        let err = qdev_core::QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to emit chore list text: {}", e),
        );
        let _ = output.emit_error(&err);
        return ExitCode::InfrastructureFailure;
    }

    ExitCode::Success
}

/// Settles the open chore without committing it, which is the way out of a chore whose
/// allowlist was wrong: `commit` refuses (`nothing_to_commit`) and `start` refuses
/// (`chore_in_progress`) until the record is settled one way or the other.
pub fn handle_chore_finish(
    finish_args: &cli::ChoreFinishArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    outcome: ChoreOutcome,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let author = match resolve_author(
        finish_args.author_type.as_deref(),
        finish_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let input = qdev_core::FinishChoreInput {
        workspace_root: &root,
        storage: Some(&annotated_config.config.storage),
        reason: finish_args.reason.clone(),
        author,
    };

    let (verb, result) = match outcome {
        ChoreOutcome::Closed => ("Closed", qdev_core::close_chore(&input)),
        ChoreOutcome::Abandoned => ("Abandoned", qdev_core::abort_chore(&input)),
    };
    let record = match result {
        Ok(record) => record,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(&record);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit chore {} envelope: {}", verb, e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
        return ExitCode::Success;
    }

    let mut text = format!(
        "{} chore {} ({}) as {}.\n",
        verb, record.id, record.title, record.status
    );
    if let Some(reason) = &record.reason {
        text.push_str(&format!("  reason: {}\n", reason));
    }
    text.push_str("  A new chore can now be started with `qdev chore start`.\n");
    if let Err(e) = output.emit_text(&text) {
        let err = qdev_core::QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to emit chore {} text: {}", verb, e),
        );
        let _ = output.emit_error(&err);
        return ExitCode::InfrastructureFailure;
    }

    ExitCode::Success
}

/// `qdev chore list --json`: every record, plus where they are kept.
#[derive(Serialize)]
struct ChoreListPayload {
    chores: Vec<qdev_core::ChoreRecord>,
    record_dir: String,
}

pub fn handle_chore_start(
    start_args: &cli::ChoreStartArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let author = match resolve_author(
        start_args.author_type.as_deref(),
        start_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let input = qdev_core::StartChoreInput {
        workspace_root: &root,
        storage: Some(&annotated_config.config.storage),
        title: start_args.title.clone(),
        paths: start_args.paths.clone(),
        author,
        alongside: start_args.alongside,
    };

    let record = match qdev_core::start_chore(&input) {
        Ok(record) => record,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(&record);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit chore start envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = format!(
            "Started chore {} ({}) under allowlist: {}\nRecord: {}/{}.json — run `qdev chore commit` to commit it\n",
            record.id,
            record.title,
            record.paths.join(", "),
            qdev_core::chore_dir(&root, Some(&annotated_config.config.storage))
                .strip_prefix(&root)
                .unwrap_or(std::path::Path::new(".qdev/chores"))
                .display(),
            record.id,
        );
        if let Err(e) = output.emit_text(&text) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit chore start text: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

pub fn handle_chore_commit(
    commit_args: &cli::ChoreCommitArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let author = match resolve_author(
        commit_args.author_type.as_deref(),
        commit_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let input = qdev_core::CommitChoreInput {
        workspace_root: &root,
        storage: Some(&annotated_config.config.storage),
        author,
        strict: commit_args.strict,
        config: Some(&annotated_config.config),
    };

    let result = match qdev_core::commit_chore(&input) {
        Ok(result) => result,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(&result);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit chore commit envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let mut text = format!(
            "Committed chore {} as {}\n  included: {}\n",
            result.chore_id,
            result.commit.as_deref().unwrap_or("-"),
            result.included.join(", "),
        );
        // Two lists, not one: calling an unstaged change "still staged" tells the reader
        // something false about every path that was never in the index.
        let staged: Vec<String> = result
            .excluded
            .iter()
            .filter(|e| e.staged)
            .map(|e| e.path.clone())
            .collect();
        let unstaged: Vec<String> = result
            .excluded
            .iter()
            .filter(|e| !e.staged)
            .map(|e| e.path.clone())
            .collect();
        if result.excluded.is_empty() {
            text.push_str("  excluded: none\n");
        } else {
            if !staged.is_empty() {
                text.push_str(&format!(
                    "  NOT INCLUDED — still staged: {}\n",
                    staged.join(", ")
                ));
            }
            if !unstaged.is_empty() {
                text.push_str(&format!(
                    "  NOT INCLUDED — not staged: {}\n",
                    unstaged.join(", ")
                ));
            }
        }
        if let (Some(id), Some(path)) = (&result.decision_id, &result.decision_path) {
            text.push_str(&format!("  recorded: {} ({})\n", id, path));
        }
        if let Err(e) = output.emit_text(&text) {
            let err = qdev_core::QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit chore commit text: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}
