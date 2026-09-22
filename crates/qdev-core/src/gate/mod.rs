//! Module root for gate execution models, options, outcomes, and runner.

pub mod adapter;
pub mod process;
pub mod result;
pub mod ring_buffer;
pub mod runner;

use serde::{Deserialize, Serialize};

use crate::errors::ExitCode;

pub use adapter::{parse_with_adapter, validate_adapter_name, VALID_ADAPTERS};
pub use result::GateResultDocument;
pub use ring_buffer::HeadTailBuffer;
pub use runner::{execute_gate, GateRunOptions};

/// Gate execution status taxonomy conforming to AD-5 and compliance & safety specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Pass,
    Fail,
    Infra,
    Skip,
}

impl GateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateStatus::Pass => "pass",
            GateStatus::Fail => "fail",
            GateStatus::Infra => "infra",
            GateStatus::Skip => "skip",
        }
    }

    pub fn default_exit_code(&self) -> ExitCode {
        match self {
            GateStatus::Pass | GateStatus::Skip => ExitCode::Success,
            GateStatus::Fail => ExitCode::LogicalFailure,
            GateStatus::Infra => ExitCode::InfrastructureFailure,
        }
    }
}

impl std::fmt::Display for GateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Structured failure reported by a gate or output adapter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateFailure {
    pub location: String,
    pub message: String,
}

/// Output payload of a gate run matching payload-gate-run.json schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateRunPayload {
    pub gate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub status: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub summary: String,
    pub skipped_locally: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<GateFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraint_ids: Vec<String>,
}

/// Outcome of running a single gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateRunOutcome {
    pub gate_id: String,
    pub status: GateStatus,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub summary: String,
    pub skipped_locally: bool,
    pub commit_sha: Option<String>,
    pub story_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_instruction: Option<String>,
    #[serde(default)]
    pub failures: Vec<GateFailure>,
    #[serde(default)]
    pub metric: Option<f64>,
    #[serde(default)]
    pub constraint_ids: Vec<String>,
}

impl GateRunOutcome {
    pub fn cli_exit_code(&self) -> ExitCode {
        self.status.default_exit_code()
    }

    pub fn to_payload(&self) -> GateRunPayload {
        GateRunPayload {
            gate: self.gate_id.clone(),
            story: self.story_id.clone(),
            commit: self.commit_sha.clone(),
            status: self.status.to_string(),
            exit_code: self.exit_code,
            duration_ms: self.duration_ms,
            summary: self.summary.clone(),
            skipped_locally: self.skipped_locally,
            stdout: self.stdout.clone(),
            stderr: self.stderr.clone(),
            agent_instruction: self.agent_instruction.clone(),
            failures: self.failures.clone(),
            metric: self.metric,
            constraint_ids: self.constraint_ids.clone(),
        }
    }

    /// Console receipt conforming to docs/compliance-and-safety.md §2.
    pub fn receipt(&self) -> String {
        let clean_summary = self.summary.split_whitespace().collect::<Vec<_>>().join(" ");
        match self.status {
            GateStatus::Pass => {
                let sha = self
                    .commit_sha
                    .as_deref()
                    .map(|s| {
                        let char_count = s.chars().count();
                        if char_count >= 7 {
                            s.chars().take(7).collect::<String>()
                        } else {
                            s.to_string()
                        }
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                format!(
                    "[PASS] {} | {} | {} | {}",
                    self.gate_id,
                    clean_summary,
                    sha,
                    format_duration(self.duration_ms)
                )
            }
            GateStatus::Fail => {
                if self.failures.is_empty() {
                    format!(
                        "[FAIL] {} (exit {}) | {}",
                        self.gate_id, self.exit_code, clean_summary
                    )
                } else {
                    let lines: Vec<String> = self
                        .failures
                        .iter()
                        .map(|f| {
                            let first_line = f.message.lines().next().unwrap_or("").trim();
                            if first_line.is_empty() {
                                format!(
                                    "[FAIL] {} (exit {}) | {}",
                                    self.gate_id, self.exit_code, f.location
                                )
                            } else {
                                format!(
                                    "[FAIL] {} (exit {}) | {} | {}",
                                    self.gate_id, self.exit_code, f.location, first_line
                                )
                            }
                        })
                        .collect();
                    lines.join("\n")
                }
            }
            GateStatus::Infra => {
                format!("[INFRA] {} | {} | halt and alert", self.gate_id, clean_summary)
            }
            GateStatus::Skip => {
                format!("[SKIP] {} | skipped_locally", self.gate_id)
            }
        }
    }
}

/// Formats duration for receipts (e.g. "1.2 s" or "50 ms").
pub fn format_duration(duration_ms: u64) -> String {
    if duration_ms >= 1000 {
        format!("{:.1} s", duration_ms as f64 / 1000.0)
    } else {
        format!("{} ms", duration_ms)
    }
}
