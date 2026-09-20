//! `qdev` / `qdev status` end-to-end tests (Story 2.12): real `git init` + seeded
//! cache fixtures (pattern from `next_cli_tests.rs`) covering the §5 text layout,
//! the `--json` envelope with its `payload-pulse.json` round-trip, `next` parity
//! with `qdev next`, the outside-workspace hint (no cache file created), the
//! non-git workspace, the own-worktree lease line, the gate-evidence case, the
//! "cache missing / mismatched is reported, not repaired" cases, and the
//! performance-AC form (no network git args; wall-clock bound).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Instant;

use assert_cmd::Command;
use serde_json::{json, Value};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture plumbing (mirrors `next_cli_tests.rs`)
// ---------------------------------------------------------------------------

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

fn git(root: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .expect("git must be available");
    assert!(status.success(), "git {args:?} failed");
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

fn write_epic(root: &Path, id: &str) {
    write_file(
        root,
        &format!("docs/specs/epics/{id}.md"),
        &format!(
            "---\nid: {id}\ntitle: \"{id}\"\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\n# {id}\n"
        ),
    );
}

/// Seeds `docs/state/sprints/sprint-{n}.md` with the given status and story assignments.
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

/// Seeds a deferred-work entity with the given status and safety risk.
fn write_dw(root: &Path, id: &str, status: &str, risk: &str, origin: &str) {
    let mut fm = format!(
        "id: {id}\ntitle: \"{id} debt\"\nstatus: {status}\nversion: 1\norigin_story_id: {origin}\ntarget_module: bridge\nsafety_risk: {risk}\n"
    );
    if risk != "negligible" {
        fm.push_str("rationale: \"recorded in the fixture\"\n");
    }
    fm.push_str(
        "created_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n",
    );
    write_file(
        root,
        &format!("docs/state/dw/{id}.md"),
        &format!("---\n{fm}---\n\n# {id}\n"),
    );
}

/// Writes a lease record into the shared Git directory so the pulse's Environment
/// section sees this story as leased in another worktree.
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

/// Runs `qdev` with the given args and parses stdout as JSON.
fn run_json(root: &Path, args: &[&str]) -> (assert_cmd::assert::Assert, Value) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root);
    cmd.args(args);
    let assert = cmd.assert();
    let stdout = assert.get_output().stdout.clone();
    let value: Value = serde_json::from_slice(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be JSON ({e}):\n{}",
            String::from_utf8_lossy(&stdout)
        )
    });
    (assert, value)
}

fn run_text(root: &Path, args: &[&str]) -> String {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root);
    cmd.args(args);
    let assert = cmd.assert().success().code(0);
    String::from_utf8_lossy(&assert.get_output().stdout).to_string()
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

/// The full Story-2.12 matrix fixture: git repo on `main` (committed, so the
/// working tree is clean) with `develop` pushed to a local-path `origin` (so
/// the integration line renders `up_to_date`); active sprint 5 covering all
/// four buckets — `E12S1` done, `E12S2` blocked (dep `E12S5` not done),
/// `E12S3` leased by bot-9 in another worktree, `E12S4` to be claimed by
/// simon in this worktree, `E12S5`/`E12S6` backlog (`E12S6` is in-progress,
/// its dep `E12S1` done); open DW including one `unacceptable`; no gate runs.
/// Story counters: 1 done / 1 in progress / 1 blocked / 3 backlog (sum 6,
/// matching the six assignments to sprint 5).
fn seed_full_fixture(root: &Path) {
    write_epic(root, "E12");
    write_story(root, "E12S1", "done", &["simon"], &[]);
    write_story(root, "E12S2", "ready", &["simon"], &["E12S5"]); // blocked
    write_story(root, "E12S3", "ready", &["simon"], &[]); // foreign lease → backlog
    write_story(root, "E12S4", "ready", &["simon"], &[]); // own claim → backlog + selected
    write_story(root, "E12S5", "ready", &["simon"], &[]); // backlog (E12S2's blocker)
    write_story(root, "E12S6", "in-progress", &["simon"], &["E12S1"]); // in-progress, unblocked
    write_sprint(
        root,
        5,
        "active",
        &["E12S1", "E12S2", "E12S3", "E12S4", "E12S5", "E12S6"],
    );
    write_foreign_lease(root, "E12S3", "bot-9", "/wt/x");
    write_dw(root, "DW-1111", "open", "negligible", "E12S4");
    write_dw(root, "DW-2222", "open", "unacceptable", "E12S4");
    sync_rebuild(root);
    // Keep local artifacts out of `git status`: the local origin repo (and the
    // probe-shim dir the perf test writes) would otherwise keep the tree dirty.
    // `git status --porcelain` never lists ignored paths, so the working tree
    // stays `clean` for the §5 layout.
    let exclude = root.join(".git/info/exclude");
    let mut excludes = fs::read_to_string(&exclude).unwrap_or_default();
    for pattern in ["origin.git/", "shim/"] {
        if !excludes.contains(pattern) {
            excludes.push_str(&format!("{pattern}\n"));
        }
    }
    fs::write(&exclude, excludes).unwrap();
    // Commit everything so the working tree is clean, then stand up `origin`
    // (a local-path repository — never a network remote) with `develop`
    // identical to the local one.
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
    git(root, &["branch", "develop"]);
    git(
        root,
        &["init", "--bare", &root.join("origin.git").to_string_lossy()],
    );
    git(
        root,
        &[
            "remote",
            "add",
            "origin",
            &root.join("origin.git").to_string_lossy(),
        ],
    );
    git(root, &["push", "origin", "develop"]);
}

