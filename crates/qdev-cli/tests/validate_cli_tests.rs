//! `qdev validate` CLI tests (spec-1-11): `--json` shape, exit codes, `--changed` filtering, and
//! `--fix-ids` non-interactive refusal / guided renumber.

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

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

fn write_story(dir: &Path, id: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

fn read_story(dir: &Path, id: &str) -> String {
    fs::read_to_string(dir.join(format!("{}.md", id))).unwrap()
}

fn git(root: &Path, args: &[&str]) {
    let mut full_args = vec!["-c", "commit.gpgsign=false"];
    full_args.extend_from_slice(args);
    let status = StdCommand::new("git")
        .args(&full_args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn test_validate_happy_path_clean_workspace_exit_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["findings"].as_array().unwrap().len(), 0);
}

// ---------------------------------------------------------------------------
// Duplicate ids -> JSON shape {code, severity, path, message}, exit 1
// ---------------------------------------------------------------------------

#[test]
fn test_validate_reports_duplicate_planning_id_json_shape_and_exit_1() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 2);
    for f in findings {
        assert_eq!(f["code"], "duplicate_planning_id");
        assert_eq!(f["severity"], "error");
        assert!(f["path"].is_string());
        assert!(f["message"].is_string());
    }
}

// ---------------------------------------------------------------------------
// Unregistered target module (seeded via `qdev update --field`)
// ---------------------------------------------------------------------------

#[test]
fn test_validate_reports_unregistered_target_module() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", "E1S1", "--field", "target_modules=[\"ghost\"]"])
        .assert()
        .success();

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert = cmd2
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert!(findings
        .iter()
        .any(|f| f["code"] == "target_module_not_registered"));
}

// ---------------------------------------------------------------------------
// --changed filtering
// ---------------------------------------------------------------------------

#[test]
fn test_validate_changed_excludes_untouched_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    git(root, &["init", "-q", "-b", "develop"]);
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    fs::write(
        stories_dir.join("E1S1-dup.md"),
        fs::read_to_string(stories_dir.join("E1S1.md")).unwrap(),
    )
    .unwrap();

    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "baseline with duplicate ids"]);
    git(root, &["checkout", "-q", "-b", "feature"]);

    // Nothing has changed relative to develop yet: --changed must exclude every finding.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--changed", "--json"])
        .assert()
        .success()
        .code(0);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["findings"].as_array().unwrap().len(), 0);

    // Touch one of the two duplicate files on the feature branch: only its finding survives.
    // (Appending, not rewriting: `write_story` produces byte-identical content, which `git diff`
    // would not consider a change at all.)
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(stories_dir.join("E1S1.md"))
        .unwrap();
    use std::io::Write;
    writeln!(f, "- extra AC.").unwrap();
    drop(f);

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["validate", "--changed", "--json"])
        .assert()
        .failure()
        .code(1);
    let output2 = assert2.get_output();
    let val2: Value = serde_json::from_str(std::str::from_utf8(&output2.stdout).unwrap()).unwrap();
    let findings2 = val2["findings"].as_array().unwrap();
    assert_eq!(findings2.len(), 1);
    assert_eq!(findings2[0]["path"], "docs/specs/stories/E1S1.md");
}

// ---------------------------------------------------------------------------
// --fix-ids gating
// ---------------------------------------------------------------------------

#[test]
fn test_fix_ids_non_interactive_without_yes_refuses_exit_3_no_writes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();
    let before = read_story(&stories_dir, "E1S2-dup");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--fix-ids", "--non-interactive", "--json"])
        .assert()
        .failure()
        .code(3);

    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    assert_eq!(val["error"]["code"], "needs_confirmation");

    let after = read_story(&stories_dir, "E1S2-dup");
    assert_eq!(before, after, "no write must occur on refusal");
}

