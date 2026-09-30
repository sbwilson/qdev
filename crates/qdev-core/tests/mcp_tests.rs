//! Unit tests for MCP server protocol, tool execution, installation, and inspection per Story 4.5.

use std::fs;
use std::path::Path;

use qdev_core::config::Config;
use qdev_core::{
    init, inspect_mcp, inspect_mcp_with_handshake_override, install_mcp, InitLayout, InitOptions,
    McpServer,
};
use tempfile::TempDir;

fn setup_test_workspace(root: &Path) {
    let options = InitOptions {
        root: root.to_path_buf(),
        name: "test".to_string(),
        developer: "simon".to_string(),
        teams: vec!["core".to_string()],
        layout: InitLayout::default(),
    };
    init(&options).unwrap();
}

fn write_story(root: &Path, id: &str, title: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: draft
version: 1
appetite: small
target_modules:
  - core
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC 1
"#
        ),
    )
    .unwrap();

    let _ = qdev_core::ensure_cache(root, &qdev_core::StorageConfig::default());
}

#[test]
fn test_mcp_initialize_handshake() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0"}}}"#;
    let resp_str = server.handle_request_str(req).expect("must produce response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["protocolVersion"], "2024-11-05");
    assert!(resp["result"]["capabilities"]["tools"].is_object());
    assert_eq!(resp["result"]["serverInfo"]["name"], "qdev");
    assert_eq!(
        resp["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn test_mcp_initialized_notification_produces_no_response() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    let req = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let resp = server.handle_request_str(req);
    assert!(resp.is_none(), "notifications must not produce responses");
}

#[test]
fn test_mcp_tools_list_advertises_all_12_tools_with_schemas() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
    let resp_str = server.handle_request_str(req).expect("must produce response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 12, "must advertise exactly 12 tools");

    let tool_names: Vec<&str> = tools
        .iter()
        .map(|t| t["name"].as_str().expect("name"))
        .collect();

    let expected = vec![
        "get_entity",
        "list_entities",
        "context",
        "next",
        "claim",
        "transition",
        "scratch_append",
        "scratch_read",
        "dw_add",
        "decision_log",
        "gate_run",
        "validate",
    ];

    for exp in &expected {
        assert!(tool_names.contains(exp), "tool '{}' missing from catalog", exp);
    }

    // Every advertised tool must have inputSchema and outputSchema
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["inputSchema"].is_object(),
            "tool '{}' missing inputSchema",
            name
        );
        assert_eq!(
            tool["inputSchema"]["type"], "object",
            "tool '{}' inputSchema type must be object",
            name
        );
        assert!(
            tool["outputSchema"].is_object(),
            "tool '{}' missing outputSchema",
            name
        );
    }
}

#[test]
fn test_mcp_malformed_json_returns_parse_error() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    let req = "this is not json { [";
    let resp_str = server.handle_request_str(req).expect("must produce parse error response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["jsonrpc"], "2.0");
    assert!(resp["id"].is_null());
    assert_eq!(resp["error"]["code"], -32700);
}

#[test]
fn test_mcp_unknown_method_returns_method_not_found() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    let req = r#"{"jsonrpc":"2.0","id":99,"method":"unknown_rpc_method"}"#;
    let resp_str = server.handle_request_str(req).expect("must produce error response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 99);
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn test_mcp_tool_call_get_entity_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E12S4", "MCP Story");

    let server = McpServer::from_workspace(root).unwrap();

    let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_entity","arguments":{"id":"E12S4"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 3);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().expect("text");
    let payload: serde_json::Value = serde_json::from_str(text).expect("valid JSON text");
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["id"], "E12S4");
    assert_eq!(payload["title"], "MCP Story");
}

#[test]
fn test_mcp_tool_call_claim_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let server = McpServer::from_workspace(root).unwrap();

    // Call claim on non-existent story
    let req = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"claim","arguments":{"story_id":"NONEXISTENT"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 4);
    let result = &resp["result"];
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().expect("text");
    let err_env: serde_json::Value = serde_json::from_str(text).expect("valid JSON error envelope");
    assert_eq!(err_env["schema_version"], "1");
    assert!(err_env["error"]["code"].is_string());
    assert!(err_env["error"]["message"].is_string());
}

#[test]
fn test_mcp_unknown_tool_call_returns_tool_error() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    let req = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"unknown_tool","arguments":{}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 5);
    let result = &resp["result"];
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().expect("text");
    let err_env: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(err_env["error"]["code"], "usage_error");
}

