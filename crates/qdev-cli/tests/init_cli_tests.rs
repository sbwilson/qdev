use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command as StdCommand;
use tempfile::TempDir;

use qdev_core::{rusqlite, CACHE_SCHEMA_VERSION, STANDARD_DIRECTORIES};

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
    assert!(stdout_str.contains(&format!("✔ cache schema v{}", CACHE_SCHEMA_VERSION)));

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
    assert_eq!(val["cache_schema_version"], CACHE_SCHEMA_VERSION);
    assert_eq!(val["cache_migrated"], false);
    assert_eq!(val["gitignore_updated"], true);
    assert!(val["created_files"]
        .as_array()
        .unwrap()
        .contains(&Value::String("qdev.toml".to_string())));
}

/// An older cache and no `--yes`: `init` migrates it and exits 0.
///
/// The `needs_confirmation` refusal this replaced protected nothing — every other command
/// performed the same drop-and-rebuild silently at boot, so a user told a migration needed
/// confirming could run `qdev list` instead and have it happen anyway.
#[test]
fn test_existing_workspace_older_cache_migrates_without_yes() {
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
        .success()
        .code(0)
        .stdout(predicate::str::contains(format!(
            "migrated to v{}",
            CACHE_SCHEMA_VERSION
        )))
        .stderr(predicate::str::contains("needs_confirmation").not());

    let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);
    drop(conn);

    // JSON mode, from an older stamp again: a success envelope, not an error one.
    {
        let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
        conn.execute_batch("PRAGMA user_version = 0;").unwrap();
    }
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
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");
    assert_eq!(val["schema_version"], "1");
    assert!(
        val.get("error").is_none(),
        "no error envelope: {}",
        stdout_str
    );
    assert_eq!(val["cache_migrated"], true);
    assert_eq!(val["cache_schema_version"], CACHE_SCHEMA_VERSION);
}

/// `--yes` is still accepted and changes nothing: it existed only for the migration gate, and
/// turning `qdev init --yes` into a usage error would break every script and CI job passing it.
#[test]
fn test_yes_is_still_accepted_and_changes_nothing() {
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
        .stdout(predicate::str::contains(format!(
            "migrated to v{}",
            CACHE_SCHEMA_VERSION
        )));

    // The same outcome as without the flag.
    let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);
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
        .stdout(predicate::str::contains(format!(
            "schema v{} up to date",
            CACHE_SCHEMA_VERSION
        )));

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
        .stdout(predicate::str::contains(format!(
            "✔ cache schema v{}",
            CACHE_SCHEMA_VERSION
        )));

    let qdev_content = fs::read_to_string(root.join("qdev.toml")).unwrap();
    assert!(qdev_content.contains("name = \"InteractiveApp\""));
    assert!(qdev_content.contains("\"ui-team\" = [\"bob\"]"));
}

/// Interactive `init` on an older cache asks nothing about the cache and migrates it. The stdin
/// here is a `n` — the answer that used to abort the migration — and it is simply never read.
#[test]
fn test_interactive_init_does_not_prompt_about_the_cache() {
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
        .success()
        .code(0)
        .stderr(predicate::str::contains("Migrate cache schema").not())
        .stdout(predicate::str::contains(format!(
            "migrated to v{}",
            CACHE_SCHEMA_VERSION
        )));

    let conn = rusqlite::Connection::open(cache_dir.join("cache.sqlite")).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |row| row.get(0))
        .unwrap();
    assert_eq!(user_version, CACHE_SCHEMA_VERSION);
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

/// A newer-than-supported cache is still exit 5 — with `--yes` and without it. Removing the
/// migration gate removed the refusal for an *older* cache only.
#[test]
fn test_future_schema_version_conflict_fails_with_and_without_yes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Create .qdev/cache/cache.sqlite stamped one past CACHE_SCHEMA_VERSION (newer than supported)
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let cache_file = cache_dir.join("cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_file).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION + 1
        ))
        .unwrap();
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

    // And without `--yes`, which is now inert either way.
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
        .code(5)
        .stderr(predicate::str::contains("newer than supported version"));
    assert!(!root.join("qdev.toml").exists());
}

