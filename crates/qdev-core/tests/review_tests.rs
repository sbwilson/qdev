#![allow(clippy::disallowed_methods)]

use qdev_core::{
    review_epic_with_gates, DeferredWorkRecord, EntityFilter, EntityKind, EntityRecord,
    GateRunRecord, SqliteStore, Store,
};

fn story(id: &str, status: &str) -> EntityRecord {
    EntityRecord {
        id: id.into(),
        kind: EntityKind::Story,
        title: Some(id.into()),
        status: Some(status.into()),
        owners: None,
        source_path: format!("docs/specs/stories/{id}.md"),
        content_hash: "hash".into(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "now".into(),
        stale: false,
        epic_id: Some("E12".into()),
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    }
}

#[test]
fn epic_review_requires_each_bound_gate_evidence() {
    let store = SqliteStore::open_in_memory().unwrap();
    store.upsert_entity(&story("E12S1", "done")).unwrap();
    store
        .upsert_gate_run(&GateRunRecord {
            id: "one".into(),
            story_id: Some("E12S1".into()),
            gate_id: "unit".into(),
            commit_sha: "sha".into(),
            status: Some("pass".into()),
            evidence_path: "docs/state/evidence/E12S1/sha-unit.json".into(),
            ..Default::default()
        })
        .unwrap();
    let required = vec!["unit".into(), "lint".into()];
    let error = review_epic_with_gates(&store, "E12", &required).unwrap_err();
    assert_eq!(error.exit_code().as_i32(), 1);
    assert!(error.message().contains("lint"));
    store
        .upsert_gate_run(&GateRunRecord {
            id: "two".into(),
            story_id: Some("E12S1".into()),
            gate_id: "lint".into(),
            commit_sha: "sha".into(),
            status: Some("pass".into()),
            evidence_path: "docs/state/evidence/E12S1/sha-lint.json".into(),
            ..Default::default()
        })
        .unwrap();
    let review = review_epic_with_gates(&store, "E12", &required).unwrap();
    assert_eq!(review.stories[0].covered_gate_ids, vec!["lint", "unit"]);
}

#[test]
fn epic_review_has_no_fabricated_stories() {
    let store = SqliteStore::open_in_memory().unwrap();
    assert!(store
        .list_entities(&EntityFilter {
            kind: Some(EntityKind::Story),
            epic_id: Some("E404".into()),
            ..Default::default()
        })
        .unwrap()
        .is_empty());
    let review = review_epic_with_gates(&store, "E404", &[]).unwrap();
    assert!(review.stories.is_empty());
}

#[test]
fn epic_review_scopes_open_deferred_work_to_its_stories() {
    let store = SqliteStore::open_in_memory().unwrap();
    store.upsert_entity(&story("E12S1", "ready")).unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-12".into(),
            origin_story_id: Some("E12S1".into()),
            status: Some("open".into()),
            safety_risk: Some("acceptable_with_mitigation".into()),
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-other".into(),
            origin_story_id: Some("E99S1".into()),
            status: Some("open".into()),
            safety_risk: None,
            ..Default::default()
        })
        .unwrap();
    let review = review_epic_with_gates(&store, "E12", &[]).unwrap();
    assert_eq!(
        review
            .open_deferred_work
            .iter()
            .map(|debt| debt.id.as_str())
            .collect::<Vec<_>>(),
        vec!["DW-12"]
    );
    assert!(review.unclassified_debt.is_empty());
}

#[test]
fn epic_review_reports_unclassified_debt() {
    let store = SqliteStore::open_in_memory().unwrap();
    store.upsert_entity(&story("E12S1", "ready")).unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-12".into(),
            origin_story_id: Some("E12S1".into()),
            status: Some("open".into()),
            safety_risk: None,
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-13".into(),
            origin_story_id: Some("E12S1".into()),
            status: Some("open".into()),
            safety_risk: Some("negligible".into()),
            ..Default::default()
        })
        .unwrap();
    // A resolved in-epic DW is no longer open debt, whatever its risk.
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-14".into(),
            origin_story_id: Some("E12S1".into()),
            status: Some("done".into()),
            safety_risk: Some("unacceptable".into()),
            ..Default::default()
        })
        .unwrap();
    let review = review_epic_with_gates(&store, "E12", &[]).unwrap();
    assert_eq!(review.unclassified_debt, vec!["DW-12".to_string()]);
    assert_eq!(
        review
            .open_deferred_work
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        vec!["DW-12", "DW-13"]
    );
}

