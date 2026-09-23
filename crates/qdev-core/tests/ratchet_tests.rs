use std::fs;
use std::path::Path;

use qdev_core::config::{Config, GateConfig, StorageConfig};
use qdev_core::gate::{
    evaluate_ratchet, execute_gate, format_metric_number, read_baseline, resolve_baseline_path,
    write_baseline, GateRunOptions, GateStatus, RatchetBaseline, RatchetDirection,
};
use qdev_core::write::Author;
use tempfile::TempDir;

fn setup_test_workspace(root: &Path) {
    fs::create_dir_all(root.join("docs/state/baselines/main")).unwrap();
    fs::create_dir_all(root.join("docs/specs")).unwrap();
}

fn sample_baseline(gate: &str, metric: &str, direction: &str, value: f64) -> RatchetBaseline {
    RatchetBaseline {
        gate: gate.to_string(),
        metric: metric.to_string(),
        direction: direction.to_string(),
        value,
        commit: "8f1b2c4d5e6f".to_string(),
        author: Author {
            author_type: "human".to_string(),
            id: "simon".to_string(),
        },
        timestamp: "2026-09-23T06:30:00Z".to_string(),
    }
}

// ---------------------------------------------------------------------------
// 1. evaluate_ratchet unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_evaluate_ratchet_missing_baseline() {
    let eval = evaluate_ratchet(10.0, None, "must_not_increase", "warnings", "main").unwrap();
    assert_eq!(eval.status, GateStatus::Pass);
    assert_eq!(eval.exit_code, 0);
    assert!(!eval.is_regression);
    assert_eq!(eval.delta, None);
    assert_eq!(
        eval.summary,
        "ratchet passed with no baseline (warnings: 10, baseline: null) [warning: no baseline recorded for branch 'main']"
    );
}

#[test]
fn test_evaluate_ratchet_must_not_increase_no_regression() {
    let baseline = sample_baseline("warning-count", "warnings", "must_not_increase", 12.0);
    let eval = evaluate_ratchet(
        10.0,
        Some(&baseline),
        "must_not_increase",
        "warnings",
        "main",
    )
    .unwrap();
    assert_eq!(eval.status, GateStatus::Pass);
    assert_eq!(eval.exit_code, 0);
    assert!(!eval.is_regression);
    assert_eq!(eval.delta, Some(-2.0));
    assert_eq!(
        eval.summary,
        "ratchet passed: warnings is 10 (baseline: 12, delta: -2)"
    );
}

#[test]
fn test_evaluate_ratchet_must_not_increase_equal_value() {
    let baseline = sample_baseline("warning-count", "warnings", "must_not_increase", 10.0);
    let eval = evaluate_ratchet(
        10.0,
        Some(&baseline),
        "must_not_increase",
        "warnings",
        "main",
    )
    .unwrap();
    assert_eq!(eval.status, GateStatus::Pass);
    assert_eq!(eval.exit_code, 0);
    assert!(!eval.is_regression);
    assert_eq!(eval.delta, Some(0.0));
    assert_eq!(
        eval.summary,
        "ratchet passed: warnings is 10 (baseline: 10, delta: 0)"
    );
}

#[test]
fn test_evaluate_ratchet_must_not_increase_regression() {
    let baseline = sample_baseline("warning-count", "warnings", "must_not_increase", 10.0);
    let eval = evaluate_ratchet(
        15.0,
        Some(&baseline),
        "must_not_increase",
        "warnings",
        "main",
    )
    .unwrap();
    assert_eq!(eval.status, GateStatus::Fail);
    assert_eq!(eval.exit_code, 1);
    assert!(eval.is_regression);
    assert_eq!(eval.delta, Some(5.0));
    assert_eq!(
        eval.summary,
        "ratchet regression: warnings increased from 10 to 15 (delta: +5)"
    );
}

#[test]
fn test_evaluate_ratchet_must_not_decrease_no_regression() {
    let baseline = sample_baseline("coverage", "coverage_pct", "must_not_decrease", 85.0);
    let eval = evaluate_ratchet(
        90.0,
        Some(&baseline),
        "must_not_decrease",
        "coverage_pct",
        "main",
    )
    .unwrap();
    assert_eq!(eval.status, GateStatus::Pass);
    assert_eq!(eval.exit_code, 0);
    assert!(!eval.is_regression);
    assert_eq!(eval.delta, Some(5.0));
    assert_eq!(
        eval.summary,
        "ratchet passed: coverage_pct is 90 (baseline: 85, delta: +5)"
    );
}

