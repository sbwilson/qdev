use std::fs;
use std::path::Path;

use sha2::Digest;
use qdev_core::config::{Config, GateConfig, StorageConfig};
use qdev_core::gate::{
    execute_gate, write_evidence_bundle, EvidenceBundle, GateRunOptions, GateStatus,
};
use qdev_core::query::{query_entity, QueryOptions};
use qdev_core::schema::{validate_evidence, EntityKind};
use qdev_core::store::sqlite::SqliteStore;
use qdev_core::store::{EntityRecord, Store};
use qdev_core::write::Author;
use tempfile::TempDir;

fn setup_test_workspace(root: &Path) {
    fs::create_dir_all(root.join("docs/state/evidence")).unwrap();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
}

fn sample_bundle(gate: &str, story: Option<&str>, commit: &str) -> EvidenceBundle {
    EvidenceBundle {
        schema_version: "1".to_string(),
        gate: gate.to_string(),
        story: story.map(str::to_string),
        commit: commit.to_string(),
        status: "pass".to_string(),
        exit_code: 0,
        duration_ms: 120,
        metric: None,
        summary: "10 tests passed".to_string(),
        output_sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_string(),
        run_by: Author {
            author_type: "agent".to_string(),
            id: "claude-code".to_string(),
        },
        ran_at: "2026-09-23T10:00:00Z".to_string(),
        verifies: vec!["FR-102".to_string()],
        skipped_locally: false,
    }
}

fn make_gate_config(
    id: &str,
    command: Option<&str>,
    skip: Option<bool>,
    verifies: Vec<&str>,
) -> GateConfig {
    GateConfig {
        id: id.to_string(),
        command: command.map(str::to_string),
        timeout_ms: Some(10_000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: verifies.into_iter().map(str::to_string).collect(),
        kind: None,
        metric: None,
        direction: None,
        skip,
    }
}

fn base_entity(id: &str, kind: EntityKind) -> EntityRecord {
    EntityRecord {
        id: id.to_string(),
        kind,
        title: Some(format!("{} title", id)),
        status: Some("draft".to_string()),
        owners: None,
        source_path: format!("docs/specs/{}.md", id),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-09-08T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    }
}

// ---------------------------------------------------------------------------
// 1. EvidenceBundle model & schema validation
// ---------------------------------------------------------------------------

#[test]
fn test_evidence_bundle_model_validation_valid() {
    let bundle = sample_bundle("c-abi-round-trip", Some("E12S4"), "8f1b2c4");
    assert!(bundle.validate().is_ok());

    let json_val = serde_json::to_value(&bundle).unwrap();
    assert!(validate_evidence(&json_val).is_ok());
}

#[test]
fn test_evidence_bundle_model_validation_invalid_status() {
    let mut bundle = sample_bundle("c-abi-round-trip", Some("E12S4"), "8f1b2c4");
    bundle.status = "invalid_status".to_string();
    assert!(bundle.validate().is_err());
}

#[test]
fn test_evidence_bundle_model_validation_empty_gate() {
    let bundle = sample_bundle("", Some("E12S4"), "8f1b2c4");
    assert!(bundle.validate().is_err());
}

#[test]
fn test_evidence_bundle_model_validation_empty_summary() {
    let mut bundle = sample_bundle("lint", Some("E12S4"), "8f1b2c4");
    bundle.summary = "".to_string();
    assert!(bundle.validate().is_err());
}

#[test]
fn test_evidence_bundle_model_validation_invalid_output_sha256() {
    let mut bundle = sample_bundle("lint", Some("E12S4"), "8f1b2c4");
    bundle.output_sha256 = "not-a-valid-sha256".to_string();
    assert!(bundle.validate().is_err());
}

// ---------------------------------------------------------------------------
// 2. Collision-free path resolution & suffixing
// ---------------------------------------------------------------------------

#[test]
fn test_collision_free_path_and_suffixing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let storage = StorageConfig::default();

    let bundle1 = sample_bundle("lint", Some("E12S4"), "8f1b2c4");
    let (abs1, rel1, stem1) = write_evidence_bundle(root, &storage, &bundle1).unwrap();
    assert_eq!(rel1, "docs/state/evidence/E12S4/8f1b2c4-lint.json");
    assert_eq!(stem1, "8f1b2c4-lint");
    assert!(abs1.is_file());
    let content1 = fs::read_to_string(&abs1).unwrap();

    // Second write on same commit -> appends -2
    let mut bundle2 = bundle1.clone();
    bundle2.summary = "second run".to_string();
    let (abs2, rel2, stem2) = write_evidence_bundle(root, &storage, &bundle2).unwrap();
    assert_eq!(rel2, "docs/state/evidence/E12S4/8f1b2c4-lint-2.json");
    assert_eq!(stem2, "8f1b2c4-lint-2");
    assert!(abs2.is_file());

    // Original file must NOT be modified
    let content1_after = fs::read_to_string(&abs1).unwrap();
    assert_eq!(content1, content1_after);

    // Third write on same commit -> appends -3
    let mut bundle3 = bundle1.clone();
    bundle3.summary = "third run".to_string();
    let (abs3, rel3, stem3) = write_evidence_bundle(root, &storage, &bundle3).unwrap();
    assert_eq!(rel3, "docs/state/evidence/E12S4/8f1b2c4-lint-3.json");
    assert_eq!(stem3, "8f1b2c4-lint-3");
    assert!(abs3.is_file());
}