/// `setup_workspace` + `git init -b main` + user config, ready to seed and commit.
fn setup_git_workspace(root: &Path) {
    setup_workspace(root);
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

// ---------------------------------------------------------------------------
// §5 layout: the full matrix fixture, rendered as text
// ---------------------------------------------------------------------------

#[test]
fn test_full_fixture_text_matches_section5_layout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_git_workspace(root);
    seed_full_fixture(root);

    // Own-worktree lease: claim `E12S4` as the configured identity (fixture
    // setup — the pulse itself must not claim or release anything).
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4", "--json"])
        .assert()
        .success();

    let stdout = run_text(root, &[]);
    let joined = stdout.replace('\r', "");

    // Header carries the real version, not the docs example's 1.0.
    assert!(
        joined.starts_with("qdev 0.1.0 — Development Engine & Gatekeeper\n"),
        "header must show the real version:\n{joined}"
    );
    assert!(!joined.contains("qdev 1.0"), "{joined}");

    // Environment — clean tree on `main`, integration up to date with
    // origin/develop, cache healthy, this-worktree lease named.
    let head_sha = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(root)
        .output()
        .unwrap()
        .stdout;
    let sha = String::from_utf8(head_sha).unwrap().trim().to_string();
    assert!(
        joined.contains(&format!("  Working tree   clean (main @ {sha})")),
        "clean tree line:\n{joined}"
    );
    assert!(
        joined.contains("  Integration    develop is up to date with origin/develop"),
        "integration line:\n{joined}"
    );
    assert!(
        joined.contains("  Cache          healthy (synced ")
            && joined.contains(" ms ago, 10 entities, 0 findings)"),
        "cache line:\n{joined}"
    );
    assert!(
        joined.contains("  Lease          E12S4 held by simon since ")
            && joined.contains("(this worktree)"),
        "lease line:\n{joined}"
    );

    // Sprint block — four fixed buckets summing to the assigned count, plus
    // the workspace-wide DW line. No Gates line: no `gate_runs` evidence.
    assert!(
        joined.contains("Sprint 5 — Sprint 5  [release 0.1.0]"),
        "{joined}"
    );
    assert!(
        joined.contains("  Stories        1 done / 1 in progress / 1 blocked / 3 backlog"),
        "story counters:\n{joined}"
    );
    assert!(
        joined.contains("  Deferred work  2 open (1 unacceptable)"),
        "DW line:\n{joined}"
    );
    assert!(
        !joined.contains("Gates"),
        "no gate evidence → no Gates line:\n{joined}"
    );

    // Next — selection with both invocations, and the 2.11 continuation note
    // because this worktree holds the lease on `E12S4`.
    assert!(
        joined.contains(
            "E12S4 \"E12S4\" is ready — leased by you here (simon); finish what you started"
        ),
        "continuation summary:\n{joined}"
    );
    assert!(
        joined.contains("  Run: /qdev-develop E12S4   (or: qdev context E12S4 --phase develop)"),
        "both invocations:\n{joined}"
    );
    // `status` remains a working alias for the same screen — modulo the
    // wall-clock-derived `synced_ms_ago` on the Cache line.
    let alias = run_text(root, &["status"]);
    assert_eq!(
        normalize_sync_age(&alias),
        normalize_sync_age(&joined),
        "`qdev status` must render the same pulse"
    );
}

