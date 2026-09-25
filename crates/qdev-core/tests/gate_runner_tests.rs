#![allow(clippy::field_reassign_with_default)]

use std::collections::BTreeMap;
use std::fs;
use qdev_core::config::{Config, EnvironmentConfig, GateConfig, ModuleConfig};
use qdev_core::errors::ExitCode;
use qdev_core::gate::{
    execute_gate, execute_gate_set, get_gate_list, resolve_gate_execution_order,
    validate_gate_dependencies, GateRunOptions, GateRunOutcome, GateRunSetOutcome, GateStatus,
    HeadTailBuffer,
};
use tempfile::TempDir;

fn setup_test_workspace() -> TempDir {
    let temp = TempDir::new().unwrap();
    // Initialize a git repo so git rev-parse HEAD works
    let _ = std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["init", "-b", "main"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.name", "Test User"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.email", "test@example.com"])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "commit.gpgsign", "false"])
        .output();

    fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
    let _ = std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["add", "."])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["commit", "-m", "initial commit"])
        .output();

    temp
}

#[test]
fn test_environment_propagation() {
    let temp = setup_test_workspace();

    let mut env_map = BTreeMap::new();
    env_map.insert("CUSTOM_KEY".to_string(), "custom_val_123".to_string());

    let mut config = Config::default();
    config.environment = EnvironmentConfig::new(env_map);
    config.modules = vec![ModuleConfig {
        id: "core".to_string(),
        paths: vec!["crates/core/**".to_string()],
        layer: Some(1),
        may_depend_on: vec![],
    }];

    // A shell script that prints environment variables
    let script_path = temp.path().join("check_env.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
echo "CUSTOM_KEY=$CUSTOM_KEY"
echo "QDEV_GATE=$QDEV_GATE"
echo "QDEV_STORY=$QDEV_STORY"
echo "QDEV_COMMIT=$QDEV_COMMIT"
echo "QDEV_RESULT_FILE=$QDEV_RESULT_FILE"
echo "QDEV_MODULE_PATHS=$QDEV_MODULE_PATHS"
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

    config.gates = vec![GateConfig {
        id: "env-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(10_000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions {
        story: Some("E12S4".to_string()),
        timeout_ms: None,
    };

    let outcome = execute_gate(temp.path(), &config, "env-gate", &options).unwrap();
    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);

    let stdout = outcome.stdout.expect("stdout should be captured");
    assert!(stdout.contains("CUSTOM_KEY=custom_val_123"));
    assert!(stdout.contains("QDEV_GATE=env-gate"));
    assert!(stdout.contains("QDEV_STORY=E12S4"));
    assert!(stdout.contains("QDEV_MODULE_PATHS={\"core\":[\"crates/core/**\"]}"));
    assert!(stdout.contains("QDEV_RESULT_FILE="));
    assert!(stdout.contains("QDEV_COMMIT="));
}

#[test]
fn test_head_tail_buffer_under_1mb() {
    let mut buf = HeadTailBuffer::new();
    // 500 KB of 'A'
    let data = vec![b'A'; 500 * 1024];
    buf.write_bytes(&data);
    assert_eq!(buf.truncated_bytes(), 0);
    assert!(!buf.is_truncated());
    let assembled = buf.to_bytes();
    assert_eq!(assembled.len(), 500 * 1024);
    assert_eq!(assembled, data);
}

#[test]
fn test_head_tail_buffer_over_1mb_retention_and_banner() {
    let mut buf = HeadTailBuffer::new();
    // Write 64 KB of 'H' (Header)
    let head_data = vec![b'H'; 64 * 1024];
    buf.write_bytes(&head_data);

    // Write 960 KB of 'M' (Middle, will be partially pushed out)
    let mid_data = vec![b'M'; 960 * 1024];
    buf.write_bytes(&mid_data);

    assert_eq!(buf.truncated_bytes(), 0);
    assert_eq!(buf.len(), 1024 * 1024);

    // Now write 100 KB of 'T' (Tail)
    let tail_data = vec![b'T'; 100 * 1024];
    buf.write_bytes(&tail_data);

    assert_eq!(buf.truncated_bytes(), 100 * 1024);
    assert!(buf.is_truncated());

    // In memory, buffer retains exactly 64 KB head + 960 KB tail = 1 MB (1,048,576 bytes)
    assert_eq!(buf.len(), 1024 * 1024);

    let output = buf.to_bytes();
    let banner = format!("\n[... qdev: {} bytes truncated ...]\n", 100 * 1024);

    // Verify head retained
    assert_eq!(&output[..64 * 1024], &head_data[..]);

    // Verify banner inserted immediately after head
    let after_head = &output[64 * 1024..64 * 1024 + banner.len()];
    assert_eq!(std::str::from_utf8(after_head).unwrap(), banner);

    // Verify tail ends with 'T'
    assert_eq!(&output[output.len() - (100 * 1024)..], &tail_data[..]);
}

#[test]
fn test_timeout_and_tree_kill() {
    let temp = setup_test_workspace();

    // Command that spawns background child sleep processes and hangs
    let script_path = temp.path().join("hang.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
# Spawn background grandchild
sleep 100 &
sleep 100 &
# Sleep in foreground
sleep 100
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "hang-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(200), // 200 ms timeout
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let start = std::time::Instant::now();
    let outcome = execute_gate(temp.path(), &config, "hang-gate", &options).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(outcome.status, GateStatus::Infra);
    assert_eq!(outcome.exit_code, 4);
    assert!(outcome.summary.contains("timeout after 200ms"));
    assert_eq!(
        outcome.receipt(),
        "[INFRA] hang-gate | timeout after 200ms | halt and alert"
    );
    assert_eq!(
        outcome.agent_instruction.as_deref(),
        Some("halt_and_alert")
    );
    // Should terminate quickly, well before 100 seconds
    assert!(elapsed.as_millis() < 5000);
}

#[test]
fn test_missing_executable_classification() {
    let temp = setup_test_workspace();

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "missing-gate".to_string(),
        command: Some("definitely-nonexistent-executable-12345".to_string()),
        timeout_ms: Some(1000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "missing-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Infra);
    assert_eq!(outcome.exit_code, 4);
    assert!(outcome
        .summary
        .contains("missing executable: definitely-nonexistent-executable-12345"));
    assert_eq!(
        outcome.receipt(),
        "[INFRA] missing-gate | missing executable: definitely-nonexistent-executable-12345 | halt and alert"
    );
    assert_eq!(
        outcome.agent_instruction.as_deref(),
        Some("halt_and_alert")
    );
}

#[test]
fn test_local_skip_honored() {
    let temp = setup_test_workspace();

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "skipped-gate".to_string(),
        command: Some("definitely-nonexistent-binary-must-not-run".to_string()),
        timeout_ms: Some(1000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: Some(true),
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "skipped-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Skip);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.duration_ms, 0);
    assert!(outcome.skipped_locally);
    assert_eq!(outcome.summary, "skipped_locally");
    assert_eq!(outcome.receipt(), "[SKIP] skipped-gate | skipped_locally");
}

#[test]
fn test_logical_failure_exit_code() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("fail.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
echo "some error occurred" >&2
exit 101
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "failing-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(1000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "failing-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 101);
    assert_eq!(outcome.summary, "some error occurred");
    assert_eq!(
        outcome.receipt(),
        "[FAIL] failing-gate (exit 101) | some error occurred"
    );
    assert_eq!(
        outcome.agent_instruction.as_deref(),
        Some("fix_cited_failures")
    );
}

#[test]
fn test_parse_command_args_multi_token_and_empty_strings() {
    use qdev_core::gate::runner::parse_command_args;

    let args = parse_command_args(r#"cargo test --package "foo bar" --message "" --other '' -v"#);
    assert_eq!(
        args,
        vec![
            "cargo".to_string(),
            "test".to_string(),
            "--package".to_string(),
            "foo bar".to_string(),
            "--message".to_string(),
            "".to_string(),
            "--other".to_string(),
            "".to_string(),
            "-v".to_string(),
        ]
    );

    let empty_only = parse_command_args(r#""""#);
    assert_eq!(empty_only, vec!["".to_string()]);

    let single_empty = parse_command_args(r#"''"#);
    assert_eq!(single_empty, vec!["".to_string()]);
}

#[test]
fn test_active_story_resolved_from_lease() {
    let temp = setup_test_workspace();

    // Create a lease in .qdev/leases
    let lease_dir = temp.path().join(".qdev").join("leases");
    fs::create_dir_all(&lease_dir).unwrap();
    let lease_json = r#"{
        "story_id": "E12S10",
        "holder": "simon",
        "author_type": "human",
        "worktree_path": "/tmp/test",
        "branch": "main",
        "started_at": "2026-09-22T00:00:00Z",
        "session_token": "token123"
    }"#;
    fs::write(lease_dir.join("E12S10.json"), lease_json).unwrap();

    let script_path = temp.path().join("check_story.sh");
    fs::write(
        &script_path,
        "#!/bin/sh\necho \"QDEV_STORY=$QDEV_STORY\"\nexit 0\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "lease-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(1000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    // Execute gate with NO story specified in options — should resolve from lease
    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "lease-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.story_id.as_deref(), Some("E12S10"));
    let stdout = outcome.stdout.expect("stdout should be captured");
    assert!(stdout.contains("QDEV_STORY=E12S10"));
}

#[test]
fn test_result_document_file_precedence_and_fields() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("write_result.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF' > "$QDEV_RESULT_FILE"
{
  "status": "fail",
  "summary": "1 of 18 tests failed",
  "failures": [
    {
      "location": "crates/bridge/tests/c_abi_round_trip.rs:142",
      "message": "assertion failed: left == right\n  left: EffectsUnavailable\n right: EventNotUnderstood"
    }
  ],
  "metric": 42.5,
  "constraint_ids": ["E12S4/NG-2"]
}
EOF
exit 101
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "result-doc-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "result-doc-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 101);
    assert_eq!(outcome.summary, "1 of 18 tests failed");
    assert_eq!(outcome.agent_instruction.as_deref(), Some("fix_cited_failures"));
    assert_eq!(outcome.metric, Some(42.5));
    assert_eq!(outcome.constraint_ids, vec!["E12S4/NG-2"]);
    assert_eq!(outcome.failures.len(), 1);
    assert_eq!(
        outcome.failures[0].location,
        "crates/bridge/tests/c_abi_round_trip.rs:142"
    );
    assert_eq!(
        outcome.receipt(),
        "[FAIL] result-doc-gate (exit 101) | crates/bridge/tests/c_abi_round_trip.rs:142 | assertion failed: left == right"
    );
}

#[test]
fn test_result_document_stdout_parsing() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("stdout_result.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF'
{
  "status": "pass",
  "summary": "all 18 tests passed",
  "failures": [],
  "metric": null,
  "constraint_ids": []
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "stdout-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "stdout-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.summary, "all 18 tests passed");
    assert_eq!(outcome.agent_instruction.as_deref(), Some("continue"));
    assert_eq!(outcome.failures.len(), 0);
}

#[test]
fn test_output_adapter_json_missing_fails_infra_exit_4() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("empty_json.sh");
    fs::write(&script_path, "#!/bin/sh\nexit 0\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "empty-json-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: Some("json".to_string()),
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "empty-json-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Infra);
    assert_eq!(outcome.exit_code, 4);
    assert_eq!(outcome.agent_instruction.as_deref(), Some("halt_and_alert"));
    assert!(outcome.summary.contains("produced no result document"));
}

#[test]
fn test_output_adapter_json_invalid_schema_fails_infra_exit_4() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("invalid_json.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF' > "$QDEV_RESULT_FILE"
{
  "status": "invalid_status",
  "summary": "something"
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "invalid-json-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: Some("json".to_string()),
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "invalid-json-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Infra);
    assert_eq!(outcome.exit_code, 4);
    assert_eq!(outcome.agent_instruction.as_deref(), Some("halt_and_alert"));
    assert!(outcome.summary.contains("schema validation failed"));
}

#[test]
#[cfg(unix)]
fn test_process_killed_by_signal_classified_infra_exit_4() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("self_kill.sh");
    fs::write(&script_path, "#!/bin/sh\nkill -9 $$\n").unwrap();

    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "sigkill-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "sigkill-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Infra);
    assert_eq!(outcome.exit_code, 4);
    assert_eq!(outcome.agent_instruction.as_deref(), Some("halt_and_alert"));
    assert!(outcome.summary.contains("terminated by signal 9"));
}

#[test]
fn test_fallback_40_lines_stderr_summary() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("many_stderr.sh");
    // Emit 50 lines to stderr; the summary should capture the last 40 lines
    let mut script = String::from("#!/bin/sh\n");
    for i in 1..=50 {
        script.push_str(&format!("echo 'stderr error line {}' >&2\n", i));
    }
    script.push_str("exit 101\n");
    fs::write(&script_path, &script).unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
    }

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "many-stderr-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "many-stderr-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Fail);
    assert_eq!(outcome.exit_code, 101);
    assert_eq!(outcome.agent_instruction.as_deref(), Some("fix_cited_failures"));
    let summary_lines: Vec<&str> = outcome.summary.lines().collect();
    assert_eq!(summary_lines.len(), 40);
    assert_eq!(summary_lines[0], "stderr error line 11");
    assert_eq!(summary_lines[39], "stderr error line 50");
}

#[test]
fn test_receipt_formatting_multiple_failures() {
    let outcome = qdev_core::GateRunOutcome {
        gate_id: "test-gate".to_string(),
        status: GateStatus::Fail,
        exit_code: 1,
        duration_ms: 50,
        summary: "2 of 2 tests failed".to_string(),
        skipped_locally: false,
        commit_sha: None,
        story_id: None,
        stdout: None,
        stderr: None,
        agent_instruction: Some("fix_cited_failures".to_string()),
        failures: vec![
            qdev_core::GateFailure {
                location: "src/foo.rs:10".to_string(),
                message: "assertion failed: a == b\n extra details".to_string(),
            },
            qdev_core::GateFailure {
                location: "src/bar.rs:25".to_string(),
                message: "assertion failed: x == y".to_string(),
            },
        ],
        metric: None,
        constraint_ids: Vec::new(),
        evidence_path: None,
    };

    let receipt = outcome.receipt();
    let lines: Vec<&str> = receipt.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "[FAIL] test-gate (exit 1) | src/foo.rs:10 | assertion failed: a == b");
    assert_eq!(lines[1], "[FAIL] test-gate (exit 1) | src/bar.rs:25 | assertion failed: x == y");
}

#[test]
fn test_receipt_formatting_empty_message_omits_trailing_separator() {
    let outcome = qdev_core::GateRunOutcome {
        gate_id: "empty-msg-gate".to_string(),
        status: GateStatus::Fail,
        exit_code: 1,
        duration_ms: 10,
        summary: "1 test failed".to_string(),
        skipped_locally: false,
        commit_sha: None,
        story_id: None,
        stdout: None,
        stderr: None,
        agent_instruction: Some("fix_cited_failures".to_string()),
        failures: vec![qdev_core::GateFailure {
            location: "src/empty.rs:5".to_string(),
            message: "".to_string(),
        }],
        metric: None,
        constraint_ids: Vec::new(),
        evidence_path: None,
    };

    assert_eq!(outcome.receipt(), "[FAIL] empty-msg-gate (exit 1) | src/empty.rs:5");
}

#[test]
fn test_output_adapter_json_valid_document_success() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("json_success.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF' > "$QDEV_RESULT_FILE"
{
  "status": "pass",
  "summary": "18 tests passed",
  "failures": [],
  "metric": 100.0,
  "constraint_ids": ["FR-101"]
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "json-success-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: Some("json".to_string()),
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "json-success-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.summary, "18 tests passed");
    assert_eq!(outcome.agent_instruction.as_deref(), Some("continue"));
    assert_eq!(outcome.metric, Some(100.0));
    assert_eq!(outcome.constraint_ids, vec!["FR-101"]);
    assert!(outcome.failures.is_empty());
}

#[test]
fn test_output_adapter_json_fallback_from_empty_result_file_to_stdout() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("json_stdout_fallback.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
# Touch empty result file, write valid JSON to stdout
touch "$QDEV_RESULT_FILE"
cat << 'EOF'
{
  "status": "pass",
  "summary": "stdout result passed",
  "failures": [],
  "metric": null,
  "constraint_ids": []
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "json-stdout-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: Some("json".to_string()),
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "json-stdout-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.summary, "stdout result passed");
    assert_eq!(outcome.agent_instruction.as_deref(), Some("continue"));
}

#[test]
fn test_output_adapter_cargo_success() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("cargo_pass.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF'
running 18 tests
test test_one ... ok
test test_eighteen ... ok

test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "cargo-success-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: Some("cargo".to_string()),
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "cargo-success-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.summary, "18 tests passed");
    assert_eq!(outcome.agent_instruction.as_deref(), Some("continue"));
    assert!(outcome.failures.is_empty());
}

#[test]
fn test_output_adapter_xcodebuild_success() {
    let temp = setup_test_workspace();

    let script_path = temp.path().join("xcode_pass.sh");
    fs::write(
        &script_path,
        r#"#!/bin/sh
cat << 'EOF'
Test Suite 'All tests' started at 2026-09-22 10:00:00.000
	 Executed 18 tests, with 0 failures (0 unexpected) in 1.234 (1.234) seconds
** TEST SUCCEEDED **
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

    let mut config = Config::default();
    config.gates = vec![GateConfig {
        id: "xcode-success-gate".to_string(),
        command: Some(script_path.to_string_lossy().to_string()),
        timeout_ms: Some(5000),
        depends_on: vec![],
        output_adapter: Some("xcodebuild".to_string()),
        on_transition: vec![],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    }];

    let options = GateRunOptions::default();
    let outcome = execute_gate(temp.path(), &config, "xcode-success-gate", &options).unwrap();

    assert_eq!(outcome.status, GateStatus::Pass);
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(outcome.summary, "18 tests passed");
    assert_eq!(outcome.agent_instruction.as_deref(), Some("continue"));
    assert!(outcome.failures.is_empty());
}

#[test]
fn test_topological_sort_kahn_declaration_order_tie_break() {
    let config = Config {
        gates: vec![
            GateConfig {
                id: "C".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec!["B".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "B".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec!["A".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "D".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "A".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let order = resolve_gate_execution_order(&config.gates, None).unwrap();
    // D and A both have in_degree 0 initially.
    // D is declared before A, so D runs first, then A.
    // With A resolved, B has in_degree 0, so B runs.
    // With B resolved, C runs.
    assert_eq!(order, vec!["D", "A", "B", "C"]);

    // Test targeting C and its transitive dependencies
    let target_order = resolve_gate_execution_order(&config.gates, Some(&["C".to_string()])).unwrap();
    assert_eq!(target_order, vec!["A", "B", "C"]);
}

#[test]
fn test_circular_dependency_rejection_exit_code_2() {
    let config = Config {
        gates: vec![
            GateConfig {
                id: "A".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec!["B".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "B".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec!["A".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let err = validate_gate_dependencies(&config.gates).unwrap_err();
    assert_eq!(err.exit_code, ExitCode::UsageError);
    assert!(err.message.contains("A -> B -> A") || err.message.contains("B -> A -> B"));
}

#[test]
fn test_unknown_dependency_rejection_exit_code_2() {
    let config = Config {
        gates: vec![GateConfig {
            id: "A".to_string(),
            command: Some("true".to_string()),
            timeout_ms: None,
            depends_on: vec!["nonexistent".to_string()],
            output_adapter: None,
            on_transition: vec![],
            verifies: vec![],
            kind: None,
            metric: None,
            direction: None,
            skip: None,
        }],
        ..Default::default()
    };

    let err = validate_gate_dependencies(&config.gates).unwrap_err();
    assert_eq!(err.exit_code, ExitCode::UsageError);
    assert!(err.message.contains("gate 'A' depends on unknown gate 'nonexistent'"));
}

#[test]
fn test_prerequisite_logical_failure_cascades_skip() {
    let temp = setup_test_workspace();

    let fail_script = temp.path().join("fail.sh");
    fs::write(&fail_script, "#!/bin/sh\necho 'assertion failed'\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fail_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fail_script, perms).unwrap();
    }

    let pass_script = temp.path().join("pass.sh");
    fs::write(&pass_script, "#!/bin/sh\necho 'passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&pass_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&pass_script, perms).unwrap();
    }

    let config = Config {
        gates: vec![
            GateConfig {
                id: "A".to_string(),
                command: Some(fail_script.to_string_lossy().to_string()),
                timeout_ms: Some(5000),
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "B".to_string(),
                command: Some(pass_script.to_string_lossy().to_string()),
                timeout_ms: Some(5000),
                depends_on: vec!["A".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let options = GateRunOptions::default();
    let set_outcome =
        execute_gate_set(temp.path(), &config, &["A".to_string(), "B".to_string()], &options).unwrap();

    assert_eq!(set_outcome.outcomes.len(), 2);
    assert_eq!(set_outcome.outcomes[0].gate_id, "A");
    assert_eq!(set_outcome.outcomes[0].status, GateStatus::Fail);

    assert_eq!(set_outcome.outcomes[1].gate_id, "B");
    assert_eq!(set_outcome.outcomes[1].status, GateStatus::Skip);
    assert_eq!(set_outcome.outcomes[1].summary, "dependency failed: A");
    assert!(!set_outcome.outcomes[1].skipped_locally);
    assert_eq!(set_outcome.outcomes[1].receipt(), "[SKIP] B | dependency failed: A");

    assert_eq!(set_outcome.aggregate_exit_code(), ExitCode::LogicalFailure);
}

#[test]
fn test_prerequisite_infra_failure_cascades_skip() {
    let temp = setup_test_workspace();

    let config = Config {
        gates: vec![
            GateConfig {
                id: "A".to_string(),
                command: Some("nonexistent_executable_12345".to_string()),
                timeout_ms: Some(5000),
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "B".to_string(),
                command: Some("true".to_string()),
                timeout_ms: Some(5000),
                depends_on: vec!["A".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let options = GateRunOptions::default();
    let set_outcome =
        execute_gate_set(temp.path(), &config, &["A".to_string(), "B".to_string()], &options).unwrap();

    assert_eq!(set_outcome.outcomes.len(), 2);
    assert_eq!(set_outcome.outcomes[0].status, GateStatus::Infra);
    assert_eq!(set_outcome.outcomes[1].status, GateStatus::Skip);
    assert_eq!(set_outcome.outcomes[1].summary, "dependency failed: A");
    assert_eq!(set_outcome.aggregate_exit_code(), ExitCode::InfrastructureFailure);
}

#[test]
fn test_multi_level_cascade_skip() {
    let temp = setup_test_workspace();

    let fail_script = temp.path().join("fail.sh");
    fs::write(&fail_script, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&fail_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&fail_script, perms).unwrap();
    }

    let config = Config {
        gates: vec![
            GateConfig {
                id: "A".to_string(),
                command: Some(fail_script.to_string_lossy().to_string()),
                timeout_ms: Some(5000),
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "B".to_string(),
                command: Some("true".to_string()),
                timeout_ms: Some(5000),
                depends_on: vec!["A".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "C".to_string(),
                command: Some("true".to_string()),
                timeout_ms: Some(5000),
                depends_on: vec!["B".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let options = GateRunOptions::default();
    let set_outcome = execute_gate_set(
        temp.path(),
        &config,
        &["A".to_string(), "B".to_string(), "C".to_string()],
        &options,
    )
    .unwrap();

    assert_eq!(set_outcome.outcomes.len(), 3);
    assert_eq!(set_outcome.outcomes[0].status, GateStatus::Fail);
    assert_eq!(set_outcome.outcomes[1].status, GateStatus::Skip);
    assert_eq!(set_outcome.outcomes[1].summary, "dependency failed: A");
    assert_eq!(set_outcome.outcomes[2].status, GateStatus::Skip);
    assert_eq!(set_outcome.outcomes[2].summary, "dependency failed: B");
    assert_eq!(set_outcome.outcomes[2].receipt(), "[SKIP] C | dependency failed: B");

    assert_eq!(set_outcome.aggregate_exit_code(), ExitCode::LogicalFailure);
}

#[test]
fn test_local_skip_does_not_cascade() {
    let temp = setup_test_workspace();

    let pass_script = temp.path().join("pass.sh");
    fs::write(&pass_script, "#!/bin/sh\necho 'b passed'\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&pass_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&pass_script, perms).unwrap();
    }

    let config = Config {
        gates: vec![
            GateConfig {
                id: "A".to_string(),
                command: Some("true".to_string()),
                timeout_ms: Some(5000),
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: Some(true),
            },
            GateConfig {
                id: "B".to_string(),
                command: Some(pass_script.to_string_lossy().to_string()),
                timeout_ms: Some(5000),
                depends_on: vec!["A".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let options = GateRunOptions::default();
    let set_outcome =
        execute_gate_set(temp.path(), &config, &["A".to_string(), "B".to_string()], &options).unwrap();

    assert_eq!(set_outcome.outcomes.len(), 2);
    assert_eq!(set_outcome.outcomes[0].status, GateStatus::Skip);
    assert!(set_outcome.outcomes[0].skipped_locally);
    assert_eq!(set_outcome.outcomes[0].receipt(), "[SKIP] A | skipped_locally");

    assert_eq!(set_outcome.outcomes[1].status, GateStatus::Pass);
    assert_eq!(set_outcome.aggregate_exit_code(), ExitCode::Success);
}

#[test]
fn test_aggregate_exit_code_precedence() {
    let outcome_fail = GateRunOutcome {
        gate_id: "fail-gate".to_string(),
        status: GateStatus::Fail,
        exit_code: 1,
        duration_ms: 10,
        summary: "failed".to_string(),
        skipped_locally: false,
        commit_sha: None,
        story_id: None,
        stdout: None,
        stderr: None,
        agent_instruction: None,
        failures: vec![],
        metric: None,
        constraint_ids: vec![],
        evidence_path: None,
    };

    let outcome_infra = GateRunOutcome {
        gate_id: "infra-gate".to_string(),
        status: GateStatus::Infra,
        exit_code: 4,
        duration_ms: 10,
        summary: "timeout".to_string(),
        skipped_locally: false,
        commit_sha: None,
        story_id: None,
        stdout: None,
        stderr: None,
        agent_instruction: None,
        failures: vec![],
        metric: None,
        constraint_ids: vec![],
        evidence_path: None,
    };

    let outcome_pass = GateRunOutcome {
        gate_id: "pass-gate".to_string(),
        status: GateStatus::Pass,
        exit_code: 0,
        duration_ms: 10,
        summary: "passed".to_string(),
        skipped_locally: false,
        commit_sha: None,
        story_id: None,
        stdout: None,
        stderr: None,
        agent_instruction: None,
        failures: vec![],
        metric: None,
        constraint_ids: vec![],
        evidence_path: None,
    };

    // Fail + Infra -> ExitCode::LogicalFailure (1)
    let set1 = GateRunSetOutcome::new(vec![outcome_fail.clone(), outcome_infra.clone()]);
    assert_eq!(set1.aggregate_exit_code(), ExitCode::LogicalFailure);

    // Infra + Pass -> ExitCode::InfrastructureFailure (4)
    let set2 = GateRunSetOutcome::new(vec![outcome_infra.clone(), outcome_pass.clone()]);
    assert_eq!(set2.aggregate_exit_code(), ExitCode::InfrastructureFailure);

    // Only Pass -> ExitCode::Success (0)
    let set3 = GateRunSetOutcome::new(vec![outcome_pass.clone()]);
    assert_eq!(set3.aggregate_exit_code(), ExitCode::Success);
}

#[test]
fn test_get_gate_list_with_evidence() {
    let temp = setup_test_workspace();

    // Create an evidence file for "lint"
    let ev_dir = temp.path().join("docs/state/evidence/E12S4");
    fs::create_dir_all(&ev_dir).unwrap();
    let ev_file = ev_dir.join("8f1b2c4-lint.json");
    fs::write(
        &ev_file,
        r#"{
  "schema_version": "1",
  "gate": "lint",
  "status": "pass",
  "exit_code": 0,
  "duration_ms": 120,
  "summary": "all ok",
  "skipped_locally": false,
  "ran_at": "2026-09-23T08:00:00Z"
}"#,
    )
    .unwrap();

    let config = Config {
        gates: vec![
            GateConfig {
                id: "lint".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec!["review".to_string()],
                verifies: vec![],
                kind: Some("check".to_string()),
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "integration".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec!["lint".to_string()],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let items = get_gate_list(temp.path(), &config).unwrap();
    assert_eq!(items.len(), 2);

    assert_eq!(items[0].id, "lint");
    assert_eq!(items[0].kind, "check");
    assert_eq!(items[0].transitions, vec!["review"]);
    assert_eq!(items[0].dependencies, Vec::<String>::new());
    assert_eq!(items[0].last_status.as_deref(), Some("pass"));

    assert_eq!(items[1].id, "integration");
    assert_eq!(items[1].kind, "command"); // default kind
    assert_eq!(items[1].transitions, Vec::<String>::new());
    assert_eq!(items[1].dependencies, vec!["lint"]);
    assert_eq!(items[1].last_status, None);
}

#[test]
fn test_duplicate_gate_id_rejection_exit_code_2() {
    let config = Config {
        gates: vec![
            GateConfig {
                id: "dup".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "dup".to_string(),
                command: Some("true".to_string()),
                timeout_ms: None,
                depends_on: vec![],
                output_adapter: None,
                on_transition: vec![],
                verifies: vec![],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        ..Default::default()
    };

    let err = validate_gate_dependencies(&config.gates).unwrap_err();
    assert_eq!(err.exit_code, ExitCode::UsageError);
    assert!(err.message.contains("duplicate gate id 'dup'"));
}

#[test]
fn test_get_gate_list_with_sqlite_store() {
    use qdev_core::store::Store;

    let temp = setup_test_workspace();

    let config = Config {
        gates: vec![GateConfig {
            id: "test-gate".to_string(),
            command: Some("true".to_string()),
            timeout_ms: None,
            depends_on: vec![],
            output_adapter: None,
            on_transition: vec![],
            verifies: vec![],
            kind: None,
            metric: None,
            direction: None,
            skip: None,
        }],
        ..Default::default()
    };

    let store = qdev_core::store::ensure_cache(temp.path(), &config.storage).unwrap();

    let record = qdev_core::store::GateRunRecord {
        id: "run-1".to_string(),
        story_id: Some("E12S4".to_string()),
        gate_id: "test-gate".to_string(),
        commit_sha: "8f1b2c4".to_string(),
        status: Some("pass".to_string()),
        exit_code: Some(0),
        duration_ms: Some(150),
        metric_value: None,
        summary: Some("all passed".to_string()),
        evidence_path: "docs/state/evidence/E12S4/8f1b2c4-test-gate.json".to_string(),
        output_hash: None,
        run_by_type: None,
        run_by_id: None,
        ran_at: Some("2026-09-23T10:00:00Z".to_string()),
    };
    store.upsert_gate_run(&record).unwrap();

    let items = get_gate_list(temp.path(), &config).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "test-gate");
    assert_eq!(items[0].last_status.as_deref(), Some("pass"));
}

#[test]
fn test_get_gate_list_respects_custom_state_dir() {
    let temp = setup_test_workspace();

    let custom_evidence_dir = temp.path().join("custom_state/evidence/E1S1");
    fs::create_dir_all(&custom_evidence_dir).unwrap();
    let ev_file = custom_evidence_dir.join("run.json");
    fs::write(
        &ev_file,
        r#"{
  "gate": "custom-gate",
  "status": "infra",
  "ran_at": "2026-09-23T10:00:00Z"
}"#,
    )
    .unwrap();

    let mut config = Config {
        gates: vec![GateConfig {
            id: "custom-gate".to_string(),
            command: Some("true".to_string()),
            timeout_ms: None,
            depends_on: vec![],
            output_adapter: None,
            on_transition: vec![],
            verifies: vec![],
            kind: None,
            metric: None,
            direction: None,
            skip: None,
        }],
        ..Default::default()
    };
    config.storage.state_dir = "custom_state".to_string();

    let items = get_gate_list(temp.path(), &config).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "custom-gate");
    assert_eq!(items[0].last_status.as_deref(), Some("infra"));
}


