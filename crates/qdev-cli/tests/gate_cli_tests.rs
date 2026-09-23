use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    // 1. Initialize git repo
    let _ = std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["config", "commit.gpgsign", "false"])
        .current_dir(root)
        .status();

    // 2. Initialize qdev workspace
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "TestProject",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();

    // Commit initial files
    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(root)
        .status();
}

fn append_to_qdev_toml(root: &Path, content: &str) {
    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push('\n');
    toml.push_str(content);
    fs::write(toml_path, toml).unwrap();
}

#[test]
fn test_cli_gate_run_pass_receipt_and_exit_0() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("pass.sh");
    fs::write(&script_path, "#!/bin/sh\necho 'all 12 checks passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "lint"
command = "{}"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "lint"])
        .assert()
        .success()
        .code(0);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(stdout.contains("[PASS] lint | all 12 checks passed |"));
}

#[test]
fn test_cli_gate_run_fail_receipt_and_exit_1() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("fail.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\necho 'tests failed: 2 assertion errors' >&2\nexit 101\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "test"
command = "{}"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "test"])
        .assert()
        .failure()
        .code(1);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        "[FAIL] test (exit 101) | tests failed: 2 assertion errors"
    );
}

#[test]
fn test_cli_gate_run_timeout_and_exit_4() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("hang.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\nsleep 100 & sleep 100 & sleep 100\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "hang"
command = "{}"
timeout_ms = 200
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "hang"])
        .assert()
        .failure()
        .code(4);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        "[INFRA] hang | timeout after 200ms | halt and alert"
    );
}

#[test]
fn test_cli_gate_run_missing_executable_and_exit_4() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "missing"
command = "nonexistent-binary-12345"
timeout_ms = 1000
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "missing"])
        .assert()
        .failure()
        .code(4);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert_eq!(
        stdout.trim(),
        "[INFRA] missing | missing executable: nonexistent-binary-12345 | halt and alert"
    );
}

#[test]
fn test_cli_gate_run_local_skip_and_exit_0() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "skipped-gate"
command = "should-never-be-run"
timeout_ms = 1000
skip = true
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "skipped-gate"])
        .assert()
        .success()
        .code(0);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert_eq!(stdout.trim(), "[SKIP] skipped-gate | skipped_locally");
}

#[test]
fn test_cli_gate_run_unknown_gate_and_exit_2() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "unknown-gate"])
        .assert()
        .failure()
        .code(2);

    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("gate 'unknown-gate' not found in configuration"));
}

#[test]
fn test_cli_gate_run_json_mode_schema_validation() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("test_json.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF' > "$QDEV_RESULT_FILE"
{
  "status": "pass",
  "summary": "18 tests passed"
}
EOF
exit 0
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "json-gate"
command = "{}"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "json-gate", "--story", "E12S4", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["gate"], "json-gate");
    assert_eq!(payload["story"], "E12S4");
    assert_eq!(payload["status"], "pass");
    assert_eq!(payload["summary"], "18 tests passed");
    assert_eq!(payload["exit_code"], 0);
    assert_eq!(payload["skipped_locally"], false);
    assert_eq!(payload["agent_instruction"], "continue");

    // Validate payload against payload-gate-run schema
    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_json_mode_infra_failure_schema_validation() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "missing-infra"
command = "nonexistent-cmd"
timeout_ms = 1000
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "missing-infra", "--json"])
        .assert()
        .failure()
        .code(4);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["gate"], "missing-infra");
    assert_eq!(payload["status"], "infra");
    assert_eq!(payload["exit_code"], 4);
    assert_eq!(payload["skipped_locally"], false);
    assert_eq!(payload["agent_instruction"], "halt_and_alert");
    assert!(payload["summary"]
        .as_str()
        .unwrap()
        .contains("missing executable: nonexistent-cmd"));

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_json_mode_local_skip_schema_validation() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "skip-json"
command = "never-run"
timeout_ms = 1000
skip = true
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "skip-json", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["gate"], "skip-json");
    assert_eq!(payload["status"], "skip");
    assert_eq!(payload["exit_code"], 0);
    assert_eq!(payload["skipped_locally"], true);
    assert_eq!(payload["summary"], "skipped_locally");

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_unknown_output_adapter_and_exit_2() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "bad-adapter-gate"
command = "echo hello"
output_adapter = "unsupported_adapter"
timeout_ms = 1000
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "bad-adapter-gate"])
        .assert()
        .failure()
        .code(2);

    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("Unknown output adapter 'unsupported_adapter'"));
}