#[test]
fn test_mcp_install_missing_flags_fails() {
    let temp = TempDir::new().unwrap();
    let res = install_mcp(temp.path(), false, false);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code(), "usage_error");
}

#[test]
fn test_mcp_install_claude_and_cursor_preserves_existing_keys() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Pre-seed .claude/settings.json with existing configuration
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{
  "theme": "dark",
  "mcpServers": {
    "other_server": {
      "command": "python",
      "args": ["server.py"]
    }
  }
}
"#,
    )
    .unwrap();

    // Pre-seed .cursor/mcp.json with existing configuration
    let cursor_dir = root.join(".cursor");
    fs::create_dir_all(&cursor_dir).unwrap();
    fs::write(
        cursor_dir.join("mcp.json"),
        r#"{
  "customSetting": 123
}
"#,
    )
    .unwrap();

    let report = install_mcp(root, true, true).unwrap();
    assert_eq!(report.targets, vec!["claude", "cursor"]);
    assert_eq!(
        report.installed_files,
        vec![".claude/settings.json", ".cursor/mcp.json"]
    );
    assert_eq!(report.command, "qdev");
    assert_eq!(report.args, vec!["mcp", "serve"]);

    // Verify Claude settings preserved keys
    let claude_content = fs::read_to_string(claude_dir.join("settings.json")).unwrap();
    let claude_val: serde_json::Value = serde_json::from_str(&claude_content).unwrap();
    assert_eq!(claude_val["theme"], "dark");
    assert_eq!(
        claude_val["mcpServers"]["other_server"]["command"],
        "python"
    );
    assert_eq!(claude_val["mcpServers"]["qdev"]["command"], "qdev");
    assert_eq!(
        claude_val["mcpServers"]["qdev"]["args"],
        serde_json::json!(["mcp", "serve"])
    );

    // Verify Cursor settings preserved keys
    let cursor_content = fs::read_to_string(cursor_dir.join("mcp.json")).unwrap();
    let cursor_val: serde_json::Value = serde_json::from_str(&cursor_content).unwrap();
    assert_eq!(cursor_val["customSetting"], 123);
    assert_eq!(cursor_val["mcpServers"]["qdev"]["command"], "qdev");
    assert_eq!(
        cursor_val["mcpServers"]["qdev"]["args"],
        serde_json::json!(["mcp", "serve"])
    );
}

#[test]
fn test_mcp_doctor_inspection_unregistered_and_registered() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    // In fresh workspace, MCP is unregistered but handshake succeeds
    let status = inspect_mcp(root).unwrap();
    assert_eq!(status.status, "unregistered");
    assert_eq!(status.registered, false);
    assert_eq!(status.handshake_ok, true);
    assert!(status.registered_targets.is_empty());

    // After install, status is ok and registered is true
    install_mcp(root, true, false).unwrap();
    let status_after = inspect_mcp(root).unwrap();
    assert_eq!(status_after.status, "ok");
    assert_eq!(status_after.registered, true);
    assert_eq!(status_after.handshake_ok, true);
    assert_eq!(status_after.registered_targets, vec!["claude"]);
}

#[test]
fn test_mcp_notification_without_id_returns_none() {
    let temp = TempDir::new().unwrap();
    let server = McpServer::new(temp.path().to_path_buf(), Config::default());

    // Custom notification (no "id") must never produce a response
    let notif = r#"{"jsonrpc":"2.0","method":"custom/event","params":{}}"#;
    assert!(server.handle_request_str(notif).is_none());
}

#[test]
fn test_mcp_install_refuses_corrupted_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(claude_dir.join("settings.json"), "{ invalid json [").unwrap();

    let res = install_mcp(root, true, false);
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().code(), "corrupted_config");
}

#[test]
fn test_mcp_inspect_handshake_failure_when_registered() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    install_mcp(root, true, false).unwrap();

    let status = inspect_mcp_with_handshake_override(root, false).unwrap();
    assert_eq!(status.status, "unavailable");
    assert_eq!(status.unavailable_reason, Some("handshake_failed".to_string()));
    assert_eq!(status.registered, true);
    assert_eq!(status.handshake_ok, false);
    assert_eq!(status.registered_targets, vec!["claude"]);
}

#[test]
fn test_mcp_inspect_ignores_wrong_args() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let claude_dir = root.join(".claude");
    fs::create_dir_all(&claude_dir).unwrap();
    fs::write(
        claude_dir.join("settings.json"),
        r#"{"mcpServers":{"qdev":{"command":"qdev","args":["wrong","subcommand"]}}}"#,
    ).unwrap();

    let status = inspect_mcp(root).unwrap();
    assert_eq!(status.status, "unregistered");
    assert_eq!(status.registered, false);
    assert!(status.registered_targets.is_empty());
}