#[test]
fn test_evaluate_ratchet_must_not_decrease_equal_value() {
    let baseline = sample_baseline("coverage", "coverage_pct", "must_not_decrease", 85.0);
    let eval = evaluate_ratchet(
        85.0,
        Some(&baseline),
        "must_not_decrease",
        "coverage_pct",
        "main",
    )
    .unwrap();
    assert_eq!(eval.status, GateStatus::Pass);
    assert_eq!(eval.exit_code, 0);
    assert!(!eval.is_regression);
    assert_eq!(eval.delta, Some(0.0));
    assert_eq!(
        eval.summary,
        "ratchet passed: coverage_pct is 85 (baseline: 85, delta: 0)"
    );
}

#[test]
fn test_evaluate_ratchet_must_not_decrease_regression() {
    let baseline = sample_baseline("coverage", "coverage_pct", "must_not_decrease", 85.0);
    let eval = evaluate_ratchet(
        80.0,
        Some(&baseline),
        "must_not_decrease",
        "coverage_pct",
        "main",
    )
    .unwrap();
    assert_eq!(eval.status, GateStatus::Fail);
    assert_eq!(eval.exit_code, 1);
    assert!(eval.is_regression);
    assert_eq!(eval.delta, Some(-5.0));
    assert_eq!(
        eval.summary,
        "ratchet regression: coverage_pct decreased from 85 to 80 (delta: -5)"
    );
}

#[test]
fn test_evaluate_ratchet_invalid_direction_usage_error() {
    let err = evaluate_ratchet(10.0, None, "invalid_dir", "metric", "main").unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("direction"));
}

#[test]
fn test_format_metric_number() {
    assert_eq!(format_metric_number(0.0), "0");
    assert_eq!(format_metric_number(-0.0), "0");
    assert_eq!(format_metric_number(42.0), "42");
    assert_eq!(format_metric_number(-5.0), "-5");
    assert_eq!(format_metric_number(12.5), "12.5");
    assert_eq!(format_metric_number(-3.75), "-3.75");
}

#[test]
fn test_ratchet_direction_parsing() {
    assert_eq!(
        RatchetDirection::from_str_loose("must_not_increase"),
        Some(RatchetDirection::MustNotIncrease)
    );
    assert_eq!(
        RatchetDirection::from_str_loose("MUST_NOT_DECREASE"),
        Some(RatchetDirection::MustNotDecrease)
    );
    assert_eq!(RatchetDirection::from_str_loose("invalid"), None);
}

// ---------------------------------------------------------------------------
// 2. read_baseline / write_baseline I/O tests
// ---------------------------------------------------------------------------

#[test]
fn test_baseline_read_write_roundtrip() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("warning-count", "warnings", "must_not_increase", 42.0);

    // Initial read is None
    let initial = read_baseline(root, &storage.state_dir, "main", "warning-count").unwrap();
    assert_eq!(initial, None);

    // Write baseline
    let path = write_baseline(root, &storage, "main", &baseline).unwrap();
    assert!(path.exists());
    assert_eq!(
        path,
        resolve_baseline_path(root, &storage.state_dir, "main", "warning-count")
    );

    // Read back
    let read_back = read_baseline(root, &storage.state_dir, "main", "warning-count")
        .unwrap()
        .expect("baseline should exist");
    assert_eq!(read_back, baseline);
}

// ---------------------------------------------------------------------------
// 3. execute_gate with ratchet gates
// ---------------------------------------------------------------------------

#[test]
fn test_execute_gate_ratchet_missing_baseline_passes_with_warning() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "warn-count".to_string(),
        command: Some("echo 10".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("warnings".to_string()),
        direction: Some("must_not_increase".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "warn-count", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.metric, Some(10.0));
    assert!(outcome.summary.contains("baseline: null"));
    assert!(outcome
        .summary
        .contains("[warning: no baseline recorded for branch 'main']"));
    assert!(outcome.receipt().contains("[PASS] warn-count"));
}

#[test]
fn test_execute_gate_ratchet_regression_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("warn-count", "warnings", "must_not_increase", 10.0);
    write_baseline(root, &storage, "main", &baseline).unwrap();

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "warn-count".to_string(),
        command: Some("echo 15".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("warnings".to_string()),
        direction: Some("must_not_increase".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "warn-count", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 1);
    assert_eq!(outcome.metric, Some(15.0));
    assert!(outcome
        .summary
        .contains("ratchet regression: warnings increased from 10 to 15 (delta: +5)"));
    assert_eq!(outcome.agent_instruction.as_deref(), Some("fix_cited_failures"));
    assert!(outcome.receipt().contains("[FAIL] warn-count (exit 1)"));
}

