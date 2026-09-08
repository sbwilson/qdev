use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
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

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn write_story(
    root: &Path,
    id: &str,
    epic: &str,
    title: &str,
    status: &str,
    owners: &str,
    modules: &str,
) {
    write_file(
        root,
        &format!("docs/specs/stories/{}.md", id),
        &format!(
            r#"---
id: {id}
title: "{title}"
status: {status}
version: 1
epic_id: {epic}
owners: {owners}
target_modules: {modules}
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#,
            id = id,
            title = title,
            status = status,
            epic = epic,
            owners = owners,
            modules = modules,
        ),
    );
}

fn setup_list_fixture(root: &Path) {
    write_story(
        root,
        "E12S4",
        "E12",
        "Buffer Layout",
        "ready",
        r#"["simon"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E12S9",
        "E12",
        "Later Story",
        "ready",
        r#"["simon"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E12S5",
        "E12",
        "Different Status",
        "draft",
        r#"["simon"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E13S1",
        "E13",
        "Different Epic",
        "ready",
        r#"["simon"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E12S6",
        "E12",
        "Different Owner",
        "ready",
        r#"["sally"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E12S7",
        "E12",
        "Different Module",
        "ready",
        r#"["simon"]"#,
        r#"["foundation"]"#,
    );
}

#[test]
fn test_list_all_filters_apply_as_and_ordered_by_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "list", "stories", "--epic", "E12", "--status", "ready", "--owner", "simon",
            "--module", "bridge", "--json",
        ])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["schema_version"], "1");
    let items = val["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    // Ordered by id ascending; E12S5 (draft), E12S6 (sally), E12S7 (foundation), E13S1
    // (different epic) are all excluded by one filter each.
    assert_eq!(ids, vec!["E12S4", "E12S9"]);
}

#[test]
fn test_list_ac_literal_command_all_five_filters_and_text_table() {
    // Matches the spec's acceptance criterion literally:
    // `qdev list stories --epic E12 --status ready --owner me --module bridge --sprint 5`
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);

    let mut assign_cmd = Command::cargo_bin("qdev").unwrap();
    // No `sprint assign` CLI command exists yet (later story), so assign directly via the
    // cache after the first boot sweep has hydrated the fixture stories.
    assign_cmd
        .current_dir(root)
        .args(["status", "--json"])
        .assert()
        .success();
    let db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = qdev_core::rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("INSERT INTO sprints (id, status) VALUES (5, 'active');", [])
        .unwrap();
    conn.execute(
        "INSERT INTO sprint_assignments (sprint_id, story_id, assigned_at) VALUES (5, 'E12S4', '2026-09-08T00:00:00Z');",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sprint_assignments (sprint_id, story_id, assigned_at) VALUES (5, 'E12S9', '2026-09-08T00:00:00Z');",
        [],
    )
    .unwrap();
    drop(conn);

    let mut json_cmd = Command::cargo_bin("qdev").unwrap();
    let json_assert = json_cmd
        .current_dir(root)
        .args([
            "list", "stories", "--epic", "E12", "--status", "ready", "--owner", "me", "--module",
            "bridge", "--sprint", "5", "--json",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&json_assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        vec!["E12S4", "E12S9"],
        "every filter applies as AND, ordered by id"
    );

    let mut text_cmd = Command::cargo_bin("qdev").unwrap();
    let text_assert = text_cmd
        .current_dir(root)
        .args([
            "list", "stories", "--epic", "E12", "--status", "ready", "--owner", "me", "--module",
            "bridge", "--sprint", "5",
        ])
        .assert()
        .success()
        .code(0);
    let stdout = String::from_utf8(text_assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("ID"));
    assert!(stdout.contains("TITLE"));
    assert!(stdout.contains("STATUS"));
    assert!(stdout.contains("OWNERS"));
    assert!(stdout.contains("E12S4"));
    assert!(stdout.contains("E12S9"));
}

#[test]
fn test_list_owner_me_resolves_active_identity() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--owner", "me", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    // "simon" is the configured developer identity; "sally"-owned E12S6 must be excluded.
    assert!(ids.contains(&"E12S4"));
    assert!(!ids.contains(&"E12S6"));
}

#[test]
fn test_list_unknown_kind_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "widgets", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

/// Blanks out `.qdev.local.toml`'s identity and neuters every git config source (global,
/// system, and any repo-local config) via env overrides, so `resolve_git_email` deterministically
/// returns `None` regardless of the host machine's ambient git configuration.
fn blank_identity_and_git_config(root: &Path) {
    fs::write(
        root.join(".qdev.local.toml"),
        "[preferences]\ncolor = true\n",
    )
    .unwrap();
}

fn no_git_identity_envs(cmd: &mut Command, root: &Path) {
    let empty_global = root.join("empty.gitconfig");
    fs::write(&empty_global, "").unwrap();
    cmd.env("GIT_CONFIG_GLOBAL", &empty_global)
        .env("GIT_CONFIG_SYSTEM", &empty_global)
        .env("HOME", root)
        .env_remove("GIT_AUTHOR_EMAIL")
        .env_remove("GIT_COMMITTER_EMAIL");
}