#[test]
fn test_evidence_bundle_workspace_level() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let storage = StorageConfig::default();

    let bundle = sample_bundle("format", None, "8f1b2c4");
    let (abs, rel, stem) = write_evidence_bundle(root, &storage, &bundle).unwrap();
    assert_eq!(rel, "docs/state/evidence/_workspace/8f1b2c4-format.json");
    assert_eq!(stem, "8f1b2c4-format");
    assert!(abs.is_file());

    let content = fs::read_to_string(&abs).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(val.get("story").unwrap().is_null());
    assert_eq!(val["gate"], "format");
}

// ---------------------------------------------------------------------------
// 3. execute_gate writes evidence & generates receipt with evidence path
// ---------------------------------------------------------------------------

#[test]
fn test_execute_gate_writes_evidence_and_receipt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let config = Config {
        gates: vec![make_gate_config(
            "echo-pass",
            Some("echo test-output"),
            None,
            vec!["FR-102", "HAZ-01"],
        )],
        ..Default::default()
    };

    let options = GateRunOptions {
        story: Some("E12S4".to_string()),
        ..Default::default()
    };

    let outcome = execute_gate(root, &config, "echo-pass", &options).unwrap();
    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);

    let ev_path_rel = outcome
        .evidence_path
        .clone()
        .expect("evidence_path must be present");
    assert!(ev_path_rel.starts_with("docs/state/evidence/E12S4/"));
    assert!(ev_path_rel.ends_with("-echo-pass.json"));

    // Verify receipt formatting
    let receipt = outcome.receipt();
    assert!(receipt.contains("[PASS] echo-pass"));
    assert!(receipt.contains(&format!("| evidence {}", ev_path_rel)));

    // Verify evidence file content on disk
    let full_path = root.join(&ev_path_rel);
    assert!(full_path.is_file());
    let content = fs::read_to_string(&full_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["gate"], "echo-pass");
    assert_eq!(val["story"], "E12S4");
    assert_eq!(val["status"], "pass");
    assert_eq!(val["verifies"], serde_json::json!(["FR-102", "HAZ-01"]));
    assert_eq!(val["skipped_locally"], false);
    assert_eq!(val["output_sha256"].as_str().unwrap().len(), 64);
    let mut expected_hasher = sha2::Sha256::new();
    if let Some(ref out) = outcome.stdout {
        if !out.is_empty() {
            sha2::Digest::update(&mut expected_hasher, out.as_bytes());
        }
    }
    if let Some(ref err) = outcome.stderr {
        if !err.is_empty() {
            sha2::Digest::update(&mut expected_hasher, err.as_bytes());
        }
    }
    let expected_sha256 = format!("{:x}", sha2::Digest::finalize(expected_hasher));
    assert_eq!(val["output_sha256"], expected_sha256);
    assert!(validate_evidence(&val).is_ok());
}

#[test]
fn test_execute_gate_failure_generates_fail_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let config = Config {
        gates: vec![make_gate_config(
            "fail-gate",
            Some("sh -c 'echo failed-output >&2; exit 101'"),
            None,
            vec!["FR-103"],
        )],
        ..Default::default()
    };

    let options = GateRunOptions {
        story: Some("E12S4".to_string()),
        ..Default::default()
    };

    let outcome = execute_gate(root, &config, "fail-gate", &options).unwrap();
    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 101);

    let ev_path_rel = outcome.evidence_path.expect("evidence_path must be present");
    let full_path = root.join(&ev_path_rel);
    assert!(full_path.is_file());

    let content = fs::read_to_string(&full_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["gate"], "fail-gate");
    assert_eq!(val["story"], "E12S4");
    assert_eq!(val["status"], "fail");
    assert_eq!(val["exit_code"], 101);
    assert_eq!(val["verifies"], serde_json::json!(["FR-103"]));
    assert_eq!(val["skipped_locally"], false);
    assert!(validate_evidence(&val).is_ok());
}