/// Replace `synced <digits> ms ago` with `synced <N> ms ago` so two runs at
/// different instants compare equal (the only run-dependent field).
fn normalize_sync_age(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("synced ") {
        out.push_str(&rest[..i + "synced ".len()]);
        rest = &rest[i + "synced ".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && rest[digits.len()..].starts_with(" ms ago") {
            out.push_str("<N>");
            rest = &rest[digits.len()..];
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// --json: envelope shape, `next` parity with `qdev next`, schema round-trip
// ---------------------------------------------------------------------------

#[test]
fn test_json_envelope_and_next_parity_and_schema_round_trip() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_git_workspace(root);
    seed_full_fixture(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["claim", "story", "E12S4", "--json"])
        .assert()
        .success();

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(value["schema_version"], "1");
    assert_eq!(value["workspace"], true);
    assert_eq!(value["environment"]["working_tree"]["clean"], true);
    assert_eq!(value["environment"]["working_tree"]["branch"], "main");
    assert_eq!(value["environment"]["working_tree"]["dirty_files"], 0);
    assert_eq!(
        value["environment"]["integration"],
        json!({
            "remote": "origin", "branch": "develop", "state": "up_to_date",
            "ahead": 0, "behind": 0,
        })
    );
    assert_eq!(value["environment"]["cache"]["schema_status"], "ok");
    let leases = value["environment"]["lease"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["story_id"], "E12S4");
    assert_eq!(leases[0]["holder"], "simon");
    assert_eq!(
        value["sprints"],
        json!([{
            "id": 5, "title": "Sprint 5", "release": "0.1.0", "status": "active",
            "stories": {"done": 1, "in_progress": 1, "blocked": 1, "backlog": 3},
            "deferred_work": {"open": 2, "unacceptable": 1},
        }])
    );
    // No gate_runs rows → `gates: null`.
    assert!(value["gates"].is_null());

    // `next` equals `qdev next --json`'s selection for the same input.
    let (next_assert, next_value) = run_json(root, &["next", "--json"]);
    next_assert.success().code(0);
    let mut next_only = next_value.clone();
    next_only.as_object_mut().unwrap().remove("schema_version");
    assert_eq!(
        value["next"], next_only,
        "pulse `next` must match `qdev next --json` verbatim"
    );

    // Round-trip: the envelope validates against `payload-pulse.json`.
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "pulse", "--json"])
        .assert()
        .success();
    let mut schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    schema.as_object_mut().unwrap().remove("schema_version");
    // The envelope flattens `schema_version` + payload fields at the top level,
    // which is exactly the shape `payload-pulse.json` describes.
    validate_against_schema(&schema, &value);
}

fn validate_against_schema(schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("schema must compile");
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "instance failed schema validation: {errors:?}\ninstance: {instance}"
    );
}

// ---------------------------------------------------------------------------
// No active sprints
// ---------------------------------------------------------------------------

#[test]
fn test_no_active_sprints_reports_none_and_next_is_null() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E12");
    write_story(root, "E12S2", "ready", &["simon"], &[]);
    write_sprint(root, 4, "completed", &["E12S2"]);
    sync_rebuild(root);

    let text = run_text(root, &[]);
    assert!(
        text.contains("Sprints\n  No active sprints — nothing to report"),
        "{text}"
    );
    assert!(
        text.contains("Nothing eligible: No active sprints"),
        "{text}"
    );

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    // Empty array (not null) inside a workspace with no active sprints.
    assert_eq!(value["sprints"].as_array().unwrap().len(), 0);
    let next = &value["next"];
    assert!(next["next"].is_null());
    assert_eq!(next["reason"]["code"], "no_active_sprints");
    assert_eq!(next["blockers"][0]["kind"], "no_active_sprints");
}

// ---------------------------------------------------------------------------
// Outside a workspace
// ---------------------------------------------------------------------------

#[test]
fn test_outside_workspace_is_the_one_line_hint_with_all_nulls() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let text = run_text(root, &[]);
    // `emit_text` appends its own newline after the renderer's trailing \n,
    // so the exact hint is the trimmed output.
    assert_eq!(
        text.trim_end(),
        "not a qdev workspace — run `qdev init` first",
        "D-1 pins the exact hint"
    );

    // Nothing was created — the store is never opened outside a workspace.
    assert_eq!(fs::read_dir(root).unwrap().count(), 0, "{root:?}");

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value,
        json!({
            "schema_version": "1",
            "workspace": false,
            "environment": null,
            "sprints": null,
            "gates": null,
            "next": null,
        })
    );

    // And the payload round-trips in the all-null branch too.
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "pulse", "--json"])
        .assert()
        .success();
    let mut schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    schema.as_object_mut().unwrap().remove("schema_version");
    validate_against_schema(&schema, &value);
    assert_eq!(fs::read_dir(root).unwrap().count(), 0, "{root:?}");
}

// ---------------------------------------------------------------------------
// Non-git workspace
// ---------------------------------------------------------------------------

