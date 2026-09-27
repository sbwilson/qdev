//! Release review projections and deterministic release reports.
//!
//! `review_epic` is a read-only projection. `review_sprint` additionally executes the
//! configured `sprint_close` gates and writes the requested release reports; those writes
//! run under the advisory workspace write lock, and the gate runs are attributed to the
//! workspace (never to an unrelated leased story).

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{Config, StorageConfig};
use crate::errors::QdevError;
use crate::gate::{execute_gate_set, resolve_gate_execution_order, GateRunOptions, GateStatus};
use crate::schema::EntityKind;
use crate::store::{DeferredWorkRecord, EntityFilter, Store};
use crate::write::{
    acquire_workspace_write_lock, resolve_entity_file, workspace_rel_path, write_file_atomic,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpicReview {
    pub epic_id: String,
    pub stories: Vec<ReviewStory>,
    pub open_deferred_work: Vec<ReviewDebt>,
    pub unclassified_debt: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewStory {
    pub id: String,
    pub status: String,
    pub evidence_paths: Vec<String>,
    pub covered_gate_ids: Vec<String>,
    pub missing_required_gate_ids: Vec<String>,
    /// `true` when the story's cached row is stale (its file no longer matches the cache).
    /// Stale stories are reported, not dropped: an audit that silently omits an unreadable
    /// `done` story would let it escape the missing-evidence check.
    pub stale: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewDebt {
    pub id: String,
    pub origin_story_id: Option<String>,
    pub safety_risk: Option<String>,
    pub rationale: Option<String>,
}

/// Audits an epic from the hydrated store. A completed story without any evidence is a
/// logical failure; callers receive the useful projection in error details.
pub fn review_epic(store: &dyn Store, epic_id: &str) -> Result<EpicReview, QdevError> {
    review_epic_with_gates(store, epic_id, &[])
}

/// As [`review_epic`], additionally requiring evidence for every gate bound to story review.
pub fn review_epic_with_gates(
    store: &dyn Store,
    epic_id: &str,
    required_gate_ids: &[String],
) -> Result<EpicReview, QdevError> {
    let stories = store.list_entities(&EntityFilter {
        kind: Some(EntityKind::Story),
        epic_id: Some(epic_id.to_string()),
        ..Default::default()
    })?;
    let debt = store.list_deferred_work()?;
    let mut projection = EpicReview {
        epic_id: epic_id.to_string(),
        stories: Vec::new(),
        open_deferred_work: Vec::new(),
        unclassified_debt: Vec::new(),
    };
    for story in stories {
        let runs = store.get_gate_runs_for_story(&story.id)?;
        // Evidence and coverage count passing runs only; a failing or skipped run is not
        // evidence that a bound gate passed.
        let mut evidence_paths = Vec::new();
        let mut covered_gate_ids = Vec::new();
        for run in runs {
            if run.status.as_deref() == Some("pass") {
                evidence_paths.push(run.evidence_path);
                covered_gate_ids.push(run.gate_id);
            }
        }
        evidence_paths.sort();
        covered_gate_ids.sort();
        covered_gate_ids.dedup();
        let missing_required_gate_ids = required_gate_ids
            .iter()
            .filter(|gate| !covered_gate_ids.contains(gate))
            .cloned()
            .collect::<Vec<_>>();
        let status = story
            .status
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let lacks_evidence = if required_gate_ids.is_empty() {
            evidence_paths.is_empty()
        } else {
            !missing_required_gate_ids.is_empty()
        };
        if status == "done" && lacks_evidence {
            let detail = if required_gate_ids.is_empty() {
                format!("Done story '{}' has no gate evidence", story.id)
            } else {
                format!(
                    "Done story '{}' lacks required bound-gate evidence: {}",
                    story.id,
                    missing_required_gate_ids.join(", ")
                )
            };
            return Err(QdevError::logical_failure("missing_gate_evidence", detail)
                .with_details(serde_json::to_value(&projection).unwrap_or_default()));
        }
        projection.stories.push(ReviewStory {
            id: story.id,
            status,
            evidence_paths,
            covered_gate_ids,
            missing_required_gate_ids,
            stale: story.stale,
        });
    }
    let epic_story_ids = projection
        .stories
        .iter()
        .map(|story| story.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    // The DW status vocabulary is `open`/`done`/`wont_fix` ("closed" is not a DW status);
    // only `open` (or an unset status, which hydrates as open) is still open.
    for item in debt.into_iter().filter(|d| {
        d.status.as_deref().unwrap_or("open") == "open"
            && d.origin_story_id
                .as_deref()
                .is_some_and(|story| epic_story_ids.contains(story))
    }) {
        if item.safety_risk.is_none() {
            projection.unclassified_debt.push(item.id.clone());
        }
        projection.open_deferred_work.push(ReviewDebt {
            id: item.id,
            origin_story_id: item.origin_story_id,
            safety_risk: item.safety_risk,
            rationale: item.rationale,
        });
    }
    projection.stories.sort_by(|a, b| a.id.cmp(&b.id));
    projection
        .open_deferred_work
        .sort_by(|a, b| a.id.cmp(&b.id));
    projection.unclassified_debt.sort();
    Ok(projection)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintReview {
    pub sprint_id: i64,
    pub release: String,
    pub rtm_path: String,
    pub anomalies_path: String,
    pub gate_runs: Vec<String>,
    /// Open deferred work surfaced in `anomalies.md`, in report order (risk, then id).
    pub open_deferred_work: Vec<ReviewDebt>,
}

/// Requirement IDs a story traces to via `traces_to` relations (AD-8: the RTM maps
/// requirements to stories, gate runs, and evidence paths).
fn requirements_for_story(store: &dyn Store, story_id: &str) -> Result<Vec<String>, QdevError> {
    let mut reqs = store
        .get_relations_for_source(story_id)?
        .into_iter()
        .filter(|r| r.relation == "traces_to")
        .map(|r| r.target_id)
        .collect::<Vec<_>>();
    reqs.sort();
    reqs.dedup();
    Ok(reqs)
}

/// Runs configured `sprint_close` gates and writes deterministic release reports
/// (`rtm.md`, `anomalies.md`) under the linked release. Requires an active,
/// release-linked sprint; gate runs are attributed to the workspace, and the report
/// writes run under the advisory workspace write lock.
pub fn review_sprint(
    workspace_root: &Path,
    storage: &StorageConfig,
    store: &dyn Store,
    config: &Config,
    sprint: i64,
) -> Result<SprintReview, QdevError> {
    let sprint_record = store.get_sprint(sprint)?.ok_or_else(|| {
        QdevError::usage_error_with_code("sprint_not_found", format!("Sprint {} not found", sprint))
    })?;
    if sprint_record.status.as_deref() != Some("active") {
        return Err(QdevError::usage_error_with_code(
            "sprint_not_active",
            format!(
                "Sprint {} is {:?}, not active; sprint review requires an active sprint",
                sprint, sprint_record.status
            ),
        ));
    }
    let release = sprint_record
        .release_version
        .filter(|r| !r.trim().is_empty())
        .ok_or_else(|| QdevError::usage_error("Sprint review requires a release-linked sprint"))?;
    // Ensure the linked release exists before any report is emitted.
    resolve_entity_file(
        workspace_root,
        Some(EntityKind::Release),
        &release,
        Some(storage),
    )?;
    let ids: Vec<String> = config
        .gates
        .iter()
        .filter(|g| g.on_transition.iter().any(|t| t == "sprint_close"))
        .map(|g| g.id.clone())
        .collect();
    let outcomes = if ids.is_empty() {
        Vec::new()
    } else {
        let order = resolve_gate_execution_order(&config.gates, Some(&ids))?;
        // Workspace-level: these runs belong to the sprint review, not to any leased story.
        execute_gate_set(
            workspace_root,
            config,
            &order,
            &GateRunOptions {
                workspace_level: true,
                ..Default::default()
            },
        )?
        .outcomes
    };
    for outcome in &outcomes {
        if outcome.status == GateStatus::Fail {
            return Err(
                QdevError::logical_failure("gate_failed", outcome.summary.clone())
                    .with_details(serde_json::to_value(outcome.to_payload()).unwrap_or_default()),
            );
        }
        if outcome.status == GateStatus::Infra {
            return Err(QdevError::infrastructure_failure(
                "gate_infra_failure",
                outcome.summary.clone(),
            )
            .with_details(serde_json::to_value(outcome.to_payload()).unwrap_or_default()));
        }
    }
    let mut assignments = store.get_sprint_assignments(sprint)?;
    assignments.sort_by(|a, b| a.story_id.cmp(&b.story_id));
    let mut rtm_report = String::from(
        "# Requirements Traceability Matrix\n\n| Story | Status | Requirements | Evidence |\n|---|---|---|---|\n",
    );
    for assignment in assignments {
        let entity = store.get_live_entity_for_derivation(&assignment.story_id)?;
        let status = entity
            .and_then(|e| e.status)
            .unwrap_or_else(|| "missing".to_string());
        let requirements = requirements_for_story(store, &assignment.story_id)?.join(", ");
        let mut evidence = store
            .get_gate_runs_for_story(&assignment.story_id)?
            .into_iter()
            .map(|r| r.evidence_path)
            .collect::<Vec<_>>();
        evidence.sort();
        rtm_report.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            assignment.story_id,
            status,
            requirements,
            evidence.join(", ")
        ));
    }
    // Workspace-level runs executed by this review: the RTM must surface the very gate
    // runs it just performed (they are attributed to the workspace, not to a story row).
    let workspace_runs = if ids.is_empty() {
        Vec::new()
    } else {
        store
            .list_gate_runs()?
            .into_iter()
            .filter(|r| r.story_id.is_none() && ids.iter().any(|g| g == &r.gate_id))
            .collect::<Vec<_>>()
    };
    if !workspace_runs.is_empty() {
        rtm_report.push_str(
            "\n### Sprint-level gate runs\n\n| Gate | Status | Summary | Evidence |\n|---|---|---|---|\n",
        );
        for run in &workspace_runs {
            rtm_report.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                run.gate_id,
                run.status.as_deref().unwrap_or("unknown"),
                run.summary.clone().unwrap_or_default().replace('|', "\\|"),
                run.evidence_path
            ));
        }
    }

    // Residual anomalies: open deferred work (the DW vocabulary is `open`/`done`/`wont_fix`),
    // grouped by safety risk, most severe first.
    let mut open = store
        .list_deferred_work()?
        .into_iter()
        .filter(|d| d.status.as_deref().unwrap_or("open") == "open")
        .collect::<Vec<_>>();
    let risk_rank = |risk: &str| match risk {
        "unacceptable" => 0,
        "acceptable_with_mitigation" => 1,
        "negligible" => 2,
        _ => 3,
    };
    let risk_label = |d: &DeferredWorkRecord| {
        d.safety_risk
            .clone()
            .unwrap_or_else(|| "unclassified".to_string())
    };
    open.sort_by(|a, b| {
        let ra = risk_label(a);
        let rb = risk_label(b);
        risk_rank(&ra)
            .cmp(&risk_rank(&rb))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut anomalies_report = String::from("# Residual Anomalies\n");
    if open.is_empty() {
        anomalies_report.push_str("\n_(no open deferred work)_\n");
    } else {
        let mut groups: Vec<(String, Vec<DeferredWorkRecord>)> = Vec::new();
        for d in &open {
            let label = risk_label(d);
            match groups.iter_mut().find(|(l, _)| *l == label) {
                Some((_, g)) => g.push(d.clone()),
                None => groups.push((label, vec![d.clone()])),
            }
        }
        for (risk, group) in groups {
            anomalies_report.push_str(&format!("\n## {}\n\n", risk));
            anomalies_report.push_str("| Deferred work | Rationale |\n|---|---|\n");
            for d in group {
                anomalies_report.push_str(&format!(
                    "| {} | {} |\n",
                    d.id,
                    d.rationale.unwrap_or_default()
                ));
            }
        }
    }
    // Gate execution already takes the workspace write lock internally for each evidence
    // write (the file lock is not reentrant); take it only around the report writes below
    // so a concurrent sprint close cannot interleave with them.
    let _lock_guard = acquire_workspace_write_lock(workspace_root, Some(storage))?;

    let report_dir = workspace_root
        .join(&storage.state_dir)
        .join("releases")
        .join(&release);
    fs::create_dir_all(&report_dir)
        .map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?;
    let rtm_path = report_dir.join("rtm.md");
    let anomalies_path = report_dir.join("anomalies.md");
    write_file_atomic(&rtm_path, &rtm_report)?;
    write_file_atomic(&anomalies_path, &anomalies_report)?;
    let open_deferred_work = open
        .iter()
        .map(|d| ReviewDebt {
            id: d.id.clone(),
            origin_story_id: d.origin_story_id.clone(),
            safety_risk: d.safety_risk.clone(),
            rationale: d.rationale.clone(),
        })
        .collect();
    Ok(SprintReview {
        sprint_id: sprint,
        release,
        rtm_path: workspace_rel_path(&rtm_path, workspace_root),
        anomalies_path: workspace_rel_path(&anomalies_path, workspace_root),
        gate_runs: outcomes.into_iter().map(|o| o.gate_id).collect(),
        open_deferred_work,
    })
}