/// The epic 1 cross-story review's reproduction (H1): `[storage] cache_dir` in
/// `.qdev.local.toml` used to give *two* databases — `init` created and gitignored
/// `.qdev/cache/cache.sqlite` while every later command created and used the configured one,
/// untracked only by luck. `init` now resolves through the same loader, so there is one cache, at
/// the path the commands use, and `.gitignore` covers it.
#[test]
fn test_local_cache_dir_yields_exactly_one_cache_covered_by_gitignore() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(root.join("qdev.toml"), "[project]\nname = \"Repro\"\n").unwrap();
    fs::write(
        root.join(".qdev.local.toml"),
        "[storage]\ncache_dir = \"local/cache\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Repro",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success()
        .code(0);

    // Any other command, which resolves the cache through the loader.
    let mut sync = Command::cargo_bin("qdev").unwrap();
    sync.current_dir(root)
        .args(["sync", "--json"])
        .assert()
        .success();

    assert!(root.join("local/cache/cache.sqlite").is_file());
    assert!(
        !root.join(".qdev/cache/cache.sqlite").exists(),
        "a second, abandoned database must not exist"
    );

    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|l| l.trim() == "local/cache/"),
        "the cache every command uses must be gitignored, got: {gitignore}"
    );
    assert!(
        gitignore.lines().any(|l| l.trim() == ".qdev/cache/"),
        "the committed ignore file must stay meaningful to everyone else, got: {gitignore}"
    );
}

/// `specs_dir` and `state_dir` select committed content, so they are a project decision. Either
/// key in the local file is a schema error naming the key and the file — from `init` too, which
/// is dispatched through the same loader as everything else.
#[test]
fn test_local_specs_dir_is_a_schema_error_for_init_and_for_other_commands() {
    for key in ["specs_dir", "state_dir"] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("qdev.toml"), "[project]\nname = \"Demo\"\n").unwrap();
        fs::write(
            root.join(".qdev.local.toml"),
            format!("[storage]\n{key} = \"elsewhere\"\n"),
        )
        .unwrap();

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
            .code(2);
        let output = assert.get_output();
        let val: Value =
            serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
        assert_eq!(val["error"]["details"]["key"], format!("storage.{key}"));
        assert_eq!(val["error"]["details"]["file"], ".qdev.local.toml");

        let mut other = Command::cargo_bin("qdev").unwrap();
        other
            .current_dir(root)
            .args(["config", "show"])
            .assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains(key))
            .stderr(predicate::str::contains(".qdev.local.toml"));
    }
}

/// An unparseable configuration file is the same refusal from `init` as from every other command
/// (M7): the tool's own remedy no longer reports success on a workspace no command can use, and
/// it does not scaffold the default layout over a configured one.
#[test]
fn test_unparseable_config_refuses_init_without_scaffolding() {
    for file in ["qdev.toml", ".qdev.local.toml"] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        fs::write(root.join("qdev.toml"), "[project]\nname = \"Demo\"\n").unwrap();
        fs::write(
            root.join(file),
            "[project]\nname = \"Demo\"\nthis is not toml = = =\n",
        )
        .unwrap();

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
            .code(2)
            .stderr(predicate::str::contains(file));

        assert!(!root.join(".qdev").exists(), "nothing may be scaffolded");
        assert!(!root.join("docs").exists(), "nothing may be scaffolded");
        assert!(
            !root.join(".gitignore").exists(),
            "nothing may be scaffolded"
        );
    }
}

/// A cache stamped by a newer binary is refused by `init` too, and the cache it inspected is the
/// one the commands use — not an abandoned default-layout file that happens to look healthy.
#[test]
fn test_newer_cache_in_configured_layout_refuses_init() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(root.join("qdev.toml"), "[project]\nname = \"Demo\"\n").unwrap();
    fs::write(
        root.join(".qdev.local.toml"),
        "[storage]\ncache_dir = \"local/cache\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("local/cache")).unwrap();
    {
        let conn = rusqlite::Connection::open(root.join("local/cache/cache.sqlite")).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION + 1
        ))
        .unwrap();
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
        .stderr(predicate::str::contains("schema_version_mismatch"));
}