#[test]
fn test_non_git_workspace_degrades_git_fields_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root); // deliberately no `git init`
    write_epic(root, "E12");
    write_story(root, "E12S1", "ready", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S1"]);
    sync_rebuild(root);

    let text = run_text(root, &[]);
    assert!(
        text.contains("  Working tree   not a git repository"),
        "{text}"
    );
    assert!(
        text.contains("  Integration    not available — no git repository to compare against"),
        "{text}"
    );
    // Cache/sprint/next sections render normally; exit 0 (asserted by run_text).

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert!(value["environment"]["working_tree"].is_null());
    assert!(value["environment"]["integration"].is_null());
    assert_eq!(value["environment"]["cache"]["schema_status"], "ok");
    assert!(
        value["environment"]["cache"]["entity_count"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(value["sprints"].as_array().unwrap().len(), 1);
    assert_eq!(value["next"]["next"]["id"], "E12S1");
}

// ---------------------------------------------------------------------------
// Gate evidence
// ---------------------------------------------------------------------------

#[test]
fn test_gates_line_appears_only_with_gate_run_rows() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_git_workspace(root);
    seed_full_fixture(root);

    // Before: no `gate_runs` rows — line omitted (text), null (JSON), and
    // everything else still renders.
    let text = run_text(root, &[]);
    assert!(!text.contains("Gates"), "{text}");
    let (_, value) = run_json(root, &["--json"]);
    assert!(value["gates"].is_null());

    // Seed gate-run evidence directly into the cache (fixture plumbing; the
    // pulse itself never writes). `fmt`: early pass, later fail; `lint`: pass.
    {
        let store = qdev_core::SqliteStore::open(root.join(".qdev/cache/cache.sqlite"))
            .expect("cache opens");
        use qdev_core::{GateRunRecord, Store};
        let mk = |id: &str, gate: &str, status: &str, at: &str| GateRunRecord {
            id: id.to_string(),
            story_id: Some("E12S4".to_string()),
            gate_id: gate.to_string(),
            commit_sha: "8f1b2c4".to_string(),
            status: Some(status.to_string()),
            exit_code: Some(if status == "pass" { 0 } else { 1 }),
            duration_ms: Some(500),
            metric_value: None,
            summary: Some(format!("{gate} {status}")),
            evidence_path: format!("docs/state/evidence/{id}.json"),
            output_hash: None,
            run_by_type: Some("human".to_string()),
            run_by_id: Some("simon".to_string()),
            ran_at: Some(at.to_string()),
        };
        store
            .upsert_gate_run(&mk("gr-early", "fmt", "pass", "2026-09-17T00:00:00Z"))
            .unwrap();
        store
            .upsert_gate_run(&mk("gr-late", "fmt", "fail", "2026-09-18T00:00:00Z"))
            .unwrap();
        store
            .upsert_gate_run(&mk("gr-lint", "lint", "pass", "2026-09-18T12:00:00Z"))
            .unwrap();
    }

    let text = run_text(root, &[]);
    assert!(
        text.contains("  Gates          1/2 passing"),
        "Gates line from most recent run per gate:\n{text}"
    );
    // Rendered between the DW line and the Next section.
    let dw_at = text.find("  Deferred work").unwrap();
    let gates_at = text.find("  Gates          1/2 passing").unwrap();
    let next_at = text.find("\nNext\n").unwrap();
    assert!(
        dw_at < gates_at && gates_at < next_at,
        "line order:\n{text}"
    );

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(value["gates"], json!({"passing": 1, "total": 2}));

    // And the round-trip covers the `gates` object being present.
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "pulse", "--json"])
        .assert()
        .success();
    let mut schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    schema.as_object_mut().unwrap().remove("schema_version");
    validate_against_schema(&schema, &value);
}

// ---------------------------------------------------------------------------
// Gate evidence with no active sprint (matrix row: the line renders from
// `gate_runs` rows, not from the presence of a sprint block)
// ---------------------------------------------------------------------------