#[test]
fn test_cli_gate_run_cargo_adapter_failure_receipt_and_json() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("cargo_fail.sh");
    let fixture = include_str!("../../qdev-core/tests/fixtures/cargo_test_failure.txt");
    fs::write(
        &script_path,
        format!("#!/bin/sh\ncat << 'EOF'\n{}\nEOF\nexit 101\n", fixture),
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "cargo-gate"
command = "{}"
output_adapter = "cargo"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    // 1. Text receipt mode
    let assert_text = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "cargo-gate"])
        .assert()
        .failure()
        .code(1);

    let stdout = std::str::from_utf8(&assert_text.get_output().stdout).unwrap();
    assert!(stdout.contains("[FAIL] cargo-gate (exit 101) | crates/bridge/tests/c_abi_round_trip.rs:142 | assertion failed: left == right"));

    // 2. JSON payload mode
    let assert_json = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "cargo-gate", "--json"])
        .assert()
        .failure()
        .code(1);

    let payload: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(payload["gate"], "cargo-gate");
    assert_eq!(payload["status"], "fail");
    assert_eq!(payload["exit_code"], 101);
    assert_eq!(payload["summary"], "1 of 18 tests failed");
    assert_eq!(payload["agent_instruction"], "fix_cited_failures");
    let failures = payload["failures"].as_array().expect("failures must be array");
    assert_eq!(failures.len(), 1);
    assert_eq!(
        failures[0]["location"],
        "crates/bridge/tests/c_abi_round_trip.rs:142"
    );

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_xcodebuild_adapter_failure_receipt_and_json() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("xcode_fail.sh");
    let raw_fixture = include_str!("../../qdev-core/tests/fixtures/xcodebuild_test_failure.txt");
    let fixture = raw_fixture.replace("/Users/developer/project", &temp.path().to_string_lossy());
    fs::write(
        &script_path,
        format!("#!/bin/sh\ncat << 'EOF'\n{}\nEOF\nexit 1\n", fixture),
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "xcode-gate"
command = "{}"
output_adapter = "xcodebuild"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    // 1. Text receipt mode
    let assert_text = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "xcode-gate"])
        .assert()
        .failure()
        .code(1);

    let stdout = std::str::from_utf8(&assert_text.get_output().stdout).unwrap();
    assert!(stdout.contains("[FAIL] xcode-gate (exit 1) | Tests/AppTests/MyTests.swift:42 | XCTAssertEqual failed: (\"EffectsUnavailable\") is not equal to (\"EventNotUnderstood\")"));

    // 2. JSON payload mode
    let assert_json = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "xcode-gate", "--json"])
        .assert()
        .failure()
        .code(1);

    let payload: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(payload["gate"], "xcode-gate");
    assert_eq!(payload["status"], "fail");
    assert_eq!(payload["exit_code"], 1);
    assert_eq!(payload["summary"], "1 of 18 tests failed");
    assert_eq!(payload["agent_instruction"], "fix_cited_failures");
    let failures = payload["failures"].as_array().expect("failures must be array");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["location"], "Tests/AppTests/MyTests.swift:42");

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_json_mode_with_metric_and_constraints() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("metric_gate.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF' > "$QDEV_RESULT_FILE"
{
  "status": "pass",
  "summary": "all benchmarks passed",
  "failures": [],
  "metric": 99.5,
  "constraint_ids": ["FR-102", "NFR-402"]
}
EOF
exit 0
"#,
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "benchmark-gate"
command = "{}"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "benchmark-gate", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["gate"], "benchmark-gate");
    assert_eq!(payload["status"], "pass");
    assert_eq!(payload["metric"], 99.5);
    let constraints = payload["constraint_ids"]
        .as_array()
        .expect("constraint_ids array");
    assert_eq!(constraints.len(), 2);
    assert_eq!(constraints[0], "FR-102");
    assert_eq!(constraints[1], "NFR-402");

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_cargo_adapter_success() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("cargo_pass.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF'
running 18 tests
test test_a ... ok
test test_b ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
EOF
exit 0
"#,
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "cargo-gate"
command = "{}"
output_adapter = "cargo"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "cargo-gate", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["gate"], "cargo-gate");
    assert_eq!(payload["status"], "pass");
    assert_eq!(payload["summary"], "18 tests passed");
    assert_eq!(payload["agent_instruction"], "continue");

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_xcodebuild_adapter_success() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_path = temp.path().join("xcodebuild_pass.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF'
Test Suite 'All tests' started at 2026-09-23 10:00:00.000
Test Suite 'All tests' passed at 2026-09-23 10:00:01.000.
	 Executed 18 tests, with 0 failures (0 unexpected) in 0.123 (0.125) seconds
EOF
exit 0
"#,
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "xcodebuild-gate"
command = "{}"
output_adapter = "xcodebuild"
timeout_ms = 5000
"#,
            script_path.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "xcodebuild-gate", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["gate"], "xcodebuild-gate");
    assert_eq!(payload["status"], "pass");
    assert_eq!(payload["summary"], "18 tests passed");
    assert_eq!(payload["agent_instruction"], "continue");

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_all_topological_execution_and_receipts() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_a = temp.path().join("a.sh");
    fs::write(&script_a, "#!/bin/sh\necho 'a passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_a).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_a, perms).unwrap();
    }

    let script_b = temp.path().join("b.sh");
    fs::write(&script_b, "#!/bin/sh\necho 'b passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_b).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_b, perms).unwrap();
    }

    // B declared before A in qdev.toml, but B depends on A
    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "gate-b"
