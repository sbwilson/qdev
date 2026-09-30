//! MCP server CLI handler.

use std::path::Path;

use qdev_core::{ExitCode, McpServer};

use crate::cli;
use crate::Cli;

pub fn handle_mcp(
    args: &cli::McpArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    _cli: &Cli,
    current_dir: &Path,
) -> ExitCode {
    match args.command {
        cli::McpCommands::Serve => {
            let root = qdev_core::find_workspace_root(current_dir);
            let server = McpServer::with_annotated_config(root, annotated_config.clone());
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            if let Err(e) = server.serve(stdin.lock(), stdout.lock()) {
                eprintln!("MCP server error: {}", e);
                return ExitCode::InfrastructureFailure;
            }
            ExitCode::Success
        }
    }
}