#[test]
fn test_gates_line_renders_when_no_sprint_is_active() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E12");
    write_story(root, "E12S2", "ready", &["simon"], &[]);
    write_sprint(root, 4, "completed", &["E12S2"]);
    sync_rebuild(root);

    // Seed gate evidence directly (fixture plumbing — the pulse never writes).
    {
        let store = qdev_core::SqliteStore::open(root.join(".qdev/cache/cache.sqlite"))
            .expect("cache opens");
        use qdev_core::{GateRunRecord, Store};
        let mk = |id: &str, gate: &str, status: &str, at: &str| GateRunRecord {
            id: id.to_string(),
            story_id: Some("E12S2".to_string()),
            gate_id: gate.to_string(),
            commit_sha: "8f1b2c4".to_string(),
            status: Some(status.to_string()),
            exit_code: Some(if status == "pass" { 0 } else { 1 }),
            duration_ms: Some(500),
            metric_value: None,
            summary: Some(format!("{gate} {status}")),
            evidence_path: format!("docs/state/evidence/{id}.json"),
            output_hash: None,
            run_by_type: Some("human".to_string()),
            run_by_id: Some("simon".to_string()),
            ran_at: Some(at.to_string()),
        };
        store
            .upsert_gate_run(&mk("gr-early", "fmt", "pass", "2026-09-17T00:00:00Z"))
            .unwrap();
        store
            .upsert_gate_run(&mk("gr-late", "fmt", "fail", "2026-09-18T00:00:00Z"))
            .unwrap();
        store
            .upsert_gate_run(&mk("gr-lint", "lint", "pass", "2026-09-18T12:00:00Z"))
            .unwrap();
    }

    // No sprint block exists, so the count cannot hide inside one: it still renders,
    // between the "no active sprints" line and the Next section.
    let text = run_text(root, &[]);
    assert!(
        text.contains("  Gates          1/2 passing"),
        "Gates line must render with no active sprint:\n{text}"
    );
    let none_at = text
        .find("No active sprints — nothing to report")
        .unwrap_or_else(|| panic!("no-active-sprints line missing:\n{text}"));
    let gates_at = text
        .find("  Gates          1/2 passing")
        .unwrap_or_else(|| panic!("Gates line missing:\n{text}"));
    let next_at = text
        .find("Next\n")
        .unwrap_or_else(|| panic!("Next section missing:\n{text}"));
    assert!(
        none_at < gates_at && gates_at < next_at,
        "line order:\n{text}"
    );

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(value["gates"], json!({ "passing": 1, "total": 2 }));
    assert_eq!(value["sprints"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// D-5 phase mapping: a draft story recommends the specify phase
// ---------------------------------------------------------------------------

#[test]
fn test_draft_selection_renders_the_specify_phase() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E12");
    // The only candidate is a draft story — the canonical "not specced yet" case.
    write_story(root, "E12S1", "draft", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S1"]);
    sync_rebuild(root);

    let text = run_text(root, &[]);
    assert!(
        text.contains("  Run: /qdev-specify E12S1   (or: qdev context E12S1 --phase specify)"),
        "draft story must recommend the specify phase:\n{text}"
    );
    assert!(
        !text.contains("/qdev-develop"),
        "a draft story must not recommend develop:\n{text}"
    );

    // JSON carries the same selection with `status: "draft"`.
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(value["next"]["next"]["id"], "E12S1");
    assert_eq!(value["next"]["next"]["status"], "draft");
}

// ---------------------------------------------------------------------------
// Performance AC (checked form): no network git args, wall-clock bound
// ---------------------------------------------------------------------------

#[test]
fn test_only_local_git_probes_run_and_run_is_fast() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_git_workspace(root);
    seed_full_fixture(root);

    // Stand in a `git` shim that logs every invocation the pulse makes, then
    // run `qdev --json` with it first on PATH.
    let shim_dir = root.join("shim");
    let log = shim_dir.join("git-calls.log");
    fs::create_dir_all(&shim_dir).unwrap();
    let real_git = std::process::Command::new("which")
        .arg("git")
        .output()
        .unwrap()
        .stdout;
    let real_git = String::from_utf8(real_git).unwrap().trim().to_string();
    fs::write(
        shim_dir.join("git"),
        format!(
            "#!/bin/sh\necho \"git $*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real_git
        ),
    )
    .unwrap();
    fs::set_permissions(shim_dir.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let path_env = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("PATH", &path_env)
        .args(["--json"]);
    let started = Instant::now();
    let assert = cmd.assert();
    let elapsed = started.elapsed();
    let output = assert.get_output();
    assert!(output.status.success(), "qdev exited non-zero");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    // The probes ran and stayed local: the working tree line proves the cache
    // section came from real probes, and no network-flavoured git call was
    // ever issued.
    assert_eq!(value["environment"]["working_tree"]["clean"], true);
    let log_text = fs::read_to_string(&log).expect("shim logged the probes");
    let logged: Vec<&str> = log_text
        .lines()
        .filter(|l| l.starts_with("git "))
        .map(|l| l.strip_prefix("git ").unwrap())
        .collect();
    assert!(!logged.is_empty(), "git probes never ran:\n{log_text}");
    for args in &logged {
        assert!(
            args.starts_with("status --porcelain")
                || args.starts_with("rev-parse --abbrev-ref")
                || args.starts_with("rev-parse --short")
                || args.starts_with("rev-list --left-right --count")
                || args.starts_with("rev-parse --verify --quiet")
                // Lease discovery resolves the shared Git dir locally
                // (`discover_git_common_dir`) — local and read-only too.
                || args.starts_with("rev-parse --git-common-dir")
                // Identity resolution reads local git config (the
                // `resolve_author` chain) — local and read-only too.
                || args.starts_with("config user.email"),
            "unexpected git probe issued during the pulse: {args}"
        );
        for forbidden in ["fetch", "pull", "ls-remote", "clone", "push"] {
            assert!(
                !args.split_whitespace().any(|a| a == forbidden),
                "network git arg issued during the pulse: {args}"
            );
        }
    }
    // Generous wall-clock bound: the design target is 100 ms plus Git status
    // time; this only guards against a runaway (hydration, file-tree scans).
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "pulse took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// Integration states beyond `up_to_date` (Review row 24)
// ---------------------------------------------------------------------------

/// None of `behind`, `ahead`, `diverged`, `integration_branch_missing`, or
/// `remote_ref_missing` was ever produced by a fixture before this test — the
/// direction mapping in `integration_status` could have shipped inverted with a
/// green suite. Every non-`up_to_date` state is pinned here against a real
/// local-path origin.
#[test]
fn test_integration_states_across_local_and_origin() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_git_workspace(root);
    seed_full_fixture(root);

    // 1) A local-only commit on `develop` → 1 ahead.
    git(root, &["checkout", "develop"]);
    git(root, &["commit", "--allow-empty", "-m", "local-only"]);
    let text = run_text(root, &[]);
    assert!(
        text.contains("  Integration    develop is 1 ahead of origin/develop"),
        "1 ahead:\n{text}"
    );
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value["environment"]["integration"],
        json!({
            "remote": "origin",
            "branch": "develop",
            "state": "ahead",
            "ahead": 1,
            "behind": 0,
        }),
        "{value}"
    );

    // 2) Push that commit, then rewind `develop` past it → 1 behind.
    git(root, &["push", "origin", "develop"]);
    git(root, &["reset", "--hard", "HEAD~1"]);
    let text = run_text(root, &[]);
    assert!(
        text.contains("  Integration    develop is 1 behind origin/develop"),
        "1 behind:\n{text}"
    );
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value["environment"]["integration"],
        json!({
            "remote": "origin",
            "branch": "develop",
            "state": "behind",
            "ahead": 0,
            "behind": 1,
        }),
        "{value}"
    );

    // 3) A new local commit on the old base while origin keeps the pushed one
    //    → diverged, 1 ahead and 1 behind.
    git(root, &["commit", "--allow-empty", "-m", "diverged-local"]);
    let text = run_text(root, &[]);
    assert!(
        text.contains(
            "  Integration    develop has diverged from origin/develop (1 ahead, 1 behind)"
        ),
        "diverged:\n{text}"
    );
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value["environment"]["integration"],
        json!({
            "remote": "origin",
            "branch": "develop",
            "state": "diverged",
            "ahead": 1,
            "behind": 1,
        }),
        "{value}"
    );

    // 4) Drop the origin ref while local `develop` stays → the line names the
    //    missing remote ref; no fetch is ever attempted to "help".
    git(root, &["push", "origin", "--delete", "develop"]);
    let text = run_text(root, &[]);
    assert!(
        text.contains(
            "  Integration    'origin/develop' not found locally — run a remote update if \
             needed (qdev never does)"
        ),
        "remote ref missing:\n{text}"
    );
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value["environment"]["integration"],
        json!({
            "remote": "origin",
            "branch": "develop",
            "state": "remote_ref_missing",
            "ahead": null,
            "behind": null,
        }),
        "{value}"
    );

    // 5) Drop the local branch too → `refs_missing`, naming both.
    git(root, &["checkout", "main"]);
    git(root, &["branch", "-D", "develop"]);
    let text = run_text(root, &[]);
    assert!(
        text.contains(
            "  Integration    neither 'develop' nor 'origin/develop' found locally \
             (no remote update performed)"
        ),
        "both refs missing:\n{text}"
    );
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value["environment"]["integration"],
        json!({
            "remote": "origin",
            "branch": "develop",
            "state": "refs_missing",
            "ahead": null,
            "behind": null,
        }),
        "{value}"
    );
}

