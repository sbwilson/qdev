//! CLI dispatcher for `qdev hook <name>`.

use std::path::Path;

use qdev_core::{
    find_workspace_root, run_pre_commit, run_pre_push, run_prepare_commit_msg, ExitCode, QdevError,
};

use crate::cli;
use crate::output::OutputEmitter;
use crate::Cli;

pub fn handle_hook(
    hook_args: &cli::HookArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    _cli: &Cli,
    output: &OutputEmitter,
    current_dir: &Path,
) -> i32 {
    let root = find_workspace_root(current_dir);
    let result = match hook_args.name.as_str() {
        "pre-commit" => run_pre_commit(&root, &annotated_config.config, &hook_args.args, None),
        "pre-push" => run_pre_push(&root, &annotated_config.config, &hook_args.args, None),
        "prepare-commit-msg" => {
            run_prepare_commit_msg(&root, &annotated_config.config, &hook_args.args)
        }
        unknown => {
            let err = QdevError::usage_error(format!(
                "Unknown hook name '{}'. Supported hooks: pre-commit, pre-push, prepare-commit-msg",
                unknown
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError.as_i32();
        }
    };

    match result {
        Ok(Some(legacy_code)) => legacy_code,
        Ok(None) => ExitCode::Success.as_i32(),
        Err(e) => {
            let _ = output.emit_error(&e);
            e.exit_code().as_i32()
        }
    }
}