#[test]
fn test_fix_ids_with_yes_renumbers_duplicate_and_leaves_workspace_clean() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    fs::write(
        stories_dir.join("E1S1-dup.md"),
        fs::read_to_string(stories_dir.join("E1S1.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["validate", "--fix-ids", "--non-interactive", "--yes"])
        .assert()
        .success()
        .code(0);

    // The lexicographically-first path keeps the id ("E1S1-dup.md" sorts before "E1S1.md" because
    // '-' < '.'); the other duplicate is renumbered to the next available Story id for the same
    // epic, with its version bumped. Check both files rather than assuming which one wins.
    let a = read_story(&stories_dir, "E1S1");
    let b = read_story(&stories_dir, "E1S1-dup");
    let a_keeps_id = a.contains("id: E1S1\n");
    let b_keeps_id = b.contains("id: E1S1\n");
    assert_ne!(
        a_keeps_id, b_keeps_id,
        "exactly one file must keep id E1S1; a={:?} b={:?}",
        a, b
    );
    let renumbered = if a_keeps_id { &b } else { &a };
    assert!(!renumbered.contains("id: E1S1\n"));
    assert!(renumbered.contains("version: 2"));

    // A follow-up plain validate call must now be clean.
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);
    let output2 = assert2.get_output();
    let val2: Value = serde_json::from_str(std::str::from_utf8(&output2.stdout).unwrap()).unwrap();
    assert_eq!(val2["findings"].as_array().unwrap().len(), 0);
}

/// References follow the renumbered entity: a relation targeting the duplicated id is redirected
/// to the new id, via the same write path `qdev relate`/`qdev unrelate` use.
///
/// Note what this necessarily means in the duplicate case — the group's first file *keeps* the
/// old id, so an edge that actually meant the keeper is redirected away from it. Nothing records
/// which of the colliding files a reference meant; this is the decided behaviour, not an
/// inference (see the boundary note in spec-1-11).
#[test]
fn test_fix_ids_redirects_relations_to_the_renumbered_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    write_story(&stories_dir, "E1S2");
    // Sorts before "E1S2.md", so this file keeps the id and "E1S2.md" is the one renumbered.
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut relate_cmd = Command::cargo_bin("qdev").unwrap();
    relate_cmd
        .current_dir(root)
        .args(["relate", "E1S1", "depends_on", "E1S2"])
        .assert()
        .success();

    let mut fix_cmd = Command::cargo_bin("qdev").unwrap();
    let assert = fix_cmd
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1);
    assert_eq!(renumbered[0]["old_id"], "E1S2");
    let new_id = renumbered[0]["new_id"].as_str().unwrap().to_string();

    assert_eq!(
        renumbered[0]["relations_rewritten"].as_array().unwrap(),
        &vec![Value::String("E1S1".to_string())],
        "the source whose relation was redirected must be reported"
    );

    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    let get_assert = get_cmd
        .current_dir(root)
        .args(["get", "E1S1", "--json"])
        .assert()
        .success();
    let get_output = get_assert.get_output();
    let get_val: Value =
        serde_json::from_str(std::str::from_utf8(&get_output.stdout).unwrap()).unwrap();
    assert_eq!(
        get_val["relations"]["depends_on"].as_array().unwrap(),
        &vec![Value::String(new_id)],
        "the depends_on edge must now target the renumbered id"
    );
}

/// Citations follow the renumbered entity too, under the configured `[[modules]]` globs, using
/// `hygiene.citation_pattern`. Only occurrences whose captured id is exactly the old id are
/// touched — the pattern matches every bracket citation kind.
#[test]
fn test_fix_ids_redirects_citations_under_configured_module_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Register a module whose paths cover a citation-bearing source file.
    let mut toml_contents = fs::read_to_string(root.join("qdev.toml")).unwrap();
    toml_contents.push_str("\n[[modules]]\nid = \"core\"\npaths = [\"src/**\"]\n");
    fs::write(root.join("qdev.toml"), toml_contents).unwrap();

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "// see [E1S2] and [AD-3] for context\nfn noop() {}\n",
    )
    .unwrap();

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1);
    assert_eq!(renumbered[0]["old_id"], "E1S2");
    let new_id = renumbered[0]["new_id"].as_str().unwrap().to_string();
    assert_eq!(renumbered[0]["citations_rewritten"], 1);

    let src_content = fs::read_to_string(src_dir.join("lib.rs")).unwrap();
    assert!(src_content.contains(&format!("[{}]", new_id)));
    assert!(!src_content.contains("[E1S2]"));
    assert!(
        src_content.contains("[AD-3]"),
        "an unrelated citation must be left alone: {src_content}"
    );
}

#[test]
fn test_fix_ids_no_duplicates_is_a_clean_noop() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["validate", "--fix-ids", "--non-interactive", "--yes"])
        .assert()
        .success()
        .code(0);
}

// ---------------------------------------------------------------------------
// The interactive --fix-ids confirmation
// ---------------------------------------------------------------------------

