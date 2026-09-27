use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("failed to run `git {}`: {}", args.join(" "), e));
    assert!(
        output.status.success(),
        "`git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "HookTest",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();

    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    // Initial commit so HEAD exists
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial commit"]);
}

fn create_story(root: &Path, id: &str, title: &str) {
    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(
        stories.join(format!("{id}.md")),
        format!(
            r#"---
id: {id}
title: "{title}"
status: in-progress
version: 1
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#
        ),
    )
    .unwrap();
}

fn claim_story(root: &Path, id: &str) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["claim", "story", id])
        .assert()
        .success();
}

fn enable_commit_messages(root: &Path, format: &str) {
    let path = root.join("qdev.toml");
    let mut config = fs::read_to_string(&path).unwrap();
    config.push_str(&format!(
        "\n[commit_messages]\nenabled = true\nformat = \"{format}\"\n"
    ));
    fs::write(path, config).unwrap();
}

#[test]
fn test_cli_install_hooks_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["install", "hooks"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("Installed 3 git hook(s)"));
    assert!(stdout.contains("pre-commit"));
    assert!(stdout.contains("pre-push"));
    assert!(stdout.contains("prepare-commit-msg"));

    let hooks_dir = root.join(".git").join("hooks");
    assert!(hooks_dir.join("pre-commit").exists());
    assert!(hooks_dir.join("pre-push").exists());
    assert!(hooks_dir.join("prepare-commit-msg").exists());

    let pre_commit_content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
    assert_eq!(
        pre_commit_content,
        "#!/bin/sh\nexec qdev hook pre-commit \"$@\"\n"
    );
}

#[test]
fn test_cli_install_hooks_json_validates_schema() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Fetch schema via qdev schema payload hook_install
    let mut schema_cmd = Command::cargo_bin("qdev").unwrap();
    let schema_assert = schema_cmd
        .current_dir(root)
        .args(["schema", "payload", "hook_install"])
        .assert()
        .success();
    let schema_json: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    let compiled_schema = jsonschema::validator_for(&schema_json).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["install", "hooks", "--json"])
        .assert()
        .success();

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert!(
        compiled_schema.is_valid(&payload),
        "hook_install output must conform to its schema"
    );

    assert_eq!(payload["installed_hooks"].as_array().unwrap().len(), 3);
}

#[test]
fn test_cli_install_hooks_preserves_legacy() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let hooks_dir = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    fs::write(
        hooks_dir.join("pre-commit"),
        "#!/bin/sh\necho 'custom legacy'\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["install", "hooks"])
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("Preserved non-qdev hook: pre-commit -> pre-commit.legacy"));

    assert!(hooks_dir.join("pre-commit.legacy").exists());
    let legacy_content = fs::read_to_string(hooks_dir.join("pre-commit.legacy")).unwrap();
    assert_eq!(legacy_content, "#!/bin/sh\necho 'custom legacy'\n");

    let new_content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
    assert_eq!(new_content, "#!/bin/sh\nexec qdev hook pre-commit \"$@\"\n");
}

#[test]
fn test_cli_hook_pre_commit_clean() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "pre-commit"])
        .assert()
        .success();
}

#[test]
fn test_cli_hook_pre_commit_secret_blocks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add secret_patterns to qdev.toml
    let qdev_toml = root.join("qdev.toml");
    let mut config_text = fs::read_to_string(&qdev_toml).unwrap();
    config_text.push_str("\n[hygiene]\nsecret_patterns = [\"DO_NOT_COMMIT_[A-Z0-9]+\"]\n");
    fs::write(&qdev_toml, config_text).unwrap();

    // Stage scratchpad containing the secret
    let scratch_dir = root.join("docs").join("state").join("scratch");
    fs::create_dir_all(&scratch_dir).unwrap();
    let scratch_file = scratch_dir.join("notes.jsonl");
    fs::write(&scratch_file, "{\"secret\": \"DO_NOT_COMMIT_KEY123\"}\n").unwrap();
    git(root, &["add", "docs/state/scratch/notes.jsonl"]);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hook", "pre-commit"])
        .assert()
        .code(1);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(stderr.contains("secret pattern match"));
    assert!(stderr.contains("DO_NOT_COMMIT_[A-Z0-9]+"));
    assert!(stderr.contains("docs/state/scratch/notes.jsonl"));
}

#[test]
fn test_cli_hook_pre_commit_propagates_legacy_exit_code() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let hooks_dir = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    let legacy_script = hooks_dir.join("pre-commit.legacy");
    fs::write(&legacy_script, "#!/bin/sh\nexit 42\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&legacy_script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&legacy_script, perms).unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "pre-commit"])
        .assert()
        .code(42);
}

