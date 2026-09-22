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
    toml.push_str("\n");
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
