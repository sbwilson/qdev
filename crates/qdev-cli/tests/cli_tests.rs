use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

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
    // [E2S12] (D-1): outside an initialized workspace the default command is
    let empty_temp = TempDir::new().unwrap();
    // the one-line init hint…
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(empty_temp.path())
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::starts_with(
            "not a qdev workspace — run `qdev init` first",
        ));

    // …and inside one it renders the §5 pulse header with the real version.
    let temp = TempDir::new().unwrap();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
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
    let out = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .assert()
        .success()
        .code(0);
    assert!(String::from_utf8_lossy(&out.get_output().stdout)
        .starts_with("qdev 0.1.0 — Development Engine & Gatekeeper\n"));
}

#[test]
fn test_status_command_text() {
    let empty_temp = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(empty_temp.path())
        .arg("status")
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::starts_with(
            "not a qdev workspace — run `qdev init` first",
        ));

    let temp = TempDir::new().unwrap();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
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
    let out = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["status"])
        .assert()
        .success()
        .code(0);
    assert!(String::from_utf8_lossy(&out.get_output().stdout)
        .starts_with("qdev 0.1.0 — Development Engine & Gatekeeper\n"));
}

#[test]
fn test_non_interactive_flag() {
    // [E2S12] replaced the `PulseStatus` stub (name/version/interactivity) with
    // the real pulse: outside a workspace that is `workspace: false` with every
    // other field null, and the command still never prompts (AD-12).
    let temp = TempDir::new().unwrap();
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["--non-interactive", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["workspace"], false);
    assert!(val["environment"].is_null());
    assert!(val["sprints"].is_null());
    assert!(val["gates"].is_null());
    assert!(val["next"].is_null());
}

#[test]
fn test_non_interactive_flag_with_subcommand() {
    let temp = TempDir::new().unwrap();
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .args(["status", "--non-interactive", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["workspace"], false);
    assert!(val["environment"].is_null());
    assert!(val["next"].is_null());
}

#[test]
fn test_non_interactive_env_var() {
    let temp = TempDir::new().unwrap();
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .env("QDEV_NONINTERACTIVE", "1")
        .arg("--json")
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["workspace"], false);
    assert!(val["environment"].is_null());
    assert!(val["next"].is_null());
}

#[test]
fn test_non_interactive_env_var_truthy_values() {
    let temp = TempDir::new().unwrap();
    for val in &["true", "TRUE", "yes", "YES", "1"] {
        let assert = Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(temp.path())
            .env("QDEV_NONINTERACTIVE", val)
            .arg("--json")
            .assert()
            .success()
            .code(0);

        let output = assert.get_output();
        let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
        let json_val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

        assert_eq!(json_val["schema_version"], "1");
        assert_eq!(
            json_val["workspace"], false,
            "Expected the null pulse for env value: {val}"
        );
        assert!(json_val["environment"].is_null());
        assert!(json_val["next"].is_null());
    }
}

#[test]
fn test_non_interactive_env_var_invalid_safe_fallback() {
    let temp = TempDir::new().unwrap();
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(temp.path())
        .env("QDEV_NONINTERACTIVE", "not_a_boolean")
        .args(["status", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["workspace"], false);
    assert!(val["environment"].is_null());
    assert!(val["next"].is_null());
}