#[test]
fn test_cli_hook_pre_push_clean_and_legacy_chaining() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Set trunk mode in qdev.toml so preflight doesn't look for integration_branch remote
    let qdev_toml = root.join("qdev.toml");
    let mut config_text = fs::read_to_string(&qdev_toml).unwrap();
    config_text.push_str("\n[git]\nbranching_mode = \"trunk\"\n");
    fs::write(&qdev_toml, config_text).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "pre-push"])
        .assert()
        .success();

    // Now add a legacy pre-push hook that exits 77
    let hooks_dir = root.join(".git").join("hooks");
    let legacy_file = hooks_dir.join("pre-push.legacy");
    fs::write(&legacy_file, "#!/bin/sh\nexit 77\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&legacy_file).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&legacy_file, perms).unwrap();
    }

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["hook", "pre-push"])
        .assert()
        .code(77);
}

#[test]
fn test_cli_doctor_reports_hooks_and_fix() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Initially without hooks installed: doctor reports status = mismatch
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["doctor", "--json"])
        .assert()
        .success();

    let payload: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let sections = payload["sections"].as_array().unwrap();
    let hooks_section = sections
        .iter()
        .find(|s| s["name"] == "hooks")
        .expect("hooks section must exist in doctor output");

    assert_eq!(hooks_section["status"], "mismatch");
    assert_eq!(hooks_section["all_installed"], false);
    assert_eq!(hooks_section["missing_hooks"].as_array().unwrap().len(), 3);

    // Run doctor --fix: rewrites shims and reports all_installed = true
    let mut fix_cmd = Command::cargo_bin("qdev").unwrap();
    let fix_assert = fix_cmd
        .current_dir(root)
        .args(["doctor", "--fix", "--json"])
        .assert()
        .success();

    let fix_payload: Value = serde_json::from_slice(&fix_assert.get_output().stdout).unwrap();
    let fix_sections = fix_payload["sections"].as_array().unwrap();
    let fix_hooks = fix_sections
        .iter()
        .find(|s| s["name"] == "hooks")
        .expect("hooks section must exist in doctor output");

    assert_eq!(fix_hooks["status"], "ok");
    assert_eq!(fix_hooks["all_installed"], true);
    assert!(fix_hooks["missing_hooks"].as_array().unwrap().is_empty());
    assert!(fix_hooks["outdated_hooks"].as_array().unwrap().is_empty());
}

#[test]
fn test_cli_hook_pre_commit_validation_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create an invalid spec that causes a validation error finding
    let specs_dir = root.join("docs").join("specs");
    fs::create_dir_all(&specs_dir).unwrap();
    let broken_spec = specs_dir.join("broken.md");
    // Invalid yaml frontmatter with duplicate / malformed field causing error
    fs::write(
        &broken_spec,
        "---\nid: E99S99\ntitle: Broken\ntype: unknown_kind\n---\n",
    )
    .unwrap();
    git(root, &["add", "docs/specs/broken.md"]);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hook", "pre-commit"])
        .assert()
        .code(1);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("Validation failed") || stderr.contains("validation_error"),
        "stderr must report validation error: {}",
        stderr
    );
}

#[test]
fn test_cli_hook_pre_push_scope_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add module configuration and require_clean_tree_in_scope
    let qdev_toml = root.join("qdev.toml");
    let mut config_text = fs::read_to_string(&qdev_toml).unwrap();
    config_text.push_str(
        r#"
[git]
branching_mode = "trunk"
require_clean_tree_in_scope = true

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]
"#,
    );
    fs::write(&qdev_toml, config_text).unwrap();

    // Create uncommitted file outside of any story lease / chore allowlist
    let outside_dir = root.join("crates").join("foundation");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("lib.rs"), "// dirty file\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hook", "pre-push"])
        .assert()
        .code(3);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(
        stderr.contains("preflight")
            && (stderr.contains("stash") || stderr.contains("out of scope")),
        "stderr must report preflight refusal: {}",
        stderr
    );
}

#[test]
fn test_cli_hook_prepare_commit_msg_default_noop_and_legacy_chaining() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // 1. Default configuration: commit_messages.enabled is false -> no-op (exit 0)
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .success();

    // 2. Chained prepare-commit-msg.legacy exists and exits with code 88
    let hooks_dir = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    let legacy_file = hooks_dir.join("prepare-commit-msg.legacy");
    fs::write(&legacy_file, "#!/bin/sh\nexit 88\n").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&legacy_file).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&legacy_file, perms).unwrap();
    }

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .code(88);
}