#[test]
fn test_execute_gate_infra_generates_infra_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let config = Config {
        gates: vec![make_gate_config("infra-gate", None, None, vec![])],
        ..Default::default()
    };

    let options = GateRunOptions {
        story: Some("E12S4".to_string()),
        ..Default::default()
    };

    let outcome = execute_gate(root, &config, "infra-gate", &options).unwrap();
    assert_eq!(outcome.status, GateStatus::Infra);
    assert_eq!(outcome.exit_code, 4);

    let ev_path_rel = outcome.evidence_path.expect("evidence_path must be present");
    let full_path = root.join(&ev_path_rel);
    assert!(full_path.is_file());

    let content = fs::read_to_string(&full_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["gate"], "infra-gate");
    assert_eq!(val["story"], "E12S4");
    assert_eq!(val["status"], "infra");
    assert_eq!(val["exit_code"], 4);
    assert!(validate_evidence(&val).is_ok());
}

#[test]
fn test_execute_gate_with_skipped_locally() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let config = Config {
        gates: vec![make_gate_config(
            "skip-gate",
            Some("exit 1"),
            Some(true),
            vec!["FR-99"],
        )],
        ..Default::default()
    };

    let options = GateRunOptions {
        story: Some("E12S4".to_string()),
        ..Default::default()
    };

    let outcome = execute_gate(root, &config, "skip-gate", &options).unwrap();
    assert_eq!(outcome.status, GateStatus::Skip);
    assert!(outcome.skipped_locally);

    let ev_path_rel = outcome.evidence_path.expect("evidence_path must be present");
    let full_path = root.join(&ev_path_rel);
    assert!(full_path.is_file());

    let content = fs::read_to_string(&full_path).unwrap();
    let val: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["gate"], "skip-gate");
    assert_eq!(val["status"], "pass"); // Skipped locally writes status: "pass" per spec
    assert_eq!(val["skipped_locally"], true);
    assert_eq!(val["duration_ms"], 0);
    assert_eq!(val["verifies"], serde_json::json!(["FR-99"]));
}

// ---------------------------------------------------------------------------
// 4. SQLite hydration via rebuild_from_workspace & legacy aliases
// ---------------------------------------------------------------------------

#[test]
fn test_sqlite_hydration_and_get_gate_runs_for_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let storage = StorageConfig::default();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::store::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();

    let evidence_dir = root.join("docs/state/evidence/E12S4");
    fs::create_dir_all(&evidence_dir).unwrap();

    let bundle1 = sample_bundle("lint", Some("E12S4"), "8f1b2c4");
    let file1 = evidence_dir.join("8f1b2c4-lint.json");
    fs::write(&file1, serde_json::to_string(&bundle1).unwrap()).unwrap();

    let mut bundle2 = bundle1.clone();
    bundle2.gate = "format".to_string();
    bundle2.ran_at = "2026-09-23T10:05:00Z".to_string();
    let file2 = evidence_dir.join("8f1b2c4-format.json");
    fs::write(&file2, serde_json::to_string(&bundle2).unwrap()).unwrap();

    // Rebuild cache from workspace to trigger evidence hydration
    store.rebuild_from_workspace(root, &storage).unwrap();

    let runs = store.get_gate_runs_for_story("E12S4").unwrap();
    assert_eq!(runs.len(), 2);
    // Ordered by ran_at DESC
    assert_eq!(runs[0].gate_id, "format");
    assert_eq!(runs[1].gate_id, "lint");
}

#[test]
fn test_sqlite_hydration_legacy_aliases() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let storage = StorageConfig::default();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::store::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();

    let evidence_dir = root.join("docs/state/evidence/E12S4");
    fs::create_dir_all(&evidence_dir).unwrap();

    // Legacy format using gate_id, story_id, commit_sha, metric_value, output_hash
    let legacy_json = serde_json::json!({
        "schema_version": "1",
        "gate_id": "legacy-gate",
        "story_id": "E12S4",
        "commit_sha": "a1b2c3d",
        "status": "pass",
        "exit_code": 0,
        "duration_ms": 250,
        "metric_value": 42.5,
        "summary": "legacy pass",
        "output_hash": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "run_by": {"type": "human", "id": "tester"},
        "ran_at": "2026-09-23T11:00:00Z"
    });

    let legacy_file = evidence_dir.join("a1b2c3d-legacy-gate.json");
    fs::write(&legacy_file, serde_json::to_string(&legacy_json).unwrap()).unwrap();

    store.rebuild_from_workspace(root, &storage).unwrap();

    let runs = store.get_gate_runs_for_story("E12S4").unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].gate_id, "legacy-gate");
    assert_eq!(runs[0].commit_sha, "a1b2c3d");
    assert_eq!(runs[0].metric_value, Some(42.5));
    assert_eq!(
        runs[0].output_hash.as_deref(),
        Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
    );
}

