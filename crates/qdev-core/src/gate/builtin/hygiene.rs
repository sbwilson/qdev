//! The built-in `qdev-hygiene` gate: runs the hygiene linter over the workspace.

use std::path::Path;
use std::time::Instant;

use crate::config::{Config, GateConfig};
use crate::errors::QdevError;
use crate::gate::git::is_inside_work_tree;
use crate::gate::runner::{
    record_gate_evidence, resolve_commit_sha, resolve_story_id, GateRunOptions,
};
use crate::gate::{GateFailure, GateRunOutcome, GateStatus};

use super::BUILTIN_GATE_HYGIENE;

/// Executes the built-in `qdev-hygiene` verification gate.
pub fn execute_hygiene_gate(
    workspace_root: &Path,
    config: &Config,
    options: &GateRunOptions,
) -> Result<GateRunOutcome, QdevError> {
    let start_time = Instant::now();
    let commit_sha = resolve_commit_sha(workspace_root);
    let story_id = resolve_story_id(workspace_root, config, options);

    let configured_gate = config.gates.iter().find(|g| g.id == BUILTIN_GATE_HYGIENE);
    if configured_gate.and_then(|g| g.skip) == Some(true) {
        let gate_cfg = configured_gate.cloned().unwrap_or_else(|| GateConfig {
            id: BUILTIN_GATE_HYGIENE.to_string(),
            command: None,
            timeout_ms: None,
            depends_on: Vec::new(),
            output_adapter: None,
            on_transition: vec!["review".to_string()],
            verifies: Vec::new(),
            kind: Some("builtin".to_string()),
            metric: None,
            direction: None,
            skip: Some(true),
        });
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            &gate_cfg,
            story_id.as_deref(),
            commit_sha.as_deref(),
            "pass",
            0,
            0,
            None,
            "skipped_locally",
            None,
            None,
            true,
        )?;
        return Ok(GateRunOutcome {
            gate_id: BUILTIN_GATE_HYGIENE.to_string(),
            status: GateStatus::Skip,
            exit_code: 0,
            duration_ms: 0,
            summary: "skipped_locally".to_string(),
            skipped_locally: true,
            commit_sha,
            story_id,
            stdout: None,
            stderr: None,
            agent_instruction: None,
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
            evidence_path: Some(evidence_path),
        });
    }

    let is_git = is_inside_work_tree(workspace_root);

    let hygiene_options = crate::hygiene::HygieneCheckOptions {
        diff: is_git,
        paths: Vec::new(),
    };
    let hygiene_outcome = crate::hygiene::check_hygiene(workspace_root, config, &hygiene_options)?;

    let duration_ms = start_time.elapsed().as_millis() as u64;

    let failures: Vec<GateFailure> = hygiene_outcome
        .findings
        .iter()
        .map(|f| GateFailure {
            location: f.location.clone(),
            message: format!("[{}] {}", f.rule_id, f.excerpt),
        })
        .collect();

    let (status, exit_code, summary, agent_instruction) = if failures.is_empty() {
        (
            GateStatus::Pass,
            0,
            "hygiene check passed: 0 findings".to_string(),
            None,
        )
    } else {
        (
            GateStatus::Fail,
            1,
            format!("hygiene check failed with {} finding(s)", failures.len()),
            Some("fix_cited_failures".to_string()),
        )
    };

    let hygiene_gate_config = configured_gate.cloned().unwrap_or_else(|| GateConfig {
        id: BUILTIN_GATE_HYGIENE.to_string(),
        command: None,
        timeout_ms: None,
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: vec!["review".to_string()],
        verifies: Vec::new(),
        kind: Some("builtin".to_string()),
        metric: None,
        direction: None,
        skip: None,
    });

    let ev_status = match status {
        GateStatus::Pass => "pass",
        GateStatus::Fail => "fail",
        GateStatus::Infra => "infra",
        GateStatus::Skip => "pass",
    };

    let output_text = if failures.is_empty() {
        summary.clone()
    } else {
        let failure_lines: Vec<String> = failures
            .iter()
            .map(|f| format!("{}: {}", f.location, f.message))
            .collect();
        format!("{}\n{}", summary, failure_lines.join("\n"))
    };

    let evidence_path = record_gate_evidence(
        workspace_root,
        config,
        &hygiene_gate_config,
        story_id.as_deref(),
        commit_sha.as_deref(),
        ev_status,
        exit_code,
        duration_ms,
        None,
        &summary,
        Some(&output_text),
        None,
        false,
    )?;

    Ok(GateRunOutcome {
        gate_id: BUILTIN_GATE_HYGIENE.to_string(),
        status,
        exit_code,
        duration_ms,
        summary,
        skipped_locally: false,
        commit_sha,
        story_id,
        stdout: None,
        stderr: None,
        agent_instruction,
        failures,
        metric: None,
        constraint_ids: Vec::new(),
        evidence_path: Some(evidence_path),
    })
}