/// Everything `init` reports it created is a path it actually created (L2): both the JSON payload
/// and the human output are rendered from the resolved layout.
#[test]
fn test_init_reports_only_the_configured_layout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(
        root.join("qdev.toml"),
        "[project]\nname = \"Configured\"\n\n[storage]\nspecs_dir = \"planning/specs\"\nstate_dir = \"planning/state\"\ncache_dir = \"var/cache\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Configured",
            "--developer",
            "alice",
            "--team",
            "core",
            "--json",
        ])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    for field in ["created_files", "created_directories"] {
        for path in val[field].as_array().unwrap() {
            let path = path.as_str().unwrap();
            assert!(
                root.join(path).exists(),
                "{field} names {path}, which does not exist on disk"
            );
        }
    }
    assert!(val["created_files"]
        .as_array()
        .unwrap()
        .contains(&Value::String("var/cache/cache.sqlite".to_string())));
    assert_eq!(val["storage"]["cache_dir"], "var/cache");

    // Text output names the configured layout and nothing outside it.
    let mut text_cmd = Command::cargo_bin("qdev").unwrap();
    let text_assert = text_cmd
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Configured",
            "--developer",
            "alice",
            "--team",
            "core",
        ])
        .assert()
        .success();
    let stdout = String::from_utf8(text_assert.get_output().stdout.clone()).unwrap();
    // The gate directory follows the *project* cache directory's parent, which this workspace
    // configures as `var/` — the rule story 1.2's `[storage]` amendment shipped.
    assert!(
        stdout.contains("✔ var/cache/ (gitignored), var/gates/"),
        "got: {stdout}"
    );
    assert!(stdout.contains("✔ planning/specs/{prd,requirements,epics,stories,adrs,hazards}"));
    assert!(stdout.contains(
        "✔ planning/state/{sprints,releases,dw,decisions,scratch,evidence,baselines,soup}"
    ));
    assert!(
        !stdout.contains("docs/specs") && !stdout.contains(".qdev/cache"),
        "the default layout must not be reported, got: {stdout}"
    );
}

/// Relocating the cache *after* `init` — the ordering the story's own verification note
/// prescribes — leaves the live cache outside `.gitignore` until `init` is re-run. `.gitignore` is
/// a committed file, so no ordinary command may edit it; re-running `init` is the remedy, and
/// this pins that it works and is idempotent.
#[test]
fn test_relocating_the_cache_after_init_is_repaired_by_re_running_init() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

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
        .success();

    let local = root.join(".qdev.local.toml");
    let existing = fs::read_to_string(&local).unwrap();
    fs::write(
        &local,
        format!("{existing}\n[storage]\ncache_dir = \"local/cache\"\n"),
    )
    .unwrap();

    // A plain command creates the relocated cache and does not touch `.gitignore`.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["status"]).assert().success();
    assert!(root.join("local/cache/cache.sqlite").is_file());
    let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        !ignore.contains("local/cache/"),
        "no ordinary command may edit the committed .gitignore: {ignore}"
    );

    // Re-running `init` is the remedy: it adds the entry, reports the workspace as already
    // initialised, and does not duplicate what is already there.
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
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["already_initialized"], true);

    let ignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        ignore.contains("local/cache/"),
        "re-running init must cover the relocated cache: {ignore}"
    );
    assert_eq!(
        ignore.matches(".qdev.local.toml").count(),
        1,
        "entries must not be duplicated by a re-run: {ignore}"
    );

    // And the config files are not re-reported as created on a re-run.
    let created: Vec<&str> = val["created_files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !created.contains(&"qdev.toml") && !created.contains(&".qdev.local.toml"),
        "a re-run creates neither config file: {created:?}"
    );
}