#[test]
fn test_execute_gate_ratchet_no_regression_passes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("warn-count", "warnings", "must_not_increase", 12.0);
    write_baseline(root, &storage, "main", &baseline).unwrap();

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "warn-count".to_string(),
        command: Some("echo 10".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("warnings".to_string()),
        direction: Some("must_not_increase".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "warn-count", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.metric, Some(10.0));
    assert!(outcome
        .summary
        .contains("ratchet passed: warnings is 10 (baseline: 12, delta: -2)"));
    assert!(outcome.receipt().contains("[PASS] warn-count"));
}

#[test]
fn test_execute_gate_ratchet_direction_must_not_decrease_regression() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("coverage-gate", "coverage", "must_not_decrease", 85.0);
    write_baseline(root, &storage, "main", &baseline).unwrap();

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "coverage-gate".to_string(),
        command: Some("echo 80".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("coverage".to_string()),
        direction: Some("must_not_decrease".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "coverage-gate", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 1);
    assert_eq!(outcome.metric, Some(80.0));
    assert!(outcome
        .summary
        .contains("ratchet regression: coverage decreased from 85 to 80 (delta: -5)"));
    assert!(outcome.receipt().contains("[FAIL] coverage-gate (exit 1)"));
}

#[test]
fn test_execute_gate_ratchet_missing_direction_fails_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "warn-count".to_string(),
        command: Some("echo 10".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("warnings".to_string()),
        direction: None,
        skip: None,
    }];

    let err = execute_gate(root, &config, "warn-count", &GateRunOptions::default()).unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("direction"));
}

#[test]
fn test_evaluate_ratchet_non_finite_metric_rejected() {
    let baseline = sample_baseline("gate", "metric", "must_not_increase", 10.0);
    let err = evaluate_ratchet(f64::NAN, Some(&baseline), "must_not_increase", "metric", "main").unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("finite"));

    let err2 = evaluate_ratchet(f64::INFINITY, Some(&baseline), "must_not_increase", "metric", "main").unwrap_err();
    assert_eq!(err2.code(), "usage_error");

    let mut invalid_baseline = baseline.clone();
    invalid_baseline.value = f64::NAN;
    let err3 = evaluate_ratchet(10.0, Some(&invalid_baseline), "must_not_increase", "metric", "main").unwrap_err();
    assert_eq!(err3.code(), "usage_error");
    assert!(err3.message().contains("finite"));
}

#[test]
fn test_write_baseline_has_trailing_newline() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("coverage-gate", "coverage", "must_not_decrease", 85.0);
    let path = write_baseline(root, &storage, "main", &baseline).unwrap();

    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.ends_with('\n'), "baseline file must have trailing newline");
}

#[test]
fn test_execute_gate_ratchet_regression_populates_failures() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("coverage-gate", "coverage", "must_not_decrease", 90.0);
    write_baseline(root, &storage, "main", &baseline).unwrap();

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "coverage-gate".to_string(),
        command: Some("echo 80".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("coverage".to_string()),
        direction: Some("must_not_decrease".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "coverage-gate", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 1);
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(outcome.failures[0].location, "coverage-gate");
    assert_eq!(outcome.failures[0].message, outcome.summary);
}

#[test]
fn test_execute_gate_ratchet_no_numeric_metric_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "text-gate".to_string(),
        command: Some("echo not a number".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("score".to_string()),
        direction: Some("must_not_decrease".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "text-gate", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 1);
    assert!(outcome.summary.contains("did not produce a numeric metric"));
    assert_eq!(outcome.agent_instruction.as_deref(), Some("fix_cited_failures"));
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(outcome.failures[0].location, "text-gate");
}

#[test]
fn test_execute_gate_ratchet_fallback_extraction_patterns() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let storage = StorageConfig::default();
    let baseline = sample_baseline("pattern-gate", "warnings", "must_not_increase", 10.0);
    write_baseline(root, &storage, "main", &baseline).unwrap();

    let mut config = Config::default();
    config.git.integration_branch = "main".to_string();
    config.gates = vec![GateConfig {
        id: "pattern-gate".to_string(),
        command: Some("echo 'Total warnings: 5'".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: Vec::new(),
        kind: Some("ratchet".to_string()),
        metric: Some("warnings".to_string()),
        direction: Some("must_not_increase".to_string()),
        skip: None,
    }];

    let outcome = execute_gate(root, &config, "pattern-gate", &GateRunOptions::default()).unwrap();
    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.metric, Some(5.0));
    assert!(outcome.summary.contains("delta: -5"));
}

