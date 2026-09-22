use std::collections::BTreeMap;
use std::fs;
use qdev_core::config::{Config, EnvironmentConfig, GateConfig, ModuleConfig};
use qdev_core::gate::{execute_gate, GateRunOptions, GateStatus, HeadTailBuffer};
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
