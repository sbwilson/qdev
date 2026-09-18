//! `qdev next` end-to-end tests (Story 2.11): real `git init` + seeded cache fixtures
//! (pattern from `init_cli_tests.rs`) covering the happy path, nothing-eligible
//! (`next: null`, exit 0), `--sprint` closed/unknown cases, `--owner me`, and the
//! no-active-sprint case.

use std::fs;
use std::path::Path;

use assert_cmd::assert::Assert;
use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
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

fn git_init(root: &Path) {
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(root)
        .status()
        .expect("git init failed");
}

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// Seeds a story with the given status, owner strings, and `depends_on` targets.
fn write_story(root: &Path, id: &str, status: &str, owners: &[&str], deps: &[&str]) {
    let epic = id.split('S').next().unwrap_or(id);
    let mut fm = format!(
        "id: {id}\ntitle: \"{id}\"\nstatus: {status}\nversion: 1\nepic_id: {epic}\nappetite: small\n"
    );
    fm.push_str("owners:\n");
    for owner in owners {
        fm.push_str(&format!("  - \"{owner}\"\n"));
    }
    if !deps.is_empty() {
        fm.push_str(&format!(
            "relations:\n  depends_on: [{}]\n",
            deps.iter()
                .map(|d| format!("\"{d}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    fm.push_str(
        "created_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n",
    );
    write_file(
        root,
        &format!("docs/specs/stories/{id}.md"),
        &format!("---\n{fm}---\n\n## Acceptance Criteria\n- Done when complete.\n"),
    );
}

/// Seeds an epic; `phase` is written as a raw YAML scalar when given.
fn write_epic(root: &Path, id: &str, phase: Option<&str>) {
    let mut fm = format!("id: {id}\ntitle: \"{id}\"\nstatus: active\nversion: 1\n");
    if let Some(phase) = phase {
        fm.push_str(&format!("phase: {phase}\n"));
    }
    fm.push_str(
        "created_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n",
    );
    write_file(
        root,
        &format!("docs/specs/epics/{id}.md"),
        &format!("---\n{fm}---\n\n# {id}\n"),
    );
}

/// Seeds `docs/state/sprints/sprint-{n}.md` with the given status and story assignments.
/// Hydration requires the string id `sprint-{n}` and `created_by`/`updated_by`.
fn write_sprint(root: &Path, num: u32, status: &str, assignments: &[&str]) {
    let mut fm = format!(
        "id: sprint-{num}\ntitle: Sprint {num}\nstatus: {status}\nversion: 1\nrelease: 0.1.0\nstarted_at: 2026-09-10\nowners:\n  - \"team:core-platform\"\n"
    );
    if !assignments.is_empty() {
        fm.push_str("assignments:\n");
        for story in assignments {
            fm.push_str(&format!(
                "  - story: {story}\n    assigned_at: 2026-09-10\n"
            ));
        }
    }
    fm.push_str(
        "created_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n",
    );
    write_file(
        root,
        &format!("docs/state/sprints/sprint-{num}.md"),
        &format!("---\n{fm}---\n\n# Sprint {num}\n"),
    );
}

/// Writes a lease record into the shared Git directory so `get_lease` finds it from
/// this worktree while `worktree_path` names another one.
fn write_foreign_lease(root: &Path, story: &str, holder: &str, worktree: &str) {
    let dir = root.join(".git/qdev/leases");
    fs::create_dir_all(&dir).unwrap();
    let lease = json!({
        "story_id": story,
        "holder": holder,
        "author_type": "agent",
        "worktree_path": worktree,
        "branch": "feature/x",
        "started_at": "2026-09-17T00:00:00Z",
        "session_token": "qs_fixture",
    });
    fs::write(dir.join(format!("{story}.json")), lease.to_string()).unwrap();
}

fn sync_rebuild(root: &Path) {
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sync", "--rebuild"])
        .assert()
        .success();
}

fn run_json(root: &Path, args: &[&str]) -> (Assert, Value) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root);
    let assert = cmd.args(args).assert();
    let stdout = assert.get_output().stdout.clone();
    let value: Value = serde_json::from_slice(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be JSON ({e}):\n{}",
            String::from_utf8_lossy(&stdout)
        )
    });
    (assert, value)
}

fn snapshot_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.is_file() {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
}