command = "{}"
depends_on = ["gate-a"]

[[gates]]
id = "gate-a"
command = "{}"
"#,
            script_b.to_string_lossy(),
            script_a.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--all"])
        .assert()
        .success()
        .code(0);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    let pos_a = stdout.find("[PASS] gate-a").expect("gate-a receipt missing");
    let pos_b = stdout.find("[PASS] gate-b").expect("gate-b receipt missing");
    assert!(pos_a < pos_b, "gate-a must execute and print before gate-b");
}

#[test]
fn test_cli_gate_run_all_json_schema_validation() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_a = temp.path().join("a.sh");
    fs::write(&script_a, "#!/bin/sh\necho 'a passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_a).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_a, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "gate-a"
command = "{}"
"#,
            script_a.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--all", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["schema_version"], "1");
    let runs = payload["runs"].as_array().expect("runs must be an array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["gate"], "gate-a");
    assert_eq!(runs[0]["status"], "pass");

    let schema_str = qdev_core::PayloadKind::GateSet.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-set schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_for_transition() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script = temp.path().join("check.sh");
    fs::write(&script, "#!/bin/sh\necho 'ok'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "prereq-lint"
command = "{}"

[[gates]]
id = "review-gate"
command = "{}"
depends_on = ["prereq-lint"]
on_transition = ["review"]

[[gates]]
id = "release-gate"
command = "{}"
on_transition = ["release"]
"#,
            script.to_string_lossy(),
            script.to_string_lossy(),
            script.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--for-transition", "review"])
        .assert()
        .success()
        .code(0);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(stdout.contains("[PASS] prereq-lint"));
    assert!(stdout.contains("[PASS] review-gate"));
    assert!(!stdout.contains("release-gate"), "release-gate should not be executed");
}

#[test]
fn test_cli_gate_run_cascade_skip_receipt_and_exit_1() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let fail_script = temp.path().join("fail.sh");
    fs::write(&fail_script, "#!/bin/sh\necho 'unit test failed'\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fail_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fail_script, perms).unwrap();
    }

    let pass_script = temp.path().join("pass.sh");
    fs::write(&pass_script, "#!/bin/sh\necho 'integration passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&pass_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&pass_script, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "unit"
command = "{}"

[[gates]]
id = "integration"
command = "{}"
depends_on = ["unit"]
"#,
            fail_script.to_string_lossy(),
            pass_script.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--all"])
        .assert()
        .failure()
        .code(1);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(stdout.contains("[FAIL] unit (exit 1)"));
    assert!(stdout.contains("[SKIP] integration | dependency failed: unit"));
}

#[test]
fn test_cli_gate_run_cascade_skip_infra_exit_4() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "missing-gate"
command = "nonexistent_executable_98765"

[[gates]]
id = "dependent-gate"
command = "true"
depends_on = ["missing-gate"]
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--all"])
        .assert()
        .failure()
        .code(4);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(stdout.contains("[INFRA] missing-gate"));
    assert!(stdout.contains("[SKIP] dependent-gate | dependency failed: missing-gate"));
}

#[test]
fn test_cli_gate_run_conflicting_flags_exit_2() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    // my-gate --all
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "my-gate", "--all"])
        .assert()
        .failure()
        .code(2);

    // --all --for-transition review
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--all", "--for-transition", "review"])
        .assert()
        .failure()
        .code(2);

    // gate run with no targets
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn test_cli_gate_run_cycle_exit_2() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "gate-x"
command = "true"
depends_on = ["gate-y"]