// ---------------------------------------------------------------------------
// Two active sprints share one workspace-wide Gates count (Review row 25)
// ---------------------------------------------------------------------------

/// Seeds `gate_runs` rows straight into the cache (fixture plumbing — the pulse
/// itself never writes): `fmt` ran twice (later run failed), `lint` once (passed).
fn seed_gate_runs(root: &Path) {
    let store =
        qdev_core::SqliteStore::open(root.join(".qdev/cache/cache.sqlite")).expect("cache opens");
    use qdev_core::{GateRunRecord, Store};
    let mk = |id: &str, gate: &str, status: &str, at: &str| GateRunRecord {
        id: id.to_string(),
        story_id: Some("E12S1".to_string()),
        gate_id: gate.to_string(),
        commit_sha: "8f1b2c4".to_string(),
        status: Some(status.to_string()),
        exit_code: Some(if status == "pass" { 0 } else { 1 }),
        duration_ms: Some(500),
        metric_value: None,
        summary: Some(format!("{gate} {status}")),
        evidence_path: format!("docs/state/evidence/{id}.json"),
        output_hash: None,
        run_by_type: Some("human".to_string()),
        run_by_id: Some("simon".to_string()),
        ran_at: Some(at.to_string()),
    };
    store
        .upsert_gate_run(&mk("gr-early", "fmt", "pass", "2026-09-17T00:00:00Z"))
        .unwrap();
    store
        .upsert_gate_run(&mk("gr-late", "fmt", "fail", "2026-09-18T00:00:00Z"))
        .unwrap();
    store
        .upsert_gate_run(&mk("gr-lint", "lint", "pass", "2026-09-18T12:00:00Z"))
        .unwrap();
}