#[test]
fn test_cli_hook_prepare_commit_msg_drafts_selected_format_and_recent_context() {
    for (format, expected_header) in [
        ("simple", "E12S4: Lease-aware title"),
        ("conventional", "feat(E12S4): Lease-aware title"),
    ] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        setup_workspace(root);
        enable_commit_messages(root, format);
        create_story(root, "E12S4", "Lease-aware title");
        claim_story(root, "E12S4");

        let scratch_dir = root.join("docs/state/scratch");
        fs::create_dir_all(&scratch_dir).unwrap();
        fs::write(
            scratch_dir.join("E12S4.jsonl"),
            concat!(
                "{\"seq\":1,\"at\":\"2026-01-01T00:00:00Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"decision\",\"text\":\"first decision\"}\n",
                "{\"seq\":2,\"at\":\"2026-01-01T00:00:01Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"note\",\"text\":\"ignored note\"}\n",
                "{\"seq\":3,\"at\":\"2026-01-01T00:00:02Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"tradeoff\",\"text\":\"first tradeoff\"}\n",
                "{\"seq\":4,\"at\":\"2026-01-01T00:00:03Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"decision\",\"text\":\"second decision\"}\n",
                "{\"seq\":5,\"at\":\"2026-01-01T00:00:04Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"tradeoff\",\"text\":\"second tradeoff\"}\n"
            ),
        )
        .unwrap();
        let message = root.join(".git/COMMIT_EDITMSG");
        fs::write(&message, "# Git template comment\n").unwrap();

        let mut cmd = Command::cargo_bin("qdev").unwrap();
        cmd.current_dir(root)
            .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
            .assert()
            .success();

        assert_eq!(
            fs::read_to_string(message).unwrap(),
            format!(
                "{expected_header}\n\n- first tradeoff\n- second decision\n- second tradeoff\n\n# Git template comment\n"
            )
        );
    }
}

#[test]
fn test_cli_hook_prepare_commit_msg_preserves_existing_message_and_requires_context() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    enable_commit_messages(root, "conventional");
    let message = root.join(".git/COMMIT_EDITMSG");
    let supplied = "custom message\n\n# keep this comment\n";
    fs::write(&message, supplied).unwrap();

    let legacy = root.join(".git/hooks/prepare-commit-msg.legacy");
    fs::write(&legacy, "#!/bin/sh\nexit 91\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&legacy).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&legacy, permissions).unwrap();
    }

    let mut preserve = Command::cargo_bin("qdev").unwrap();
    preserve
        .current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .code(91);
    assert_eq!(fs::read_to_string(&message).unwrap(), supplied);

    fs::remove_file(&legacy).unwrap();
    fs::write(&message, "# empty template\n").unwrap();
    let mut missing_context = Command::cargo_bin("qdev").unwrap();
    missing_context
        .current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .code(1);
    assert_eq!(fs::read_to_string(&message).unwrap(), "# empty template\n");

    create_story(root, "E12S4", "Missing after lease");
    claim_story(root, "E12S4");
    fs::remove_file(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let mut missing_story = Command::cargo_bin("qdev").unwrap();
    missing_story
        .current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .code(2);
    assert_eq!(fs::read_to_string(&message).unwrap(), "# empty template\n");
}

#[test]
fn test_cli_hook_prepare_commit_msg_honors_git_comment_character_and_empty_context() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    enable_commit_messages(root, "conventional");
    create_story(root, "E12S4", "Comment character title");
    claim_story(root, "E12S4");
    git(root, &["config", "core.commentChar", ";"]);

    let message = root.join(".git/COMMIT_EDITMSG");
    fs::write(&message, "; Git template comment\n").unwrap();
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(message).unwrap(),
        "feat(E12S4): Comment character title\n\n; Git template comment\n"
    );
}

#[test]
fn test_cli_hook_prepare_commit_msg_draft_chains_legacy_hook() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    enable_commit_messages(root, "simple");
    create_story(root, "E12S4", "No scratchpad title");
    claim_story(root, "E12S4");
    let message = root.join(".git/COMMIT_EDITMSG");
    fs::write(&message, "").unwrap();

    let legacy = root.join(".git/hooks/prepare-commit-msg.legacy");
    fs::write(&legacy, "#!/bin/sh\nexit 73\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&legacy).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&legacy, permissions).unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .code(73);
    assert_eq!(
        fs::read_to_string(message).unwrap(),
        "E12S4: No scratchpad title\n"
    );
}