[[gates]]
id = "gate-y"
command = "true"
depends_on = ["gate-x"]
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--all"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("gate-x -> gate-y -> gate-x")
            || combined.contains("gate-y -> gate-x -> gate-y")
            || combined.contains("circular dependency detected in gates")
    );
}

#[test]
fn test_cli_gate_list_text_mode() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "lint"
command = "true"
kind = "check"
on_transition = ["review"]

[[gates]]
id = "integration"
command = "true"
depends_on = ["lint"]
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "list"])
        .assert()
        .success()
        .code(0);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    assert!(stdout.contains("lint"));
    assert!(stdout.contains("check"));
    assert!(stdout.contains("review"));
    assert!(stdout.contains("integration"));
    assert!(stdout.contains("command"));
}

#[test]
fn test_cli_gate_list_json_schema_validation() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "lint"
command = "true"
kind = "check"
on_transition = ["review"]

[[gates]]
id = "test"
command = "true"
depends_on = ["lint"]
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "list", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["schema_version"], "1");
    let gates = payload["gates"].as_array().expect("gates must be an array");
    assert_eq!(gates.len(), 2);

    assert_eq!(gates[0]["id"], "lint");
    assert_eq!(gates[0]["kind"], "check");
    assert_eq!(gates[0]["transitions"], serde_json::json!(["review"]));
    assert_eq!(gates[0]["dependencies"], serde_json::json!([]));
    assert_eq!(gates[0]["last_status"], serde_json::Value::Null);

    assert_eq!(gates[1]["id"], "test");
    assert_eq!(gates[1]["kind"], "command");
    assert_eq!(gates[1]["transitions"], serde_json::json!([]));
    assert_eq!(gates[1]["dependencies"], serde_json::json!(["lint"]));
    assert_eq!(gates[1]["last_status"], serde_json::Value::Null);

    let schema_str = qdev_core::PayloadKind::GateList.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-list schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_single_gate_with_dependencies() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script_a = temp.path().join("a.sh");
    fs::write(&script_a, "#!/bin/sh\necho 'a passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_a).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_a, perms).unwrap();
    }

    let script_b = temp.path().join("b.sh");
    fs::write(&script_b, "#!/bin/sh\necho 'b passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_b).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_b, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "gate-b"
command = "{}"
depends_on = ["gate-a"]

[[gates]]
id = "gate-a"
command = "{}"
"#,
            script_b.to_string_lossy(),
            script_a.to_string_lossy()
        ),
    );

    // 1. Text mode
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "gate-b"])
        .assert()
        .success()
        .code(0);

    let stdout = std::str::from_utf8(&assert.get_output().stdout).unwrap();
    let pos_a = stdout.find("[PASS] gate-a").expect("gate-a receipt missing");
    let pos_b = stdout.find("[PASS] gate-b").expect("gate-b receipt missing");
    assert!(pos_a < pos_b, "prerequisite gate-a must run before gate-b");

    // 2. JSON mode: must emit GateRunPayload for target gate-b
    let assert_json = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "gate-b", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(payload["gate"], "gate-b");
    assert_eq!(payload["status"], "pass");

    let schema_str = qdev_core::PayloadKind::GateRun.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-run schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_for_transition_json_schema_validation() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    let script = temp.path().join("check.sh");
    fs::write(&script, "#!/bin/sh\necho 'ok'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }

    append_to_qdev_toml(
        temp.path(),
        &format!(
            r#"
[[gates]]
id = "lint"
command = "{}"
on_transition = ["review"]
"#,
            script.to_string_lossy()
        ),
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--for-transition", "review", "--json"])
        .assert()
        .success()
        .code(0);

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(payload["schema_version"], "1");
    let runs = payload["runs"].as_array().expect("runs must be array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["gate"], "lint");

    let schema_str = qdev_core::PayloadKind::GateSet.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    assert!(
        validator.is_valid(&payload),
        "Payload must validate against payload-gate-set schema: {:?}",
        validator
            .iter_errors(&payload)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_cli_gate_run_for_transition_unmatched_exit_code_2() {
    let temp = TempDir::new().unwrap();
    setup_workspace(temp.path());

    append_to_qdev_toml(
        temp.path(),
        r#"
[[gates]]
id = "lint"
command = "true"
on_transition = ["review"]
"#,
    );

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["gate", "run", "--for-transition", "nonexistent"])
        .assert()
        .failure()
        .code(2);

    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("no gates configured for transition 'nonexistent'"));
}