/// Full happy fixture: sprint 5 active; `E12S2` blocked (dep `E12S1` not done),
/// `E12S3` leased by bot-9 in another worktree, `E12S4` ready/unblocked/unleased;
/// `E12S1` sits in no sprint assignment.
fn seed_happy_fixture(root: &Path) {
    write_epic(root, "E12", None);
    write_story(root, "E12S1", "ready", &["simon"], &[]);
    write_story(root, "E12S2", "ready", &["simon"], &["E12S1"]);
    write_story(root, "E12S3", "ready", &["simon"], &[]);
    write_story(root, "E12S4", "ready", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S2", "E12S3", "E12S4"]);
    write_foreign_lease(root, "E12S3", "bot-9", "/wt/x");
    sync_rebuild(root);
}

// ---------------------------------------------------------------------------

#[test]
fn test_next_happy_path_spec_matrix() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    git_init(root);
    seed_happy_fixture(root);

    let (assert, value) = run_json(root, &["next", "--json"]);
    assert.success().code(0);
    assert_eq!(value["schema_version"], "1");
    let next = value
        .get("next")
        .unwrap_or_else(|| panic!("next key must be present"));
    assert!(!next.is_null(), "E12S4 should be selected");
    assert_eq!(next["id"], "E12S4");
    assert_eq!(next["kind"], "story");
    assert_eq!(next["blocked"], false);
    assert_eq!(next["seq"], 4);
    assert_eq!(next["sprint"], 5);
    assert_eq!(next["sprint_status"], "active");
    assert!(value["reason"]["summary"]
        .as_str()
        .unwrap()
        .contains("ready and unblocked"));

    let blockers = value["blockers"].as_array().unwrap();
    assert_eq!(
        blockers.len(),
        2,
        "E12S2 and E12S3 under blockers: {blockers:?}"
    );
    let blocked = &blockers[0];
    assert_eq!(blocked["story_id"], "E12S2");
    assert_eq!(blocked["kind"], "blocked");
    assert_eq!(blocked["detail"], "waiting on E12S1");
    assert_eq!(blocked["blocking_ids"], json!(["E12S1"]));
    let leased = &blockers[1];
    assert_eq!(leased["story_id"], "E12S3");
    assert_eq!(leased["kind"], "leased");
    assert_eq!(leased["detail"], "held by bot-9 in /wt/x");
    assert_eq!(leased["holder"], "bot-9");
    assert_eq!(leased["worktree_path"], "/wt/x");
    // E12S1 is in no sprint assignment — never a candidate, never named.
    assert!(!blockers
        .iter()
        .any(|b| b.get("story_id").and_then(|s| s.as_str()) == Some("E12S1")));

    // Text mode: human summary with selected story, reason lines, blockers.
    let text = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["next"])
        .assert()
        .success()
        .code(0);
    let stdout = String::from_utf8_lossy(&text.get_output().stdout).to_string();
    assert!(stdout.contains("E12S4"), "{stdout}");
    assert!(stdout.contains("ready and unblocked"), "{stdout}");
    assert!(stdout.contains("Blockers"), "{stdout}");
    assert!(
        stdout.contains("E12S2 (blocked): waiting on E12S1"),
        "{stdout}"
    );
    assert!(
        stdout.contains("E12S3 (leased): held by bot-9 in /wt/x"),
        "{stdout}"
    );
}

#[test]
fn test_next_nothing_eligible_all_blocked_or_leased() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    git_init(root);
    write_epic(root, "E12", None);
    write_story(root, "E12S1", "ready", &["simon"], &[]);
    write_story(root, "E12S2", "ready", &["simon"], &["E12S1"]);
    write_story(root, "E12S3", "ready", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S2", "E12S3"]);
    write_foreign_lease(root, "E12S3", "bot-9", "/wt/x");
    sync_rebuild(root);

    let (assert, value) = run_json(root, &["next", "--json"]);
    assert.success().code(0);
    let next = value.get("next").unwrap();
    assert!(
        next.is_null(),
        "all candidates blocked or leased → next: null"
    );
    assert_eq!(value["reason"]["code"], "all_filtered");
    let blockers = value["blockers"].as_array().unwrap();
    assert!(!blockers.is_empty(), "blockers name each nearest cause");
    assert_eq!(blockers[0]["kind"], "blocked");
    assert_eq!(blockers[1]["kind"], "leased");
}