// ---------------------------------------------------------------------------
// 5. Query entity with --expand evidence deduplicates to latest run per gate
// ---------------------------------------------------------------------------

#[test]
fn test_query_entity_expand_evidence_deduplicates_latest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::store::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();

    let story_entity = EntityRecord {
        epic_id: Some("E12".to_string()),
        seq: Some(4),
        title: Some("Test Story".to_string()),
        status: Some("in_progress".to_string()),
        safety_class: Some("ClassB".to_string()),
        owners: Some("[\"dev\"]".to_string()),
        ..base_entity("E12S4", EntityKind::Story)
    };
    store.upsert_entity(&story_entity).unwrap();

    // Insert 3 runs for gate "lint" and 1 run for gate "fmt"
    let run1 = qdev_core::store::GateRunRecord {
        id: "sha1-lint".to_string(),
        story_id: Some("E12S4".to_string()),
        gate_id: "lint".to_string(),
        commit_sha: "sha0001".to_string(),
        status: Some("fail".to_string()),
        exit_code: Some(1),
        duration_ms: Some(100),
        metric_value: None,
        summary: Some("old run 1".to_string()),
        evidence_path: "docs/state/evidence/E12S4/sha0001-lint.json".to_string(),
        output_hash: None,
        run_by_type: Some("human".to_string()),
        run_by_id: Some("dev".to_string()),
        ran_at: Some("2026-09-23T10:00:00Z".to_string()),
    };
    let run2 = qdev_core::store::GateRunRecord {
        id: "sha2-lint".to_string(),
        story_id: Some("E12S4".to_string()),
        gate_id: "lint".to_string(),
        commit_sha: "sha0002".to_string(),
        status: Some("pass".to_string()),
        exit_code: Some(0),
        duration_ms: Some(120),
        metric_value: None,
        summary: Some("latest lint run".to_string()),
        evidence_path: "docs/state/evidence/E12S4/sha0002-lint.json".to_string(),
        output_hash: None,
        run_by_type: Some("human".to_string()),
        run_by_id: Some("dev".to_string()),
        ran_at: Some("2026-09-23T10:10:00Z".to_string()),
    };
    let run3 = qdev_core::store::GateRunRecord {
        id: "sha1-fmt".to_string(),
        story_id: Some("E12S4".to_string()),
        gate_id: "fmt".to_string(),
        commit_sha: "sha0001".to_string(),
        status: Some("pass".to_string()),
        exit_code: Some(0),
        duration_ms: Some(80),
        metric_value: None,
        summary: Some("fmt run".to_string()),
        evidence_path: "docs/state/evidence/E12S4/sha0001-fmt.json".to_string(),
        output_hash: None,
        run_by_type: Some("human".to_string()),
        run_by_id: Some("dev".to_string()),
        ran_at: Some("2026-09-23T10:05:00Z".to_string()),
    };

    store.upsert_gate_run(&run1).unwrap();
    store.upsert_gate_run(&run2).unwrap();
    store.upsert_gate_run(&run3).unwrap();

    // Query with expand_evidence: true
    let opts = QueryOptions {
        expand_evidence: true,
        ..Default::default()
    };
    let res = query_entity(&store, Some(qdev_core::schema::EntityKind::Story), "E12S4", &opts).unwrap();
    match res {
        qdev_core::GetResult::Entity(p) => {
            let ev = p.evidence.expect("evidence must be present when expand_evidence: true");
            // Deduplicated to 2 gates: fmt and lint (sorted ascending by gate)
            assert_eq!(ev.len(), 2);
            assert_eq!(ev[0].gate, "fmt");
            assert_eq!(ev[1].gate, "lint");
            // The lint run must be run2 (the latest one)
            assert_eq!(ev[1].summary.as_deref(), Some("latest lint run"));
            assert_eq!(ev[1].commit, "sha0002");
        }
        _ => panic!("expected entity projection"),
    }

    // Query without expand_evidence
    let opts_no_ev = QueryOptions {
        expand_evidence: false,
        ..Default::default()
    };
    let res_no_ev = query_entity(&store, Some(qdev_core::schema::EntityKind::Story), "E12S4", &opts_no_ev).unwrap();
    match res_no_ev {
        qdev_core::GetResult::Entity(p) => {
            assert!(p.evidence.is_none());
        }
        _ => panic!("expected entity projection"),
    }
}