#[test]
fn test_list_owner_me_without_identity_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);
    blank_identity_and_git_config(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    no_git_identity_envs(&mut cmd, root);
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--owner", "me", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_list_owner_me_falls_back_to_resolved_git_email() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(
        root,
        "E20S1",
        "E20",
        "Amelia's Story",
        "ready",
        r#"["amelia@example.com"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E20S2",
        "E20",
        "Simon's Story",
        "ready",
        r#"["simon"]"#,
        r#"["bridge"]"#,
    );
    blank_identity_and_git_config(root);

    // A repo-local git config takes precedence over the (neutered) global/system config, so
    // this deterministically resolves regardless of the host's ambient git identity.
    let git_init = std::process::Command::new("git")
        .arg("init")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(git_init.status.success());
    let email_cfg = std::process::Command::new("git")
        .args(["config", "user.email", "amelia@example.com"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(email_cfg.status.success());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    no_git_identity_envs(&mut cmd, root);
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--owner", "me", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|i| i["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["E20S1"]);
}

#[test]
fn test_list_text_mode_compact_table() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--epic", "E12", "--status", "ready"])
        .assert()
        .success()
        .code(0);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("ID"));
    assert!(stdout.contains("TITLE"));
    assert!(stdout.contains("STATUS"));
    assert!(stdout.contains("OWNERS"));
    assert!(stdout.contains("E12S4"));
    assert!(stdout.contains("E12S9"));
    // E12S5 is draft, so it must not appear in the filtered table.
    assert!(!stdout.contains("E12S5"));
}

#[test]
fn test_list_no_filters_returns_all_of_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    assert_eq!(items.len(), 6);
}

#[test]
fn test_list_outside_workspace_is_clean_usage_error_and_creates_no_cache() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // Deliberately skip setup_workspace: no `qdev init` has run here.

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--json"])
        .assert()
        .failure()
        .code(2);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(
        !root.join(".qdev/cache/cache.sqlite").exists(),
        "qdev list must not create a cache file outside an initialized workspace"
    );
}

#[test]
fn test_list_owner_filter_exact_match_not_fooled_by_wildcards_or_case() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    // "j_smith@corp.com" contains a literal SQL LIKE wildcard character ('_'); "Sally" differs
    // only in case from the stored "sally". Neither must produce a false match.
    write_story(
        root,
        "E16S1",
        "E16",
        "Underscore Owner",
        "ready",
        r#"["j_smith@corp.com"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E16S2",
        "E16",
        "Wildcard-Adjacent Owner",
        "ready",
        r#"["jXsmith@corp.com"]"#,
        r#"["bridge"]"#,
    );
    write_story(
        root,
        "E16S3",
        "E16",
        "Lowercase Owner",
        "ready",
        r#"["sally"]"#,
        r#"["bridge"]"#,
    );

    let mut underscore_cmd = Command::cargo_bin("qdev").unwrap();
    let underscore_assert = underscore_cmd
        .current_dir(root)
        .args(["list", "stories", "--owner", "j_smith@corp.com", "--json"])
        .assert()
        .success()
        .code(0);
    let underscore_val: Value =
        serde_json::from_slice(&underscore_assert.get_output().stdout).unwrap();
    let underscore_ids: Vec<&str> = underscore_val["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        underscore_ids,
        vec!["E16S1"],
        "'_' in the filter value must match literally, not as a SQL LIKE single-char wildcard"
    );

    let mut case_cmd = Command::cargo_bin("qdev").unwrap();
    let case_assert = case_cmd
        .current_dir(root)
        .args(["list", "stories", "--owner", "Sally", "--json"])
        .assert()
        .success()
        .code(0);
    let case_val: Value = serde_json::from_slice(&case_assert.get_output().stdout).unwrap();
    let case_items = case_val["items"].as_array().unwrap();
    assert!(
        case_items.is_empty(),
        "'Sally' must not match the differently-cased stored owner 'sally'"
    );
}

#[test]
fn test_list_stale_row_surfaces_stale_true() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    setup_list_fixture(root);

    // Trigger an initial hydration so the cache actually has rows before the raw SQL UPDATE
    // below (otherwise it would be a silent no-op on a nonexistent row).
    let mut hydrate_cmd = Command::cargo_bin("qdev").unwrap();
    hydrate_cmd
        .current_dir(root)
        .args(["status", "--json"])
        .assert()
        .success();

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = qdev_core::rusqlite::Connection::open(&db_path).unwrap();
    conn.execute("UPDATE entities SET stale = 1 WHERE id = 'E12S4';", [])
        .unwrap();
    drop(conn);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories", "--json"])
        .assert()
        .success()
        .code(0);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let items = val["items"].as_array().unwrap();
    let e12s4 = items
        .iter()
        .find(|i| i["id"] == "E12S4")
        .expect("E12S4 present");
    assert_eq!(e12s4["stale"], true);
    let e12s9 = items
        .iter()
        .find(|i| i["id"] == "E12S9")
        .expect("E12S9 present");
    assert_eq!(e12s9["stale"], false);
}

#[test]
fn test_list_text_mode_does_not_truncate_long_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    // "E1234S5678" (10 chars) fits under the 12-char id column width, but "E12345S6789" (11
    // chars) plus the story-id grammar's growth potential motivates never truncating id at all;
    // use a value that would have been ellipsis-truncated under the old fixed ID_WIDTH.
    write_story(
        root,
        "E12345S6789",
        "E12345",
        "Long Id Story",
        "ready",
        r#"["simon"]"#,
        r#"["bridge"]"#,
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["list", "stories"])
        .assert()
        .success()
        .code(0);

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains("E12345S6789"),
        "the full id must appear, not an ellipsis-truncated prefix: {}",
        stdout
    );
    assert!(!stdout.contains('…'));
}