/// The `[y/N]` prompt is the whole safety property of a "guided" renumber, and every other
/// `--fix-ids` test passes `--yes`, which skips it entirely. Declining must leave the file
/// exactly as it was and report it as skipped.
#[test]
fn test_fix_ids_declining_the_prompt_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();
    let before = read_story(&stories_dir, "E1S2");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .write_stdin("n\n")
        .args(["validate", "--fix-ids", "--json"])
        .assert();
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();

    assert!(
        val["renumbered"].as_array().unwrap().is_empty(),
        "declining must renumber nothing, got {:?}",
        val["renumbered"]
    );
    assert_eq!(
        val["skipped"].as_array().unwrap(),
        &vec![Value::String("docs/specs/stories/E1S2.md".to_string())],
        "the declined file must be reported as skipped"
    );
    assert_eq!(
        read_story(&stories_dir, "E1S2"),
        before,
        "declining must leave the file byte-for-byte unchanged"
    );
}

#[test]
fn test_fix_ids_accepting_the_prompt_renumbers() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .env("_QDEV_MOCK_TTY", "1")
        .write_stdin("y\n")
        .args(["validate", "--fix-ids", "--json"])
        .assert();
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();

    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1, "accepting must renumber the duplicate");
    assert_eq!(renumbered[0]["old_id"], "E1S2");
    assert!(val["skipped"].as_array().unwrap().is_empty());
    assert!(!read_story(&stories_dir, "E1S2").contains("id: E1S2\n"));
}

// ---------------------------------------------------------------------------
// Flag-combination guards
// ---------------------------------------------------------------------------

/// `--changed` scopes the run to the current diff and `--fix-ids` rewrites ids workspace-wide;
/// combining them is refused rather than silently ignoring the scoping the user asked for.
#[test]
fn test_changed_with_fix_ids_is_refused_and_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();
    let before = read_story(&stories_dir, "E1S2");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "validate",
            "--changed",
            "--fix-ids",
            "--non-interactive",
            "--yes",
        ])
        .assert()
        .code(2);

    assert_eq!(
        read_story(&stories_dir, "E1S2"),
        before,
        "a refused run must not renumber anything"
    );
}

// ---------------------------------------------------------------------------
// Cache-native findings surfaced through the CLI
// ---------------------------------------------------------------------------

/// The seam between hydration (story 1.10, which writes `dependency_cycle` findings) and
/// `qdev validate` (story 1.11, which reads them back). Covered only at the `Store` level
/// otherwise, so a regression in the finding codes or in the `list_findings` merge would ship
/// green.
#[test]
fn test_validate_surfaces_hydration_dependency_cycle_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    for (id, target) in [("E1S1", "E1S2"), ("E1S2", "E1S1")] {
        fs::write(
            stories_dir.join(format!("{}.md", id)),
            format!(
                r#"---
id: {id}
title: "Story {id}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
relations:
  depends_on: ["{target}"]
---

## Acceptance Criteria
- AC.
"#
            ),
        )
        .unwrap();
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .code(1);
    let output = assert.get_output();
    let val: Value = serde_json::from_str(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
    let findings = val["findings"].as_array().unwrap();

    for id in ["E1S1", "E1S2"] {
        let path = format!("docs/specs/stories/{}.md", id);
        assert!(
            findings.iter().any(|f| {
                f["code"] == "dependency_cycle" && f["path"] == Value::String(path.clone())
            }),
            "{} must carry a dependency_cycle finding; got {:#?}",
            id,
            findings
        );
    }
}

// ---------------------------------------------------------------------------
// Locking, id allocation, and partial-failure reporting
// ---------------------------------------------------------------------------

/// Holds `write.lock` for `hold` while `body` runs, so a command can be observed contending for
/// it. Mirrors `update_cli_tests.rs`'s lock-contention fixture.
fn while_write_lock_held<T>(root: &Path, hold: std::time::Duration, body: impl FnOnce() -> T) -> T {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let lock_path = root.join(".qdev/cache/write.lock");
    let acquired = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let (acquired_c, release_c, lock_path_c) = (acquired.clone(), release.clone(), lock_path);

    let holder = std::thread::spawn(move || {
        let _guard =
            qdev_core::acquire_write_lock(&lock_path_c, std::time::Duration::from_millis(5000))
                .unwrap();
        acquired_c.store(true, Ordering::SeqCst);
        let start = std::time::Instant::now();
        while start.elapsed() < hold && !release_c.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });
    while !acquired.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let result = body();
    release.store(true, Ordering::SeqCst);
    let _ = holder.join();
    result
}

/// The renumber writes entity files and cache rows directly, so it must take the same advisory
/// lock `qdev update` and `qdev relate` take. Without it a concurrent write can interleave with
/// a half-rewritten file; nothing else in the suite observes the acquisition.
#[test]
fn test_fix_ids_waits_on_the_advisory_write_lock() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();
    let before = read_story(&stories_dir, "E1S2");

    let root_buf = root.to_path_buf();
    while_write_lock_held(root, std::time::Duration::from_millis(6000), || {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd
            .current_dir(&root_buf)
            .args([
                "validate",
                "--fix-ids",
                "--non-interactive",
                "--yes",
                "--json",
            ])
            .timeout(std::time::Duration::from_secs(15))
            .assert()
            .failure()
            .code(5);
        let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        assert_eq!(val["error"]["code"], "lock_timeout");
    });

    assert_eq!(
        read_story(&stories_dir, "E1S2"),
        before,
        "a renumber that could not take the lock must not have written"
    );
}

