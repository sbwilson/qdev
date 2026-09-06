use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn test_schema_text_happy_path() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd.args(["schema", "story"]).assert().success().code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();

    let parsed: Value = serde_json::from_str(stdout_str).expect("stdout must be valid JSON");
    assert!(parsed.is_object());
    assert_eq!(parsed["type"], "object");
    assert!(parsed["properties"]["constraints"].is_object());
    assert!(parsed["properties"]["relations"].is_object());
    assert!(parsed["required"]
        .as_array()
        .unwrap()
        .contains(&Value::String("created_by".to_string())));
    assert!(parsed["required"]
        .as_array()
        .unwrap()
        .contains(&Value::String("updated_by".to_string())));
}

#[test]
fn test_schema_json_happy_path() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["schema", "story", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();

    let envelope: Value =
        serde_json::from_str(stdout_str).expect("stdout must be valid JSON envelope");
    assert_eq!(envelope["schema_version"], "1");
    assert_eq!(envelope["type"], "object");
    assert!(envelope["properties"]["constraints"].is_object());
    assert!(envelope["properties"]["relations"].is_object());
}

#[test]
fn test_schema_kind_aliases() {
    // dw alias
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "dw"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Deferred Work Frontmatter Schema"));

    // deferred_work alias
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "deferred_work"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Deferred Work Frontmatter Schema"));

    // adrs alias
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "adrs"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("ADR Frontmatter Schema"));

    // plural stories alias
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "stories"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("Story Frontmatter Schema"));

    // gate_run alias
    Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "gate_run"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains(
            "Evidence Record Frontmatter Schema",
        ));
}

#[test]
fn test_schema_in_uninitialized_dir() {
    let empty_dir = TempDir::new().unwrap();

    // In text mode
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(empty_dir.path())
        .args(["schema", "prd"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("PRD Frontmatter Schema"));

    // In JSON mode
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(empty_dir.path())
        .args(["schema", "prd", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["title"], "PRD Frontmatter Schema");
}

#[test]
fn test_unknown_schema_kind_text() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.args(["schema", "invalid_kind"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "Unknown schema kind 'invalid_kind'",
        ))
        .stderr(predicate::str::contains("Valid schema kinds"));
}

#[test]
fn test_unknown_schema_kind_json() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["schema", "invalid_kind", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).unwrap();

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Unknown schema kind 'invalid_kind'"));
}

#[test]
fn test_missing_kind_argument() {
    // Text mode: Clap error on stderr, exit 2
    Command::cargo_bin("qdev")
        .unwrap()
        .arg("schema")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("<KIND>"));

    // JSON mode: Clap error wrapped in AD-13 error envelope on stdout, exit 2
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_all_13_kinds_via_cli() {
    let kinds = [
        "prd",
        "requirement",
        "epic",
        "story",
        "adr",
        "hazard",
        "sprint",
        "release",
        "dw",
        "decision",
        "scratchpad",
        "soup",
        "evidence",
    ];

    for kind in kinds {
        // Text mode
        let assert_text = Command::cargo_bin("qdev")
            .unwrap()
            .args(["schema", kind])
            .assert()
            .success()
            .code(0);
        let output_text = assert_text.get_output();
        let parsed_text: Value =
            serde_json::from_str(std::str::from_utf8(&output_text.stdout).unwrap())
                .unwrap_or_else(|e| panic!("Failed to parse JSON for {}: {}", kind, e));
        assert_eq!(parsed_text["type"], "object");

        // JSON mode
        let assert_json = Command::cargo_bin("qdev")
            .unwrap()
            .args(["schema", kind, "--json"])
            .assert()
            .success()
            .code(0);
        let output_json = assert_json.get_output();
        let parsed_json: Value =
            serde_json::from_str(std::str::from_utf8(&output_json.stdout).unwrap())
                .unwrap_or_else(|e| panic!("Failed to parse JSON envelope for {}: {}", kind, e));
        assert_eq!(parsed_json["schema_version"], "1");
        assert_eq!(parsed_json["type"], "object");
    }
}