/// The D-4 contract: the `Gates N/M passing` count is workspace-wide, so with two
/// active sprints it renders exactly once — in the first block — while every block
/// keeps its own Stories and Deferred work lines.
#[test]
fn test_gates_line_renders_once_across_two_active_sprints() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E12");
    write_story(root, "E12S1", "done", &["simon"], &[]);
    write_story(root, "E12S2", "ready", &["simon"], &[]);
    write_story(root, "E12S3", "in-progress", &["simon"], &[]);
    write_story(root, "E12S4", "done", &["simon"], &[]);
    write_story(root, "E12S5", "ready", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S1", "E12S2", "E12S3"]);
    write_sprint(root, 6, "active", &["E12S4", "E12S5"]);
    write_dw(root, "DW-1111", "open", "unacceptable", "E12S1");
    sync_rebuild(root);
    seed_gate_runs(root);

    let text = run_text(root, &[]);
    // Both active sprints get their own block...
    assert!(
        text.contains("Sprint 5 — Sprint 5  [release 0.1.0]"),
        "{text}"
    );
    assert!(
        text.contains("Sprint 6 — Sprint 6  [release 0.1.0]"),
        "{text}"
    );
    // ...but the workspace-wide count renders exactly once, in the first block:
    // between the first block's Deferred work line and the second block's header.
    assert_eq!(
        text.match_indices("  Gates          1/2 passing").count(),
        1,
        "the Gates count must not repeat per block:\n{text}"
    );
    let first_dw = text
        .find("  Deferred work  1 open (1 unacceptable)")
        .unwrap();
    let gates = text.find("  Gates          1/2 passing").unwrap();
    let sprint6 = text.find("Sprint 6 — Sprint 6").unwrap();
    assert!(first_dw < gates && gates < sprint6, "line order:\n{text}");
    // The second block still reports its own counts (E12S4 done, E12S5 backlog)
    // and the same workspace-wide DW line.
    let second_block = &text[sprint6..];
    assert!(
        second_block.contains("  Stories        1 done / 0 in progress / 0 blocked / 1 backlog"),
        "second block counters:\n{second_block}"
    );
    assert!(
        second_block.contains("  Deferred work  1 open (1 unacceptable)"),
        "second block DW line:\n{second_block}"
    );

    // JSON carries the one workspace-wide summary and both blocks.
    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(value["gates"], json!({ "passing": 1, "total": 2 }));
    let sprints = value["sprints"].as_array().unwrap();
    assert_eq!(sprints.len(), 2, "{value}");
    assert_eq!(sprints[0]["id"], 5);
    assert_eq!(sprints[1]["id"], 6);
    assert_eq!(
        sprints[1]["stories"],
        json!({ "done": 1, "in_progress": 0, "blocked": 0, "backlog": 1 }),
        "{value}"
    );
}

// ---------------------------------------------------------------------------
// An uninitialized or unreadable cache is reported, never repaired
// ---------------------------------------------------------------------------

/// The frozen `Never` clause: "an unsynced workspace is reported as stale cache, not
/// repaired". With the cache deleted the pulse still exits 0, reports the cache as
/// degraded with every count `null`, lists no sprints, and recommends nothing — and it
/// creates neither the cache file nor its directory.
#[test]
fn test_workspace_with_no_cache_reports_degraded_and_creates_nothing() {
    for args in [
        vec![],
        vec!["status"],
        vec!["--json"],
        vec!["status", "--json"],
    ] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        setup_workspace(root);
        write_epic(root, "E12");
        write_story(root, "E12S1", "ready", &["simon"], &[]);
        write_sprint(root, 5, "active", &["E12S1"]);

        // Delete everything the workspace has, cache included: `qdev` must not put it
        // back.
        fs::remove_dir_all(root.join(".qdev/cache")).unwrap();
        assert!(!root.join(".qdev/cache").exists());

        let degraded = json!({
            "schema_status": "mismatch",
            "entity_count": null,
            "finding_count": null,
            "synced_ms_ago": null,
        });

        if args.last().copied() == Some("--json") {
            let (assert, value) = run_json(root, &args);
            assert.success().code(0);
            assert_eq!(value["workspace"], true, "{args:?}");
            assert_eq!(value["environment"]["cache"], degraded, "{args:?}");
            // Empty array, not null: inside a workspace the sections still render.
            assert_eq!(value["sprints"], json!([]), "{args:?}");
            assert!(value["gates"].is_null(), "{args:?}");
            // No store, so `select_next` cannot run and nothing is recommended.
            assert!(value["next"].is_null(), "{args:?}");

            // And the degraded payload still round-trips `payload-pulse.json`.
            let schema_assert = Command::cargo_bin("qdev")
                .unwrap()
                .args(["schema", "payload", "pulse", "--json"])
                .assert()
                .success();
            let mut schema: Value =
                serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
            schema.as_object_mut().unwrap().remove("schema_version");
            validate_against_schema(&schema, &value);
        } else {
            let text = run_text(root, &args);
            assert!(
                text.contains("  Cache          degraded (cache schema mismatch"),
                "{args:?}:\n{text}"
            );
        }

        // Nothing was created, for the text run or the JSON run.
        assert!(
            !root.join(".qdev/cache/cache.sqlite").exists(),
            "the cache must not be created ({args:?})"
        );
        assert!(
            !root.join(".qdev/cache").exists(),
            "the cache directory must not be created ({args:?})"
        );
    }
}