/// The replacement id must not collide with an entity the `specs_dir` scan cannot see. Seeding
/// the in-use set from the cache as well as the scan is what prevents a renumber from fixing one
/// duplicate by minting another.
#[test]
fn test_fix_ids_does_not_allocate_an_id_already_used_outside_specs_dir() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // `next_available_id` restarts at 1 for the story's epic, so E1S1 is what it would pick —
    // put an entity holding that id somewhere the specs_dir scan never walks.
    let outside_dir = root.join("docs/state/scratch");
    write_story(&outside_dir, "E1S1");

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1);

    // Whatever id was chosen, it must not be one already in the cache.
    let new_id = renumbered[0]["new_id"].as_str().unwrap();
    let cached_ids: Vec<String> = {
        let store =
            qdev_core::store::SqliteStore::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
        use qdev_core::store::Store;
        store
            .list_entities(&qdev_core::EntityFilter::default())
            .unwrap()
            .into_iter()
            .map(|e| e.id)
            .collect()
    };
    assert_eq!(
        cached_ids.iter().filter(|id| *id == new_id).count(),
        1,
        "the renumber must not have reused an id that already existed elsewhere; \
         new_id = {new_id}, cache = {cached_ids:?}"
    );
}

/// A failure part-way through must still report what was already written. Returning straight out
/// of the loop would leave renumbered files on disk with no record of which ones — and in JSON
/// mode the report must remain a single parseable document, since `emit_error` writes to stdout
/// there and a second envelope would make the report unreadable.
#[test]
fn test_fix_ids_reports_what_it_wrote_when_a_later_file_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    // Group one renumbers cleanly.
    write_story(&stories_dir, "E1S2");
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        fs::read_to_string(stories_dir.join("E1S2.md")).unwrap(),
    )
    .unwrap();

    // Group two (sorted after group one) has no `title`, so it is scanned as a duplicate but
    // fails schema validation once renumbered — the renumber aborts on it.
    let no_title = r#"---
id: E2S2
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#;
    fs::write(stories_dir.join("E2S2-dup.md"), no_title).unwrap();
    fs::write(stories_dir.join("E2S2.md"), no_title).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args([
            "validate",
            "--fix-ids",
            "--non-interactive",
            "--yes",
            "--json",
        ])
        .assert()
        .failure();

    let stdout = assert.get_output().stdout.clone();
    // One document, not a payload envelope followed by a separate error envelope.
    let val: Value = serde_json::from_slice(&stdout)
        .expect("the abort report must stay a single parseable JSON document");

    assert_eq!(
        val["renumbered"].as_array().unwrap().len(),
        1,
        "the file that was successfully rewritten must still be reported: {val}"
    );
    assert_eq!(val["renumbered"][0]["old_id"], "E1S2");
    assert!(
        val["error"].is_object(),
        "the abort must be reported inside the payload: {val}"
    );

    // The successfully renumbered file really is on disk, and the cache was reconciled with it
    // despite the abort.
    assert!(!read_story(&stories_dir, "E1S2").contains("id: E1S2\n"));
    let new_id = val["renumbered"][0]["new_id"].as_str().unwrap().to_string();
    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    get_cmd
        .current_dir(root)
        .args(["get", &new_id, "--json"])
        .assert()
        .success();
}