#[test]
fn test_next_no_active_sprint() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E12", None);
    write_story(root, "E12S2", "ready", &["simon"], &[]);
    write_sprint(root, 4, "completed", &["E12S2"]);
    sync_rebuild(root);

    // No sprint has `status: active`; no `--sprint` → exit 0, `next: null`,
    // reason "no active sprints".
    let (assert, value) = run_json(root, &["next", "--json"]);
    assert.success().code(0);
    assert!(value.get("next").unwrap().is_null());
    assert_eq!(value["reason"]["code"], "no_active_sprints");
    assert!(value["reason"]["summary"]
        .as_str()
        .unwrap()
        .contains("No active sprints"));
    assert_eq!(value["blockers"][0]["kind"], "no_active_sprints");

    // Explicit closed sprint: selection runs over sprint 4's assignments anyway.
    let (assert, value) = run_json(root, &["next", "--sprint", "4", "--json"]);
    assert.success().code(0);
    assert_eq!(value["next"]["id"], "E12S2");
    assert_eq!(value["next"]["sprint_status"], "completed");

    // Unknown sprint id → exit 2 `sprint_not_found`.
    let (assert, value) = run_json(root, &["next", "--sprint", "99", "--json"]);
    assert.failure().code(2);
    assert_eq!(value["error"]["code"], "sprint_not_found");
}

#[test]
fn test_next_owner_me_filters_team_owned_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Identity `simon` in `core-platform`; only story owned by `team:frontend`.
    let mut toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    if !toml.contains("[teams]") {
        toml.push_str("\n[teams]\n");
    }
    toml.push_str("frontend = [\"sally\"]\n");
    fs::write(root.join("qdev.toml"), toml).unwrap();

    write_epic(root, "E12", None);
    write_story(root, "E12S4", "ready", &["team:frontend"], &[]);
    write_sprint(root, 5, "active", &["E12S4"]);
    sync_rebuild(root);

    let (assert, value) = run_json(root, &["next", "--owner", "me", "--json"]);
    assert.success().code(0);
    assert!(value.get("next").unwrap().is_null());
    let blockers = value["blockers"].as_array().unwrap();
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0]["kind"], "owner");
    assert!(blockers[0]["detail"]
        .as_str()
        .unwrap()
        .contains("team:frontend"));

    // A literal owner that matches selects the story.
    let (assert, value) = run_json(root, &["next", "--owner", "team:frontend", "--json"]);
    assert.success().code(0);
    assert_eq!(value["next"]["id"], "E12S4");
}

#[test]
fn test_next_own_lease_continuation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    git_init(root);
    write_epic(root, "E12", None);
    write_story(root, "E12S4", "ready", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S4"]);
    sync_rebuild(root);

    // Real claim from this worktree as the configured identity.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4", "--json"])
        .assert()
        .success();

    // `E12S4` leased by this worktree → returned (continuation).
    let (assert, value) = run_json(root, &["next", "--json"]);
    assert.success().code(0);
    assert_eq!(value["next"]["id"], "E12S4");
    assert_eq!(value["next"]["lease"]["holder"], "simon");
    assert!(value["reason"]["summary"]
        .as_str()
        .unwrap()
        .contains("leased by you here"));
    let text = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["next"])
        .assert()
        .success();
    assert!(String::from_utf8_lossy(&text.get_output().stdout).contains("leased by you here"));
}

#[test]
fn test_next_is_deterministic_and_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    git_init(root);
    seed_happy_fixture(root);

    let before = snapshot_tree(root);
    let (first, first_value) = run_json(root, &["next", "--json"]);
    let (second, second_value) = run_json(root, &["next", "--json"]);
    let first_stdout = first.get_output().stdout.clone();
    let second_stdout = second.get_output().stdout.clone();
    first.success().code(0);
    second.success().code(0);
    // Byte-identical `--json` output across repeated runs over identical state.
    assert_eq!(
        first_stdout, second_stdout,
        "repeat runs must produce identical output"
    );
    assert_eq!(first_value, second_value);

    let after = snapshot_tree(root);
    assert_eq!(
        before.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        after.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        "file list changed — nothing may be created (no lease, cache row, or state file)"
    );
    for ((path, content), (after_path, after_content)) in before.iter().zip(&after) {
        assert_eq!(path, after_path);
        assert_eq!(
            content, after_content,
            "file {path} changed — qdev next must not mutate anything"
        );
    }
    // And it never claimed anything: no lease exists for the selected story.
    assert!(after
        .iter()
        .all(|(path, _)| path != "qdev/leases/E12S4.json" && !path.ends_with("leases/E12S4.json")));
}
