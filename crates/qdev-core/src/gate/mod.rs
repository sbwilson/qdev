//! Module root for gate execution models, options, outcomes, and runner.

pub mod adapter;
pub mod builtin;
pub mod git;
pub mod process;
pub mod result;
pub mod ring_buffer;
pub mod runner;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::{ExitCode, QdevError};
use crate::write::{acquire_workspace_write_lock, write_file_atomic, Author};

pub use adapter::{parse_with_adapter, validate_adapter_name, VALID_ADAPTERS};
pub use result::GateResultDocument;
pub use ring_buffer::HeadTailBuffer;
pub use runner::{
    execute_configured_command, execute_deps_gate, execute_gate, execute_gate_set,
    execute_hygiene_gate, execute_scope_gate, get_gate_list, resolve_commit_sha,
    resolve_gate_execution_order, scan_rust_imports, scan_swift_imports,
    validate_gate_dependencies, GateRunOptions, BUILTIN_GATE_DEPS, BUILTIN_GATE_HYGIENE,
    BUILTIN_GATE_SCOPE,
};

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<String>,
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
            evidence_path: self.evidence_path.clone(),
        }
    }

    /// Console receipt conforming to docs/compliance-and-safety.md §2.
    pub fn receipt(&self) -> String {
        let clean_summary = self
            .summary
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        match self.status {
            GateStatus::Pass => {
                let sha = self
                    .commit_sha
                    .as_deref()
                    .map(|s| {
                        let char_count = s.chars().count();
                        if char_count >= 7 {
                            s.chars().take(7).collect::<String>()
                        } else if !s.trim().is_empty() {
                            s.to_string()
                        } else {
                            "unknown".to_string()
                        }
                    })
                    .unwrap_or_else(|| "unknown".to_string());
                let mut line = format!(
                    "[PASS] {} | {} | {} | {}",
                    self.gate_id,
                    clean_summary,
                    sha,
                    format_duration(self.duration_ms)
                );
                if let Some(ref path) = self.evidence_path {
                    line.push_str(&format!(" | evidence {}", path));
                }
                line
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
                format!(
                    "[INFRA] {} | {} | halt and alert",
                    self.gate_id, clean_summary
                )
            }
            GateStatus::Skip => {
                if clean_summary.is_empty() {
                    format!("[SKIP] {} | skipped_locally", self.gate_id)
                } else {
                    format!("[SKIP] {} | {}", self.gate_id, clean_summary)
                }
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

/// Single gate item representation for gate list output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateListItem {
    pub id: String,
    pub kind: String,
    pub transitions: Vec<String>,
    pub dependencies: Vec<String>,
    pub last_status: Option<String>,
}

/// Output payload of `qdev gate list --json` matching payload-gate-list.json schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateListPayload {
    pub gates: Vec<GateListItem>,
}

/// Outcome of running a set of gates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateRunSetOutcome {
    pub outcomes: Vec<GateRunOutcome>,
}

impl GateRunSetOutcome {
    pub fn new(outcomes: Vec<GateRunOutcome>) -> Self {
        Self { outcomes }
    }

    /// Computes aggregate exit code: 1 if any gate had status fail; 4 if any gate had status infra and none failed; otherwise 0.
    pub fn aggregate_exit_code(&self) -> ExitCode {
        if self.outcomes.iter().any(|o| o.status == GateStatus::Fail) {
            ExitCode::LogicalFailure
        } else if self.outcomes.iter().any(|o| o.status == GateStatus::Infra) {
            ExitCode::InfrastructureFailure
        } else {
            ExitCode::Success
        }
    }

    pub fn to_payload(&self) -> GateRunSetPayload {
        GateRunSetPayload {
            runs: self.outcomes.iter().map(|o| o.to_payload()).collect(),
        }
    }

    /// Renders receipts for all evaluated gates in execution order.
    pub fn receipts(&self) -> String {
        self.outcomes
            .iter()
            .map(|o| o.receipt())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Output payload of a multi-gate run matching payload-gate-set.json schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateRunSetPayload {
    pub runs: Vec<GateRunPayload>,
}

/// Ratchet gate metric regression direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RatchetDirection {
    MustNotIncrease,
    MustNotDecrease,
}

impl RatchetDirection {
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "must_not_increase" => Some(Self::MustNotIncrease),
            "must_not_decrease" => Some(Self::MustNotDecrease),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MustNotIncrease => "must_not_increase",
            Self::MustNotDecrease => "must_not_decrease",
        }
    }
}