/// The text screen keeps every section: the Cache line reports the gap and the Next
/// section says nothing can be recommended, instead of the Next section disappearing.
#[test]
fn test_missing_cache_text_still_renders_a_next_section() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    fs::remove_dir_all(root.join(".qdev/cache")).unwrap();

    let text = run_text(root, &[]);
    assert!(
        text.contains(
            "  Cache          degraded (cache schema mismatch — run `qdev sync --rebuild`; \
             ? entities, ? findings)"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "Next\n  Nothing to recommend — there is no readable cache to select from \
             (run `qdev sync`)"
        ),
        "the Next section must render with a hint to sync:\n{text}"
    );
    // And it sits where the Next section always sits: after the sprints.
    let sprints_at = text.find("Sprints\n  No active sprints").unwrap();
    let next_at = text.find("Next\n  Nothing to recommend").unwrap();
    assert!(sprints_at < next_at, "{text}");
    assert!(
        !text.contains("Run: /qdev-"),
        "nothing can be recommended:\n{text}"
    );
}

/// A cache whose stamp this binary cannot read is reported the same way — and is not
/// recreated: the stale stamp and the legacy table are still there afterwards.
#[test]
fn test_mismatched_cache_is_reported_and_not_rebuilt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S1", "ready", &["simon"], &[]);
    write_sprint(root, 5, "active", &["E12S1"]);

    use qdev_core::rusqlite;
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    {
        let conn = rusqlite::Connection::open(&cache_db).unwrap();
        conn.execute_batch("PRAGMA user_version = 0; CREATE TABLE old_legacy_table (val TEXT);")
            .unwrap();
    }

    let text = run_text(root, &[]);
    assert!(
        text.contains("  Cache          degraded (cache schema mismatch"),
        "{text}"
    );

    let (assert, value) = run_json(root, &["--json"]);
    assert.success().code(0);
    assert_eq!(
        value["environment"]["cache"],
        json!({
            "schema_status": "mismatch",
            "entity_count": null,
            "finding_count": null,
            "synced_ms_ago": null,
        }),
        "{value}"
    );
    assert_eq!(value["sprints"], json!([]));
    assert!(value["next"].is_null(), "no store, so no selection");

    // The cache is exactly as broken as it was: no rebuild, no restamped sync.
    let conn = rusqlite::Connection::open(&cache_db).unwrap();
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        user_version, 0,
        "the pulse must not recreate or restamp the cache"
    );
    let legacy: bool = conn
        .query_row(
            "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name='old_legacy_table';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(legacy, "the legacy table must survive — nothing rebuilt");
}

/// The one refusal: a cache stamped *newer* than this binary supports. The pulse refuses
/// exactly as every other command does — same `schema_version_mismatch`, exit 5 — so the
/// story-1.6 contract still holds for it.
#[test]
fn test_newer_cache_is_refused_for_the_pulse_with_exit_5() {
    use qdev_core::CACHE_SCHEMA_VERSION;

    for args in [vec![], vec!["status"], vec!["--json"]] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        setup_workspace(root);
        write_story(root, "E12S1", "ready", &["simon"], &[]);

        let cache_db = root.join(".qdev/cache/cache.sqlite");
        {
            let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
            conn.execute_batch(&format!(
                "PRAGMA user_version = {};",
                CACHE_SCHEMA_VERSION + 1
            ))
            .unwrap();
        }

        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd.current_dir(root).args(&args).assert().failure().code(5);
        let output = assert.get_output();
        // `--json` writes the envelope to stdout, text mode to stderr: check both.
        let reported = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            reported.contains("newer than supported version")
                && reported.contains("qdev sync --rebuild")
                && reported.contains("schema_version_mismatch"),
            "{args:?}: {reported}"
        );

        // Still stamped at the newer version — the pulse did not rebuild over it.
        let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
        let user_version: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(user_version, CACHE_SCHEMA_VERSION + 1, "{args:?}");
    }
}

// ---------------------------------------------------------------------------
// Purity: the pulse writes nothing, twice over
// ---------------------------------------------------------------------------

#[test]
fn test_pulse_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_git_workspace(root);
    seed_full_fixture(root);

    // Warm any first-open artifacts, then compare.
    run_text(root, &[]);
    let before = snapshot_tree(root);
    run_json(root, &["--json"]);
    let after = snapshot_tree(root);
    assert_eq!(
        before.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        after.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        "file list changed — nothing may be created"
    );
    for ((path, content), (after_path, after_content)) in before.iter().zip(&after) {
        assert_eq!(path, after_path);
        assert_eq!(
            content, after_content,
            "file {path} changed — the pulse must not mutate anything"
        );
    }
}
