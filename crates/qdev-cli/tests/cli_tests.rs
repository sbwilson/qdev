use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

#[test]
fn test_version_text() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::starts_with("qdev 0.1.0"));
}

#[test]
fn test_version_json() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.args(["--version", "--json"]).assert().success().code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["version"], "0.1.0");
}

#[test]
fn test_version_json_reversed() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.args(["--json", "--version"]).assert().success().code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["version"], "0.1.0");
}

#[test]
fn test_unknown_subcommand_json() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["unknown-cmd", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"].is_string());
}

#[test]
fn test_unknown_subcommand_text() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.arg("unknown-cmd")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("error:"));
}

#[test]
fn test_unknown_flag_json() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["--unknown-flag", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_default_command_text() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.assert()
        .success()
        .code(0)
        .stdout(predicate::str::starts_with("qdev 0.1.0"));
}

#[test]
fn test_status_command_text() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.arg("status")
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::starts_with("qdev 0.1.0"));
}

#[test]
fn test_non_interactive_flag() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["--non-interactive", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["name"], "qdev");
    assert_eq!(val["version"], "0.1.0");
    assert_eq!(val["interactivity"], "non_interactive");
}

#[test]
fn test_non_interactive_flag_with_subcommand() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["status", "--non-interactive", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["name"], "qdev");
    assert_eq!(val["version"], "0.1.0");
    assert_eq!(val["interactivity"], "non_interactive");
}

#[test]
fn test_non_interactive_env_var() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .env("QDEV_NONINTERACTIVE", "1")
        .arg("--json")
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["name"], "qdev");
    assert_eq!(val["version"], "0.1.0");
    assert_eq!(val["interactivity"], "non_interactive");
}

#[test]
fn test_non_interactive_env_var_truthy_values() {
    for val in &["true", "TRUE", "yes", "YES", "1"] {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd
            .env("QDEV_NONINTERACTIVE", val)
            .arg("--json")
            .assert()
            .success()
            .code(0);

        let output = assert.get_output();
        let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
        let json_val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

        assert_eq!(json_val["schema_version"], "1");
        assert_eq!(json_val["name"], "qdev");
        assert_eq!(json_val["version"], "0.1.0");
        assert_eq!(
            json_val["interactivity"], "non_interactive",
            "Expected non_interactive for env value: {}",
            val
        );
    }
}

#[test]
fn test_non_interactive_env_var_invalid_safe_fallback() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .env("QDEV_NONINTERACTIVE", "not_a_boolean")
        .args(["status", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["name"], "qdev");
    assert_eq!(val["version"], "0.1.0");
}
