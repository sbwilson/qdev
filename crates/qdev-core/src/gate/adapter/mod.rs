pub mod cargo;
pub mod xcodebuild;

use std::path::Path;

use crate::errors::QdevError;
use crate::gate::{GateFailure, GateStatus};

/// Supported output adapters per Story 3.2 specification.
pub const VALID_ADAPTERS: &[&str] = &["json", "cargo", "xcodebuild"];

/// Validates that an adapter name is one of the built-in adapters.
pub fn validate_adapter_name(adapter: &str) -> Result<(), QdevError> {
    if VALID_ADAPTERS.contains(&adapter) {
        Ok(())
    } else {
        Err(QdevError::usage_error(format!(
            "Unknown output adapter '{}'. Valid adapters: {}",
            adapter,
            VALID_ADAPTERS.join(", ")
        )))
    }
}

/// Result of parsing gate output through an adapter.
#[derive(Debug, Clone, PartialEq)]
pub struct AdapterParseResult {
    pub status: GateStatus,
    pub summary: String,
    pub failures: Vec<GateFailure>,
}

/// Dispatches output parsing to the appropriate built-in adapter.
pub fn parse_with_adapter(
    adapter: &str,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    workspace_root: Option<&Path>,
) -> Result<AdapterParseResult, QdevError> {
    validate_adapter_name(adapter)?;

    match adapter {
        "cargo" => {
            let (status, summary, failures) =
                cargo::parse_cargo_output(stdout, stderr, exit_code, workspace_root);
            Ok(AdapterParseResult {
                status,
                summary,
                failures,
            })
        }
        "xcodebuild" => {
            let (status, summary, failures) =
                xcodebuild::parse_xcodebuild_output(stdout, stderr, exit_code, workspace_root);
            Ok(AdapterParseResult {
                status,
                summary,
                failures,
            })
        }
        "json" => Err(QdevError::usage_error(
            "json adapter produces result documents and cannot be run as stream adapter",
        )),
        _ => Err(QdevError::usage_error(format!(
            "Unknown output adapter '{}'. Valid adapters: {}",
            adapter,
            VALID_ADAPTERS.join(", ")
        ))),
    }
}
