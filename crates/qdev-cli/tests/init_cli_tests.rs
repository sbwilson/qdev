use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

use qdev_core::{rusqlite, STANDARD_DIRECTORIES};

#[test]
fn test_non_interactive_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core-platform",
        ])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();

    // Verify stdout contains bullet points per CLI reference §7
    assert!(stdout_str.contains("✔ qdev.toml"));
    assert!(stdout_str.contains("✔ .qdev.local.toml (gitignored)"));
    assert!(stdout_str.contains("✔ .qdev/cache/ (gitignored), .qdev/gates/"));
    assert!(stdout_str.contains("✔ docs/specs/{prd,requirements,epics,stories,adrs,hazards}"));
    assert!(stdout_str
        .contains("✔ docs/state/{sprints,releases,dw,decisions,scratch,evidence,baselines,soup}"));
    assert!(stdout_str.contains("✔ cache schema v1"));

    // Verify created files and directories
    assert!(root.join("qdev.toml").is_file());
    assert!(root.join(".qdev.local.toml").is_file());
    assert!(root.join(".gitignore").is_file());
    assert!(root.join(".qdev/cache/cache.sqlite").is_file());

    for &dir in STANDARD_DIRECTORIES {
        assert!(root.join(dir).is_dir(), "Expected directory: {}", dir);
    }

    // Verify qdev.toml contents
    let qdev_content = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert!(qdev_content.contains("name = \"Demo\""));
    assert!(qdev_content.contains("\"core-platform\" = [\"alice\"]"));

    // Verify .qdev.local.toml contents
    let local_content = fs::read_to_string(root.join(".qdev.local.toml")).unwrap();
    assert!(local_content.contains("developer_id = \"alice\""));
    assert!(local_content.contains("core-platform"));

    // Verify .gitignore contents
    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(gitignore.contains(".qdev/cache/"));
    assert!(gitignore.contains(".qdev/leases/"));
    assert!(gitignore.contains(".qdev.local.toml"));

    // Verify scaffolded config validity via config show --json
    let mut config_cmd = Command::cargo_bin("qdev").unwrap();
    let assert_cfg = config_cmd
        .current_dir(root)
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .code(0);
    let output_cfg = assert_cfg.get_output();
    let val_cfg: Value =
        serde_json::from_str(std::str::from_utf8(&output_cfg.stdout).unwrap()).unwrap();
    assert_eq!(val_cfg["schema_version"], "1");
    assert_eq!(val_cfg["config"]["project"]["name"], "Demo");
    assert_eq!(val_cfg["config"]["identity"]["developer_id"], "alice");
}

#[test]
fn test_non_interactive_missing_name() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Text mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--developer",
            "alice",
            "--team",
            "core-platform",
        ])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("--name"))
        .stderr(predicate::str::contains("needs_confirmation"));

    // JSON mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--developer",
            "alice",
            "--team",
            "core-platform",
            "--json",
        ])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "needs_confirmation");
    assert!(val["error"]["message"].as_str().unwrap().contains("--name"));
    assert_eq!(val["error"]["details"]["flag"], "--name");
}

#[test]
fn test_non_interactive_missing_developer() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Text mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--team",
            "core-platform",
        ])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("--developer"))
        .stderr(predicate::str::contains("needs_confirmation"));

    // JSON mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--team",
            "core-platform",
            "--json",
        ])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "needs_confirmation");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--developer"));
    assert_eq!(val["error"]["details"]["flag"], "--developer");
}

#[test]
fn test_non_interactive_missing_team() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Text mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
        ])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("--team"))
        .stderr(predicate::str::contains("needs_confirmation"));

    // JSON mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--json",
        ])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "needs_confirmation");
    assert!(val["error"]["message"].as_str().unwrap().contains("--team"));
    assert_eq!(val["error"]["details"]["flag"], "--team");
}

#[test]
fn test_missing_flags_in_pipe_non_tty() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // When run via assert_cmd (stdin is not a TTY) without --non-interactive flag,
    // it must automatically resolve to NonInteractive and fail closed with code 3.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["init"])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("needs_confirmation"))
        .stderr(predicate::str::contains("--name"));
}

#[test]
fn test_subdirectory_execution() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path();

    // Initialize a git repo at repo_root
    let git_status = StdCommand::new("git")
        .args(["init"])
        .current_dir(repo_root)
        .status()
        .expect("Failed to initialize git repository");
    assert!(git_status.success());

    // Create nested subdirectory
    let nested_dir = repo_root.join("subdir").join("nested");
    fs::create_dir_all(&nested_dir).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(&nested_dir)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success()
        .code(0);

    // Created files must be at repo_root, NOT inside nested_dir!
    assert!(repo_root.join("qdev.toml").is_file());
    assert!(repo_root.join(".qdev.local.toml").is_file());
    assert!(repo_root.join(".qdev/cache/cache.sqlite").is_file());
    assert!(!nested_dir.join("qdev.toml").exists());
    assert!(!nested_dir.join(".qdev.local.toml").exists());
}