#[test]
fn test_mcp_tool_call_list_entities() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E1S1", "Story 1");

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_entities","arguments":{"kind":"story"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 10);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert!(payload["items"].is_array());
}

#[test]
fn test_mcp_tool_call_context() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E1S1", "Story 1");

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"context","arguments":{"id":"E1S1","phase":"specify"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 11);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["phase"], "specify");
}

#[test]
fn test_mcp_tool_call_next() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E1S1", "Story 1");

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"next","arguments":{}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 12);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
}

#[test]
fn test_mcp_tool_call_transition() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E1S1", "Story 1");

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"transition","arguments":{"story_id":"E1S1","target_status":"ready"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 13);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert_eq!(payload["to_status"], "ready");
}

#[test]
fn test_mcp_tool_call_scratch_append_and_read() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E1S1", "Story 1");

    let server = McpServer::from_workspace(root).unwrap();

    // 1. scratch_append with author attribution
    let append_req = r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"scratch_append","arguments":{"story_id":"E1S1","content":"Test scratch note","kind":"note","author_type":"agent","author_id":"bot-1"}}}"#;
    let append_resp_str = server.handle_request_str(append_req).expect("append response");
    let append_resp: serde_json::Value = serde_json::from_str(&append_resp_str).unwrap();

    assert_eq!(append_resp["id"], 14);
    assert_eq!(append_resp["result"]["isError"], false);
    let append_text = append_resp["result"]["content"][0]["text"].as_str().unwrap();
    let append_payload: serde_json::Value = serde_json::from_str(append_text).unwrap();
    assert_eq!(append_payload["story_id"], "E1S1");
    assert_eq!(append_payload["author"]["id"], "bot-1");

    // 2. scratch_read
    let read_req = r#"{"jsonrpc":"2.0","id":15,"method":"tools/call","params":{"name":"scratch_read","arguments":{"story_id":"E1S1","kind":"note"}}}"#;
    let read_resp_str = server.handle_request_str(read_req).expect("read response");
    let read_resp: serde_json::Value = serde_json::from_str(&read_resp_str).unwrap();

    assert_eq!(read_resp["id"], 15);
    assert_eq!(read_resp["result"]["isError"], false);
    let read_text = read_resp["result"]["content"][0]["text"].as_str().unwrap();
    let read_payload: serde_json::Value = serde_json::from_str(read_text).unwrap();
    let entries = read_payload["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["text"], "Test scratch note");
}

#[test]
fn test_mcp_tool_call_dw_add() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    // Add module core to config
    let config_path = root.join("qdev.toml");
    let mut config_text = fs::read_to_string(&config_path).unwrap();
    config_text.push_str("\n[[modules]]\nid = \"core\"\npaths = [\"src/**\"]\n");
    fs::write(&config_path, config_text).unwrap();

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":16,"method":"tools/call","params":{"name":"dw_add","arguments":{"title":"Refactor buffer","module":"core","risk":"negligible","author_type":"agent","author_id":"bot-1"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 16);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["target_module"], "core");
    assert_eq!(payload["safety_risk"], "negligible");
    assert_eq!(payload["title"], "Refactor buffer");
}

#[test]
fn test_mcp_tool_call_decision_log() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    write_story(root, "E1S1", "Story 1");

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":17,"method":"tools/call","params":{"name":"decision_log","arguments":{"topic":"Database choice","ruling":"Use SQLite","type":"human_ruling","subject_id":"E1S1"}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 17);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["topic"], "Database choice");
    assert_eq!(payload["ruling"], "Use SQLite");
}

#[test]
fn test_mcp_tool_call_gate_run() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":18,"method":"tools/call","params":{"name":"gate_run","arguments":{"all":true}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 18);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
}

#[test]
fn test_mcp_tool_call_validate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let server = McpServer::from_workspace(root).unwrap();
    let req = r#"{"jsonrpc":"2.0","id":19,"method":"tools/call","params":{"name":"validate","arguments":{"changed":false}}}"#;
    let resp_str = server.handle_request_str(req).expect("response");
    let resp: serde_json::Value = serde_json::from_str(&resp_str).unwrap();

    assert_eq!(resp["id"], 19);
    let result = &resp["result"];
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    let payload: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["schema_version"], "1");
    assert!(payload["findings"].is_array());
}