impl std::fmt::Display for RatchetDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Committed baseline metadata stored in `docs/state/baselines/<branch>/<gate>.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RatchetBaseline {
    pub gate: String,
    pub metric: String,
    pub direction: String,
    pub value: f64,
    pub commit: String,
    pub author: Author,
    pub timestamp: String,
}

/// Output payload of `qdev gate baseline <id> [--json]` matching payload-gate-baseline.json schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GateBaselinePayload {
    pub gate: String,
    pub branch: String,
    pub exists: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

impl GateBaselinePayload {
    pub fn from_baseline(branch: String, baseline: RatchetBaseline) -> Self {
        Self {
            gate: baseline.gate,
            branch,
            exists: true,
            metric: Some(baseline.metric),
            direction: Some(baseline.direction),
            value: Some(baseline.value),
            commit: Some(baseline.commit),
            author: Some(baseline.author),
            timestamp: Some(baseline.timestamp),
        }
    }

    pub fn missing(gate: String, branch: String) -> Self {
        Self {
            gate,
            branch,
            exists: false,
            metric: None,
            direction: None,
            value: None,
            commit: None,
            author: None,
            timestamp: None,
        }
    }
}

/// Result of evaluating a numeric metric against a branch ratchet baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct RatchetEvaluation {
    pub status: GateStatus,
    pub exit_code: i32,
    pub summary: String,
    pub delta: Option<f64>,
    pub is_regression: bool,
}

/// Formats a numeric metric or delta concisely (integers without decimal point, floats as-is).
pub fn format_metric_number(n: f64) -> String {
    if n == 0.0 {
        "0".to_string()
    } else if n.fract() == 0.0 && !n.is_infinite() {
        format!("{:.0}", n)
    } else {
        format!("{}", n)
    }
}

/// Resolves the absolute path to a gate's baseline JSON file for a given integration branch.
pub fn resolve_baseline_path(
    workspace_root: &Path,
    state_dir: &str,
    integration_branch: &str,
    gate_id: &str,
) -> PathBuf {
    workspace_root
        .join(state_dir)
        .join("baselines")
        .join(integration_branch)
        .join(format!("{}.json", gate_id))
}

/// Reads a committed baseline from disk if present.
pub fn read_baseline(
    workspace_root: &Path,
    state_dir: &str,
    integration_branch: &str,
    gate_id: &str,
) -> Result<Option<RatchetBaseline>, QdevError> {
    let baseline_path =
        resolve_baseline_path(workspace_root, state_dir, integration_branch, gate_id);
    if !baseline_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&baseline_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to read baseline file '{}': {}",
                baseline_path.display(),
                e
            ),
        )
    })?;
    let baseline: RatchetBaseline = serde_json::from_str(&content).map_err(|e| {
        QdevError::logical_failure(
            "invalid_baseline",
            format!(
                "Failed to parse baseline file '{}': {}",
                baseline_path.display(),
                e
            ),
        )
    })?;
    Ok(Some(baseline))
}

/// Writes a committed baseline file atomically under the workspace write lock.
pub fn write_baseline(
    workspace_root: &Path,
    storage: &StorageConfig,
    integration_branch: &str,
    baseline: &RatchetBaseline,
) -> Result<PathBuf, QdevError> {
    let _lock = acquire_workspace_write_lock(workspace_root, Some(storage))?;
    let baseline_path = resolve_baseline_path(
        workspace_root,
        &storage.state_dir,
        integration_branch,
        &baseline.gate,
    );
    if let Some(parent) = baseline_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create baseline directory '{}': {}",
                    parent.display(),
                    e
                ),
            )
        })?;
    }
    let mut content = serde_json::to_string_pretty(baseline).map_err(|e| {
        QdevError::infrastructure_failure(
            "json_serialize_error",
            format!("Failed to serialize baseline: {}", e),
        )
    })?;
    if !content.ends_with('\n') {
        content.push('\n');
    }
    write_file_atomic(&baseline_path, &content)?;
    Ok(baseline_path)
}

/// Evaluates a numeric metric against an optional baseline following direction rules.
pub fn evaluate_ratchet(
    current_metric: f64,
    baseline: Option<&RatchetBaseline>,
    direction_str: &str,
    metric_name: &str,
    branch: &str,
) -> Result<RatchetEvaluation, QdevError> {
    if !current_metric.is_finite() {
        return Err(QdevError::usage_error(format!(
            "metric '{}' value must be a finite number, got {}",
            metric_name, current_metric
        )));
    }
    if let Some(b) = baseline {
        if !b.value.is_finite() {
            return Err(QdevError::usage_error(format!(
                "baseline value for gate '{}' must be a finite number, got {}",
                b.gate, b.value
            )));
        }
    }

    let dir = RatchetDirection::from_str_loose(direction_str).ok_or_else(|| {
        QdevError::usage_error(format!(
            "ratchet gate requires direction 'must_not_increase' or 'must_not_decrease', got '{}'",
            direction_str
        ))
    })?;

    match baseline {
        None => {
            let summary = format!(
                "ratchet passed with no baseline ({}: {}, baseline: null) [warning: no baseline recorded for branch '{}']",
                metric_name,
                format_metric_number(current_metric),
                branch
            );
            Ok(RatchetEvaluation {
                status: GateStatus::Pass,
                exit_code: 0,
                summary,
                delta: None,
                is_regression: false,
            })
        }
        Some(b) => {
            let delta = current_metric - b.value;
            let is_regression = match dir {
                RatchetDirection::MustNotIncrease => current_metric > b.value,
                RatchetDirection::MustNotDecrease => current_metric < b.value,
            };

            if is_regression {
                let delta_str = match dir {
                    RatchetDirection::MustNotIncrease => {
                        format!("+{}", format_metric_number(delta))
                    }
                    RatchetDirection::MustNotDecrease => format_metric_number(delta),
                };
                let action = match dir {
                    RatchetDirection::MustNotIncrease => "increased from",
                    RatchetDirection::MustNotDecrease => "decreased from",
                };
                let summary = format!(
                    "ratchet regression: {} {} {} to {} (delta: {})",
                    metric_name,
                    action,
                    format_metric_number(b.value),
                    format_metric_number(current_metric),
                    delta_str
                );
                Ok(RatchetEvaluation {
                    status: GateStatus::Fail,
                    exit_code: 1,
                    summary,
                    delta: Some(delta),
                    is_regression: true,
                })
            } else {
                let delta_str = if delta > 0.0 {
                    format!("+{}", format_metric_number(delta))
                } else {
                    format_metric_number(delta)
                };
                let summary = format!(
                    "ratchet passed: {} is {} (baseline: {}, delta: {})",
                    metric_name,
                    format_metric_number(current_metric),
                    format_metric_number(b.value),
                    delta_str
                );
                Ok(RatchetEvaluation {
                    status: GateStatus::Pass,
                    exit_code: 0,
                    summary,
                    delta: Some(delta),
                    is_regression: false,
                })
            }
        }
    }
}

/// Immutable, committed JSON evidence bundle written on gate completion conforming to
/// docs/compliance-and-safety.md §4.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidenceBundle {
    pub schema_version: String,
    pub gate: String,
    pub story: Option<String>,
    pub commit: String,
    pub status: String,
    pub exit_code: i32,
    pub duration_ms: u64,
    pub metric: Option<f64>,
    pub summary: String,
    pub output_sha256: String,
    pub run_by: Author,
    pub ran_at: String,
    pub verifies: Vec<String>,
    pub skipped_locally: bool,
}

impl EvidenceBundle {
    /// Validates the evidence bundle against the embedded evidence schema.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let value = serde_json::to_value(self).map_err(|e| vec![e.to_string()])?;
        crate::schema::validate_evidence(&value)
    }
}

/// Resolves a collision-free filename and paths for an evidence bundle.
///
/// Directory: `<workspace_root>/<state_dir>/evidence/<story-or-_workspace>/`
/// Filename: `<sha>-<gate>.json` (or `<sha>-<gate>-2.json`, `-3.json`, etc.)
/// Returns `(absolute_path, workspace_relative_path, filename_stem)`.
pub fn resolve_collision_free_evidence_path(
    workspace_root: &Path,
    state_dir: &str,
    story_id: Option<&str>,
    short_sha: &str,
    gate_id: &str,
) -> (PathBuf, String, String) {
    let target = story_id
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && !s.contains('/') && !s.contains('\\') && !s.contains(".."))
        .unwrap_or("_workspace");
    let clean_state = state_dir.trim_matches('/').replace('\\', "/");
    let dir = workspace_root
        .join(&clean_state)
        .join("evidence")
        .join(target);
    let base_name = format!("{}-{}", short_sha, gate_id);

    let first_cand = format!("{}.json", base_name);
    let first_path = dir.join(&first_cand);
    if !first_path.exists() {
        let rel = format!("{}/evidence/{}/{}", clean_state, target, first_cand);
        return (first_path, rel, base_name);
    }

    let mut n = 2;
    loop {
        let suffixed_stem = format!("{}-{}", base_name, n);
        let cand = format!("{}.json", suffixed_stem);
        let path = dir.join(&cand);
        if !path.exists() {
            let rel = format!("{}/evidence/{}/{}", clean_state, target, cand);
            return (path, rel, suffixed_stem);
        }
        n += 1;
    }
}

/// Writes an immutable evidence bundle JSON file under workspace write lock.
/// Never overwrites an existing file; uses collision-free resolution.
pub fn write_evidence_bundle(
    workspace_root: &Path,
    storage: &StorageConfig,
    bundle: &EvidenceBundle,
) -> Result<(PathBuf, String, String), QdevError> {
    bundle.validate().map_err(|errs| {
        QdevError::logical_failure(
            "invalid_evidence_bundle",
            format!(
                "Evidence bundle failed schema validation: {}",
                errs.join("; ")
            ),
        )
    })?;

    let _lock = acquire_workspace_write_lock(workspace_root, Some(storage))?;

    let (abs_path, rel_path, stem) = resolve_collision_free_evidence_path(
        workspace_root,
        &storage.state_dir,
        bundle.story.as_deref(),
        &bundle.commit,
        &bundle.gate,
    );

    if let Some(parent) = abs_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create evidence directory '{}': {}",
                    parent.display(),
                    e
                ),
            )
        })?;
    }

    let mut content = serde_json::to_string_pretty(bundle).map_err(|e| {
        QdevError::infrastructure_failure(
            "json_serialize_error",
            format!("Failed to serialize evidence bundle: {}", e),
        )
    })?;
    if !content.ends_with('\n') {
        content.push('\n');
    }

    write_file_atomic(&abs_path, &content)?;
    Ok((abs_path, rel_path, stem))
}