#[test]
fn test_cli_hook_prepare_commit_msg_uses_configured_storage_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    let config_path = root.join("qdev.toml");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str(
        "\n[storage]\nspecs_dir = \"planning\"\nstate_dir = \"workspace-state\"\n\n[commit_messages]\nenabled = true\nformat = \"conventional\"\n",
    );
    fs::write(&config_path, config).unwrap();

    let stories = root.join("planning/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(
        stories.join("E12S4.md"),
        r#"---
id: E12S4
title: "Configured storage title"
status: in-progress
version: 1
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#,
    )
    .unwrap();
    claim_story(root, "E12S4");
    let scratch = root.join("workspace-state/scratch");
    fs::create_dir_all(&scratch).unwrap();
    fs::write(
        scratch.join("E12S4.jsonl"),
        "{\"seq\":1,\"at\":\"2026-01-01T00:00:00Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"decision\",\"text\":\"configured decision\"}\n",
    )
    .unwrap();
    let message = root.join(".git/COMMIT_EDITMSG");
    fs::write(&message, "").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(message).unwrap(),
        "feat(E12S4): Configured storage title\n\n- configured decision\n"
    );
}

#[test]
fn test_cli_hook_prepare_commit_msg_rejects_multiline_story_title() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    enable_commit_messages(root, "conventional");
    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(
        stories.join("E12S4.md"),
        r#"---
id: E12S4
title: |
  First title line
  Second title line
status: in-progress
version: 1
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#,
    )
    .unwrap();
    claim_story(root, "E12S4");
    let message = root.join(".git/COMMIT_EDITMSG");
    fs::write(&message, "# template\n").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assertion = cmd
        .current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .code(1);
    assert!(String::from_utf8_lossy(&assertion.get_output().stderr)
        .contains("leased_story_title_multiline"));
    assert_eq!(fs::read_to_string(message).unwrap(), "# template\n");
}

#[test]
fn test_cli_hook_prepare_commit_msg_indents_multiline_scratchpad_entries() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    enable_commit_messages(root, "simple");
    create_story(root, "E12S4", "Multiline scratchpad title");
    claim_story(root, "E12S4");
    let scratch = root.join("docs/state/scratch");
    fs::create_dir_all(&scratch).unwrap();
    fs::write(
        scratch.join("E12S4.jsonl"),
        "{\"seq\":1,\"at\":\"2026-01-01T00:00:00Z\",\"author\":{\"type\":\"human\",\"id\":\"simon\"},\"kind\":\"decision\",\"text\":\"first line\\nsecond line\\nthird line\"}\n",
    )
    .unwrap();
    let message = root.join(".git/COMMIT_EDITMSG");
    fs::write(&message, "").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "prepare-commit-msg", ".git/COMMIT_EDITMSG"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(message).unwrap(),
        "E12S4: Multiline scratchpad title\n\n- first line\n  second line\n  third line\n"
    );
}

#[test]
fn test_cli_hook_pre_push_stdin_forwarded_to_legacy() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Set trunk mode in qdev.toml so preflight passes
    let qdev_toml = root.join("qdev.toml");
    let mut config_text = fs::read_to_string(&qdev_toml).unwrap();
    config_text.push_str("\n[git]\nbranching_mode = \"trunk\"\n");
    fs::write(&qdev_toml, config_text).unwrap();

    // Create legacy pre-push hook that captures stdin
    let hooks_dir = root.join(".git").join("hooks");
    fs::create_dir_all(&hooks_dir).unwrap();
    let legacy_file = hooks_dir.join("pre-push.legacy");
    fs::write(
        &legacy_file,
        "#!/bin/sh\ncat > stdin_captured.txt\nexit 0\n",
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&legacy_file).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&legacy_file, perms).unwrap();
    }

    let test_input = "refs/heads/main 1111 refs/heads/main 2222\n";
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hook", "pre-push", "origin", "git@github.com:test/repo.git"])
        .write_stdin(test_input)
        .assert()
        .success();

    let captured_path = root.join("stdin_captured.txt");
    assert!(captured_path.exists(), "stdin_captured.txt must exist");
    let captured = fs::read_to_string(&captured_path).unwrap();
    assert_eq!(captured, test_input);
}

#[test]
fn test_cli_hook_pre_commit_fails_on_hygiene_violation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create a new branch
    git(root, &["checkout", "-b", "feature/bad-hygiene"]);

    // Create a staged Rust file with hygiene violation (story banner)
    let dirty_file = root.join("src/dirty.rs");
    fs::create_dir_all(dirty_file.parent().unwrap()).unwrap();
    fs::write(&dirty_file, "// ⭐ STORY 2.10 - bad comment\nfn run() {}\n").unwrap();

    git(root, &["add", "src/dirty.rs"]);

    // Run `qdev hook pre-commit`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hook", "pre-commit"])
        .assert()
        .code(1);

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(
        stderr.contains("hygiene_failure") || stdout.contains("hygiene_failure"),
        "stderr/stdout must mention hygiene_failure: stdout='{}', stderr='{}'",
        stdout,
        stderr
    );
}