#[test]
fn epic_review_keeps_stale_stories_and_flags_missing_evidence() {
    let store = SqliteStore::open_in_memory().unwrap();
    let mut done = story("E12S1", "done");
    done.stale = true;
    store.upsert_entity(&done).unwrap();
    // A done story whose file no longer matches the cache is still a done story:
    // with no gate evidence it must fail the audit, not vanish from it.
    let error = review_epic_with_gates(&store, "E12", &[]).unwrap_err();
    assert_eq!(error.exit_code().as_i32(), 1);
    assert!(error.message().contains("E12S1"));

    // With passing evidence the same story is reported, flagged stale.
    store
        .upsert_gate_run(&GateRunRecord {
            id: "one".into(),
            story_id: Some("E12S1".into()),
            gate_id: "unit".into(),
            commit_sha: "sha".into(),
            status: Some("pass".into()),
            evidence_path: "docs/state/evidence/E12S1/sha-unit.json".into(),
            ..Default::default()
        })
        .unwrap();
    let review = review_epic_with_gates(&store, "E12", &[]).unwrap();
    assert_eq!(review.stories.len(), 1);
    assert!(review.stories[0].stale);
}

#[test]
fn epic_review_counts_only_passing_evidence() {
    let store = SqliteStore::open_in_memory().unwrap();
    store.upsert_entity(&story("E12S1", "done")).unwrap();
    store
        .upsert_gate_run(&GateRunRecord {
            id: "fail-run".into(),
            story_id: Some("E12S1".into()),
            gate_id: "unit".into(),
            commit_sha: "sha".into(),
            status: Some("fail".into()),
            evidence_path: "docs/state/evidence/E12S1/sha-unit.json".into(),
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_gate_run(&GateRunRecord {
            id: "pass-run".into(),
            story_id: Some("E12S1".into()),
            gate_id: "lint".into(),
            commit_sha: "sha".into(),
            status: Some("pass".into()),
            evidence_path: "docs/state/evidence/E12S1/sha-lint.json".into(),
            ..Default::default()
        })
        .unwrap();
    let required = vec!["unit".to_string(), "lint".to_string()];
    let error = review_epic_with_gates(&store, "E12", &required).unwrap_err();
    assert!(error.message().contains("unit"));
    store
        .upsert_gate_run(&GateRunRecord {
            id: "pass-run-2".into(),
            story_id: Some("E12S1".into()),
            gate_id: "unit".into(),
            commit_sha: "sha".into(),
            status: Some("pass".into()),
            evidence_path: "docs/state/evidence/E12S1/sha-unit-2.json".into(),
            ..Default::default()
        })
        .unwrap();
    let review = review_epic_with_gates(&store, "E12", &required).unwrap();
    // Failing runs are never evidence: only the two passing paths are listed.
    assert_eq!(
        review.stories[0].evidence_paths,
        vec![
            "docs/state/evidence/E12S1/sha-lint.json",
            "docs/state/evidence/E12S1/sha-unit-2.json"
        ]
    );
}

#[test]
fn review_sprint_requires_active_release_linked_sprint() {
    use qdev_core::config::{Config, StorageConfig};
    use qdev_core::review_sprint;
    use qdev_core::store::SprintRecord;
    use std::fs;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::write(
        root.join("docs/state/releases/0.1.0.md"),
        "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n",
    )
    .unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_sprint(&SprintRecord {
            id: 1,
            status: Some("completed".into()),
            release_version: Some("0.1.0".into()),
            ..Default::default()
        })
        .unwrap();
    let error = review_sprint(
        root,
        &StorageConfig::default(),
        &store,
        &Config::default(),
        1,
    )
    .unwrap_err();
    assert_eq!(error.exit_code().as_i32(), 2);
    assert_eq!(error.code(), "sprint_not_active");

    store
        .upsert_sprint(&SprintRecord {
            id: 2,
            status: Some("active".into()),
            release_version: None,
            ..Default::default()
        })
        .unwrap();
    let error = review_sprint(
        root,
        &StorageConfig::default(),
        &store,
        &Config::default(),
        2,
    )
    .unwrap_err();
    assert!(error.message().contains("release-linked"));
}

#[test]
fn review_sprint_writes_grouped_reports_and_requirement_trace() {
    use qdev_core::config::{Config, StorageConfig};
    use qdev_core::review_sprint;
    use qdev_core::store::{RelationRecord, SprintAssignmentRecord, SprintRecord};
    use std::fs;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::write(
        root.join("docs/state/releases/0.1.0.md"),
        "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n",
    )
    .unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_sprint(&SprintRecord {
            id: 1,
            status: Some("active".into()),
            release_version: Some("0.1.0".into()),
            ..Default::default()
        })
        .unwrap();
    store.upsert_entity(&story("E12S1", "done")).unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S1".into(),
            relation: "traces_to".into(),
            target_id: "REQ-1".into(),
        })
        .unwrap();
    store
        .upsert_sprint_assignment(&SprintAssignmentRecord {
            sprint_id: 1,
            story_id: "E12S1".into(),
            assigned_at: "2026-09-27".into(),
            carried_from: None,
        })
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-1".into(),
            status: Some("open".into()),
            safety_risk: Some("unacceptable".into()),
            ..Default::default()
        })
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-2".into(),
            status: Some("open".into()),
            safety_risk: Some("acceptable_with_mitigation".into()),
            ..Default::default()
        })
        .unwrap();
    // A resolved DW must not appear in the residual-anomaly report.
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-3".into(),
            status: Some("done".into()),
            safety_risk: Some("unacceptable".into()),
            ..Default::default()
        })
        .unwrap();

    let result = review_sprint(
        root,
        &StorageConfig::default(),
        &store,
        &Config::default(),
        1,
    )
    .unwrap();

    let rtm = fs::read_to_string(root.join("docs/state/releases/0.1.0/rtm.md")).unwrap();
    assert!(rtm.contains("| Story | Status | Requirements | Evidence |"));
    assert!(rtm.contains("| E12S1 | done | REQ-1 |"));

    let anomalies =
        fs::read_to_string(root.join("docs/state/releases/0.1.0/anomalies.md")).unwrap();
    assert!(anomalies.contains("## unacceptable"));
    assert!(anomalies.contains("DW-1"));
    assert!(anomalies.contains("## acceptable_with_mitigation"));
    assert!(anomalies.contains("DW-2"));
    assert!(!anomalies.contains("DW-3"));

    // The payload carries the report-ordered open debt, not just file paths.
    assert_eq!(
        result
            .open_deferred_work
            .iter()
            .map(|d| d.id.as_str())
            .collect::<Vec<_>>(),
        vec!["DW-1", "DW-2"]
    );
}

#[test]
fn sprint_review_gate_failure_attaches_attribution() {
    use qdev_core::config::{Config, GateConfig, StorageConfig};
    use qdev_core::review_sprint;
    use qdev_core::store::SprintRecord;
    use std::fs;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::write(
        root.join("docs/state/releases/0.1.0.md"),
        "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n",
    )
    .unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_sprint(&SprintRecord {
            id: 1,
            status: Some("active".into()),
            release_version: Some("0.1.0".into()),
            ..Default::default()
        })
        .unwrap();

    let mut config = Config::default();
    config.gates.push(GateConfig {
        id: "failing-review-gate".to_string(),
        command: Some("false".to_string()),
        timeout_ms: Some(1000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: vec!["sprint_close".to_string()],
        verifies: Vec::new(),
        kind: Some("test".to_string()),
        metric: None,
        direction: None,
        skip: None,
    });

    let error = review_sprint(root, &StorageConfig::default(), &store, &config, 1).unwrap_err();

    assert_eq!(error.code(), "gate_failed");
    let details = error.details().expect("details must be present");
    assert_eq!(details["gate_id"], "failing-review-gate");
    assert!(!details["rule"].as_str().unwrap().is_empty());
}