#[test]
fn test_non_interactive_json_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--json",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert!(val.get("root").is_some());
    assert_eq!(val["cache_schema_version"], 1);
    assert_eq!(val["cache_migrated"], false);
    assert_eq!(val["gitignore_updated"], true);
    assert!(val["created_files"]
        .as_array()
        .unwrap()
        .contains(&Value::String("qdev.toml".to_string())));
}

#[test]
fn test_existing_workspace_older_cache_migration_refused_without_yes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Pre-create cache.sqlite at user_version 0
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 0;").unwrap();
    }

    // Text mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("migration"))
        .stderr(predicate::str::contains("--yes"));

    // JSON mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
            "--json",
        ])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "needs_confirmation");
    assert_eq!(val["error"]["details"]["flag"], "--yes");
}

#[test]
fn test_existing_workspace_older_cache_migration_with_yes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Pre-create cache.sqlite at user_version 0
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 0;").unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
            "--yes",
        ])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("migrated to v1"));

    // Verify cache.sqlite user_version is now 1
    let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 1);
}

#[test]
fn test_idempotent_rerun_current_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Initial run
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success()
        .code(0);

    // Modify qdev.toml to test that configuration is preserved
    let custom_config = "[project]\nname = \"CustomDemo\"\n[teams]\ncore = [\"alice\"]\n";
    fs::write(root.join("qdev.toml"), custom_config).unwrap();

    // Second run
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "DifferentName",
            "--developer",
            "bob",
            "--team",
            "other",
        ])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("schema v1 up to date"));

    // Configuration preserved
    let preserved = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert_eq!(preserved, custom_config);
}

#[test]
fn test_interactive_wizard_prompt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Test interactive prompt wizard using mocked TTY and simulated stdin
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .write_stdin("InteractiveApp\nbob\nui-team\n")
        .args(["init"])
        .assert()
        .success()
        .code(0)
        .stderr(predicate::str::contains("Project name:"))
        .stderr(predicate::str::contains("Developer ID"))
        .stderr(predicate::str::contains("Team(s)"))
        .stdout(predicate::str::contains("✔ qdev.toml"))
        .stdout(predicate::str::contains("✔ cache schema v1"));

    let qdev_content = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert!(qdev_content.contains("name = \"InteractiveApp\""));
    assert!(qdev_content.contains("\"ui-team\" = [\"bob\"]"));
}

#[test]
fn test_interactive_migration_confirmation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Pre-create cache.sqlite at user_version 0
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 0;").unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .write_stdin("y\n")
        .args([
            "init",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success()
        .code(0)
        .stderr(predicate::str::contains("Migrate cache schema"))
        .stdout(predicate::str::contains("migrated to v1"));

    let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, 1);
}

#[test]
fn test_interactive_migration_refusal_with_n() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Pre-create cache.sqlite at user_version 0
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    {
        let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 0;").unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .write_stdin("n\n")
        .args([
            "init",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .failure()
        .code(3)
        .stderr(predicate::str::contains("Migrate cache schema"))
        .stderr(predicate::str::contains("needs_confirmation"));
}

#[test]
fn test_interactive_wizard_git_email_fallback() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Initialize git repository with local user.email
    let git_init = StdCommand::new("git")
        .args(["init"])
        .current_dir(root)
        .status()
        .expect("git init failed");
    assert!(git_init.success());

    let git_config = StdCommand::new("git")
        .args(["config", "user.email", "gituser@example.com"])
        .current_dir(root)
        .status()
        .expect("git config user.email failed");
    assert!(git_config.success());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .write_stdin("GitFallbackApp\n\nplatform-team\n") // empty string for developer
        .args(["init"])
        .assert()
        .success()
        .code(0)
        .stderr(predicate::str::contains(
            "Developer ID [gituser@example.com]:",
        ));

    let local_content = fs::read_to_string(root.join(".qdev.local.toml")).unwrap();
    assert!(local_content.contains("developer_id = \"gituser@example.com\""));
}

#[test]
fn test_future_schema_version_conflict_fails_with_yes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create .qdev/cache/cache.sqlite with user_version = 2
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_file = cache_dir.join("cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_file).unwrap();
        conn.execute_batch("PRAGMA user_version = 2;").unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Demo",
            "--developer",
            "alice",
            "--team",
            "core",
            "--yes",
        ])
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("newer than supported version"));

    // Verify filesystem was not modified before conflict failure
    assert!(!root.join("qdev.toml").exists());
}