/// The defect end to end, through the binary: a cache with entity *and* child rows in it,
/// stamped to an older version, migrated by `qdev init` with no `--yes` and no FK failure — and
/// the boot path of another command doing the same thing to the same fixture afterwards.
///
/// Every migration test above this one uses an empty cache, which is why `init` shipped unable
/// to migrate any real workspace: `sqlite_error: Failed to drop table entities during migration:
/// FOREIGN KEY constraint failed`, exit 4, cache left at the old version.
#[test]
fn test_populated_older_cache_migrates_through_the_binary() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let init_args = [
        "init",
        "--non-interactive",
        "--name",
        "Demo",
        "--developer",
        "alice",
        "--team",
        "core",
    ];

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(init_args).assert().success();

    // A story with a constraint: an `entities` row, a child row in `stories` — whose
    // `stories.id REFERENCES entities(id)` is the foreign key the migration's drop order has to
    // survive — plus a `constraints` row, which has no foreign key of its own and is here only
    // so the fixture is a realistic story.
    fs::write(
        root.join("docs/specs/stories/E1S1.md"),
        r#"---
id: E1S1
title: "Only Story"
status: ready
version: 1
appetite: small
safety_class: ClassB
target_modules: ["foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: "No swift"
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
"#,
    )
    .unwrap();

    // Any command's boot hydrates it.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["status"]).assert().success();

    let cache_path = root.join(".qdev/cache/cache.sqlite");
    let row_counts = |path: &std::path::Path| -> (u32, u32, u32) {
        let conn = rusqlite::Connection::open(path).unwrap();
        let count = |table: &str| -> u32 {
            conn.query_row(&format!("SELECT count(*) FROM {};", table), [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        (count("entities"), count("stories"), count("constraints"))
    };
    assert_eq!(
        row_counts(&cache_path),
        (1, 1, 1),
        "fixture must have the entity, its `stories` child row and its constraint, or it pins \
         less than it claims"
    );

    let stamp_older = |path: &std::path::Path| {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA user_version = {};",
            CACHE_SCHEMA_VERSION - 1
        ))
        .unwrap();
    };
    let stamped_version = |path: &std::path::Path| -> u32 {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap()
    };

    // `qdev init`, no `--yes`: migrates, exit 0, and the rows come back from the files.
    stamp_older(&cache_path);
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(init_args)
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains(format!(
            "migrated to v{}",
            CACHE_SCHEMA_VERSION
        )));
    assert_eq!(stamped_version(&cache_path), CACHE_SCHEMA_VERSION);
    assert_eq!(
        row_counts(&cache_path),
        (1, 1, 1),
        "a migrated cache must be repopulated from the Markdown files"
    );

    // And the other path: another command's boot, on the same fixture, same outcome.
    stamp_older(&cache_path);
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["status"]).assert().success();
    assert_eq!(stamped_version(&cache_path), CACHE_SCHEMA_VERSION);
    assert_eq!(row_counts(&cache_path), (1, 1, 1));
}

/// `--yes` is accepted and inert. It must stay inert *only* for the retired migration
/// confirmation: the required-flag checks still exit 3, and no `--yes` may be read as blanket
/// consent to defaults. An inert-flag refactor is exactly what invites the opposite.
#[test]
fn test_yes_does_not_satisfy_the_required_flags() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    for missing in ["--name", "--developer", "--team"] {
        let mut args = vec!["init", "--non-interactive", "--yes"];
        if missing != "--name" {
            args.extend(["--name", "Demo"]);
        }
        if missing != "--developer" {
            args.extend(["--developer", "alice"]);
        }
        if missing != "--team" {
            args.extend(["--team", "core"]);
        }

        let mut cmd = Command::cargo_bin("qdev").unwrap();
        cmd.current_dir(root)
            .args(&args)
            .assert()
            .failure()
            .code(3)
            .stderr(predicate::str::contains(missing))
            .stderr(predicate::str::contains("needs_confirmation"));
    }

    assert!(
        !root.join("qdev.toml").exists(),
        "a refused init must scaffold nothing"
    );
}
