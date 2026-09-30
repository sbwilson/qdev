//! Integration tests for `qdev mcp serve` and `qdev install mcp` CLI per Story 4.5.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use qdev_core::PayloadKind;
use serde_json::{json, Value};
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
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
}

#[test]
fn test_mcp_install_fails_without_flags() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["install", "mcp"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("At least one target"));
}

#[test]
fn test_mcp_install_claude_and_cursor_human_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Human output
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["install", "mcp", "--claude", "--cursor"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".claude/settings.json"))
        .stdout(predicate::str::contains(".cursor/mcp.json"));

    // Verify Claude settings file
    let claude_path = root.join(".claude/settings.json");
    assert!(claude_path.exists());
    let claude_val: Value =
        serde_json::from_str(&fs::read_to_string(&claude_path).unwrap()).unwrap();
    assert_eq!(
        claude_val["mcpServers"]["qdev"]["args"],
        json!(["mcp", "serve"])
    );

    // Verify Cursor settings file
    let cursor_path = root.join(".cursor/mcp.json");
    assert!(cursor_path.exists());
    let cursor_val: Value =
        serde_json::from_str(&fs::read_to_string(&cursor_path).unwrap()).unwrap();
    assert_eq!(
        cursor_val["mcpServers"]["qdev"]["args"],
        json!(["mcp", "serve"])
    );

    // JSON output and schema validation
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["install", "mcp", "--claude", "--cursor", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert_json.get_output().stdout.clone()).unwrap();
    let payload: Value = serde_json::from_str(&stdout).expect("output must be valid JSON");
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["targets"], json!(["claude", "cursor"]));
    assert_eq!(
        payload["installed_files"],
        json!([".claude/settings.json", ".cursor/mcp.json"])
    );
    assert_eq!(payload["command"], "qdev");
    assert_eq!(payload["args"], json!(["mcp", "serve"]));

    // Validate payload against schema
    let schema_str = PayloadKind::McpInstall.schema_str();
    let schema_json: Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_json).unwrap();
    let errors: Vec<String> = validator.iter_errors(&payload).map(|e| e.to_string()).collect();
    assert!(
        errors.is_empty(),
        "Payload must match payload-mcp-install.json schema: {:?}",
        errors
    );
}

#[test]
fn test_mcp_install_preserves_existing_config() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{
  "theme": "dark",
  "mcpServers": {
    "other-server": {
      "command": "node",
      "args": ["server.js"]
    }
  }
}"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["install", "mcp", "--claude"])
        .assert()
        .success();

    let content = fs::read_to_string(claude_dir.join("settings.json")).unwrap();
    let val: Value = serde_json::from_str(&content).unwrap();
    assert_eq!(val["theme"], "dark");
    assert_eq!(val["mcpServers"]["other-server"]["command"], "node");
    assert_eq!(
        val["mcpServers"]["qdev"]["args"],
        json!(["mcp", "serve"])
    );
}

#[test]
fn test_mcp_serve_stdio_handshake_and_tools_list() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let init_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test-client", "version": "1.0.0" }
        }
    });

    let tools_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    });

    let call_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "validate",
            "arguments": { "changed": false }
        }
    });

    let input = format!("{}\n{}\n{}\n", init_req, tools_req, call_req);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert_run = cmd
        .current_dir(root)
        .args(["mcp", "serve"])
        .write_stdin(input)
        .assert()
        .success();

    let stdout = String::from_utf8(assert_run.get_output().stdout.clone()).unwrap();
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 3, "Expected exactly 3 JSON responses");

    let init_resp: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(init_resp["id"], 1);
    assert_eq!(init_resp["result"]["serverInfo"]["name"], "qdev");

    let tools_resp: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(tools_resp["id"], 2);
    let tools = tools_resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 12);

    let call_resp: Value = serde_json::from_str(lines[2]).unwrap();
    assert_eq!(call_resp["id"], 3);
    assert_eq!(call_resp["result"]["isError"], false);
    let text = call_resp["result"]["content"][0]["text"].as_str().unwrap();
    let payload: Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert!(payload["findings"].is_array());
}

#[test]
fn test_doctor_reports_mcp_status() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Initial doctor: unregistered
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert_out = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();

    let stdout = String::from_utf8(assert_out.get_output().stdout.clone()).unwrap();
    let report: Value = serde_json::from_str(&stdout).unwrap();
    let sections = report["sections"].as_array().unwrap();
    let mcp = sections
        .iter()
        .find(|s| s["name"] == "mcp")
        .expect("doctor must include mcp section");
    assert_eq!(mcp["status"], "unregistered");
    assert_eq!(mcp["registered"], false);
    assert_eq!(mcp["handshake_ok"], true);

    // After install --cursor
    let mut install_cmd = Command::cargo_bin("qdev").unwrap();
    install_cmd
        .current_dir(root)
        .args(["install", "mcp", "--cursor"])
        .assert()
        .success();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert_out2 = cmd2
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();

    let stdout2 = String::from_utf8(assert_out2.get_output().stdout.clone()).unwrap();
    let report2: Value = serde_json::from_str(&stdout2).unwrap();
    let sections2 = report2["sections"].as_array().unwrap();
    let mcp2 = sections2
        .iter()
        .find(|s| s["name"] == "mcp")
        .expect("doctor must include mcp section");
    assert_eq!(mcp2["status"], "ok");
    assert_eq!(mcp2["registered"], true);
    assert_eq!(mcp2["handshake_ok"], true);
    assert_eq!(mcp2["registered_targets"], json!(["cursor"]));
}
