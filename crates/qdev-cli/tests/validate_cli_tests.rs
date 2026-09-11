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
        .code(0)
        // The move is reported in text mode too, not only to `--json` consumers: a user who is
        // not told about the rename sees an unexplained rename in `git status`.
        .stdout(predicates::str::contains(
            "Renamed docs/specs/stories/E1S1.md -> docs/specs/stories/E1S2.md",
        ));

    // The lexicographically-first path keeps the id ("E1S1-dup.md" sorts before "E1S1.md" because
    // '-' < '.'); the other duplicate is renumbered to the next available Story id for the same
    // epic, with its version bumped — and renamed to carry that id, so it stays writable.
    let keeper = read_story(&stories_dir, "E1S1-dup");
    assert!(
        keeper.contains("id: E1S1\n"),
        "the group's first file keeps the id: {keeper}"
    );
    assert!(
        !stories_dir.join("E1S1.md").exists(),
        "the renumbered file must have been renamed away from its old id"
    );
    let renumbered = read_story(&stories_dir, "E1S2");
    assert!(renumbered.contains("id: E1S2\n"));
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
    // The renumber renames the file to carry the new id, so the old name is gone and the new one
    // holds the new id. (This assertion previously read `E1S2.md` back in place, which is the
    // manufactured divergence the identity-seam story removed.)
    assert_eq!(renumbered[0]["old_path"], "docs/specs/stories/E1S2.md");
    let new_id = renumbered[0]["new_id"].as_str().unwrap();
    assert_eq!(
        renumbered[0]["new_path"],
        format!("docs/specs/stories/{}.md", new_id)
    );
    assert!(!stories_dir.join("E1S2.md").exists());
    assert!(read_story(&stories_dir, new_id).contains(&format!("id: {}\n", new_id)));
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
    // The hold ends when the body returns; this is only a safety valve for a panicking assert,
    // so it must clear the command's 5s timeout plus spawn time (see `update_cli_tests.rs`).
    while_write_lock_held(root, std::time::Duration::from_millis(60_000), || {
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
    // put an entity holding that id in `state_dir` rather than `specs_dir`.
    //
    // NOTE: this no longer tests what its name says. The duplicate scan was widened to walk
    // `state_dir` too, so `scan.all_ids` now supplies E1S1 on its own and the cache-seeding of
    // `used_ids` this test was written to pin can be deleted with the test still green. An
    // entity that is in the cache but under *no* scanned directory turns out to be
    // unconstructible — the next sweep sees its path as absent and purges the row, which is
    // correct — so the seeding's remaining purpose is unclear. Filed in `deferred-work.md`
    // rather than papered over with a fixture that cannot exist.
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

    // The successfully renumbered file really is on disk under its new name, and the cache was
    // reconciled with it despite the abort.
    let new_id = val["renumbered"][0]["new_id"].as_str().unwrap().to_string();
    assert!(!stories_dir.join("E1S2.md").exists());
    assert!(read_story(&stories_dir, &new_id).contains(&format!("id: {}\n", new_id)));
    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    get_cmd
        .current_dir(root)
        .args(["get", &new_id, "--json"])
        .assert()
        .success();
}

// ---------------------------------------------------------------------------
// The identity seam: what `--fix-ids` leaves behind must be writable
// ---------------------------------------------------------------------------

/// The headline defect: `--fix-ids` used to rewrite the frontmatter id without renaming the
/// file, manufacturing an entity `qdev get` could read and no writer could resolve. A renumbered
/// entity must accept `qdev update` and `qdev relate` immediately, with no manual `mv`.
#[test]
fn test_renumbered_entity_accepts_update_and_relate_with_no_manual_step() {
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
    // A relate target that is not part of the duplicate group.
    write_story(&stories_dir, "E1S5");

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
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let entry = &val["renumbered"][0];
    let new_id = entry["new_id"].as_str().unwrap().to_string();

    // The move is reported, not silent: old and new path as well as old and new id.
    assert_eq!(entry["old_id"], "E1S1");
    assert_eq!(entry["old_path"], "docs/specs/stories/E1S1.md");
    assert!(
        entry.get("path").is_none(),
        "the duplicated `path` field is gone: after the rename it named a deleted file"
    );
    assert_eq!(
        entry["new_path"],
        format!("docs/specs/stories/{}.md", new_id)
    );

    // `get` reads it...
    let mut get_cmd = Command::cargo_bin("qdev").unwrap();
    get_cmd
        .current_dir(root)
        .args(["get", &new_id, "--json"])
        .assert()
        .success();

    // ...and so do both writers, which is what used to fail with "Entity file not found".
    let mut update_cmd = Command::cargo_bin("qdev").unwrap();
    update_cmd
        .current_dir(root)
        .args(["update", &new_id, "--status", "ready"])
        .assert()
        .success();

    let mut relate_cmd = Command::cargo_bin("qdev").unwrap();
    relate_cmd
        .current_dir(root)
        .args(["relate", &new_id, "depends_on", "E1S5"])
        .assert()
        .success();
}

/// H3: the reconcile is gated on "did we write anything", not on an entry surviving the two
/// fallible steps after the write. When the relation step aborts, the keeper's id must still be
/// in the cache — asserted through `qdev get`, not by reading the cache, so it survives a change
/// of resolution rule.
#[test]
fn test_fix_ids_abort_in_the_relation_step_still_reports_and_reconciles() {
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

    // A referencing story in a file whose name does not carry its id: readable by hydration,
    // unresolvable by the write path, so redirecting its `depends_on` edge fails. That is
    // exactly the abort the old code turned into "renumbered: []" plus a lost keeper.
    fs::write(
        stories_dir.join("misnamed.md"),
        r#"---
id: E1S8
title: "Story E1S8"
status: draft
version: 1
relations:
  depends_on: ["E1S2"]
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
        .failure();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout)
        .expect("the abort report must stay a single parseable JSON document");

    // The write that happened before the abort is reported, with its move.
    let renumbered = val["renumbered"].as_array().unwrap();
    assert_eq!(renumbered.len(), 1, "{val}");
    let new_id = renumbered[0]["new_id"].as_str().unwrap().to_string();
    assert_ne!(renumbered[0]["new_path"], renumbered[0]["old_path"]);
    assert!(val["error"].is_object(), "{val}");

    // The keeper's id is present in the cache afterwards: the reconcile ran despite the abort.
    let mut keeper_get = Command::cargo_bin("qdev").unwrap();
    keeper_get
        .current_dir(root)
        .args(["get", "E1S2", "--json"])
        .assert()
        .success();

    // And so is the renumbered entity, under its new id.
    let mut new_get = Command::cargo_bin("qdev").unwrap();
    new_get
        .current_dir(root)
        .args(["get", &new_id, "--json"])
        .assert()
        .success();
}

/// An occupied rename target is refused rather than clobbered: the entry is reported as skipped
/// and the run exits per the refusal's own error.
#[test]
fn test_fix_ids_refuses_an_occupied_rename_target_instead_of_clobbering() {
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

    // The rename target is occupied by something that is not a markdown file, so it puts no id
    // in use: a *file* named `E1S2.md` could not reach this path any more, because the in-use
    // id set now includes ids carried by file names, so `E1S2` would never be allocated in the
    // first place. This is what is left of the occupied-target case, and refusing it still
    // matters — `write_file_atomic` would otherwise be pointed at a directory.
    fs::create_dir_all(stories_dir.join("E1S2.md")).unwrap();

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
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();

    assert!(val["renumbered"].as_array().unwrap().is_empty(), "{val}");
    assert_eq!(
        val["skipped"].as_array().unwrap(),
        &vec![Value::from("docs/specs/stories/E1S1.md")],
        "the refused entry must be reported as skipped: {val}"
    );
    assert_eq!(val["error"]["code"], "rename_target_exists");

    // The occupant is untouched, and so is the file that would have been renamed onto it.
    assert!(stories_dir.join("E1S2.md").is_dir());
    assert!(read_story(&stories_dir, "E1S1").contains("id: E1S1\n"));
    assert!(stories_dir.join("E1S1-dup.md").is_file());
}

// ---------------------------------------------------------------------------
// The convention is enforced, not assumed
// ---------------------------------------------------------------------------

/// A story whose filename does not carry its id is reported once, at `warning` severity, naming
/// the file and the name it should have — and `qdev validate` still exits 0, so a workspace that
/// was legal before this shipped does not start failing.
#[test]
fn test_off_convention_filename_is_a_warning_and_validate_still_exits_0() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("login-flow.md"),
        r#"---
id: E1S9
title: "Story E1S9"
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
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{val}");
    assert_eq!(findings[0]["code"], "entity_file_off_convention");
    assert_eq!(findings[0]["severity"], "warning");
    assert_eq!(findings[0]["path"], "docs/specs/stories/login-flow.md");
    let message = findings[0]["message"].as_str().unwrap();
    assert!(message.contains("docs/specs/stories/E1S9.md"), "{message}");
}

/// Every other `--fix-ids` test renumbers a Story. Non-Story planning kinds go through the same
/// renumber, rename and cache-purge path, and the purge's detail-row deletion is kind-dependent —
/// so one non-Story case belongs in the suite.
#[test]
fn test_fix_ids_renumbers_a_non_story_kind_and_renames_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let adrs = root.join("docs/specs/adrs");
    fs::create_dir_all(&adrs).unwrap();
    let adr = |title: &str| {
        format!(
            r#"---
id: AD-1
title: "{title}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Decision
- Chosen.
"#
        )
    };
    fs::write(adrs.join("AD-1.md"), adr("Keeper")).unwrap();
    fs::write(adrs.join("AD-1-copy.md"), adr("Copy")).unwrap();

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
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let entry = &val["renumbered"][0];
    let new_id = entry["new_id"].as_str().unwrap().to_string();
    assert!(new_id.starts_with("AD-"), "got {new_id}");
    assert_eq!(
        entry["new_path"],
        format!("docs/specs/adrs/{}.md", new_id),
        "the renamed file must carry the new id: {entry}"
    );
    assert!(adrs.join(format!("{}.md", new_id)).is_file());
    assert!(!adrs.join("AD-1.md").exists());

    // Readable and writable under the new id, with no manual step — and the keeper survives.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["update", &new_id, "--status", "ready"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["get", "AD-1", "--json"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        val["findings"].as_array().unwrap().len(),
        0,
        "the workspace must be clean afterwards: {val}"
    );
}

// ---------------------------------------------------------------------------
// A sweep and a rebuild converge (spec-sweep-rebuild-convergence)
// ---------------------------------------------------------------------------

/// A story declaring `depends_on: [<target>]`, written directly so the relation is present at
/// the file's first hydration.
fn write_story_depending_on(dir: &Path, id: &str, target: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: draft
version: 1
relations:
  depends_on: ["{target}"]
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

#[test]
fn test_validate_reports_dangling_relation_after_the_target_file_is_deleted() {
    // The purge cascade used to delete the edge E1S2.md declares along with E1S1, leaving
    // nothing for `validate` to report — while `sync --rebuild` on the same tree reported it.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    write_story(&stories, "E1S1");
    write_story_depending_on(&stories, "E1S2", "E1S1");

    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success();

    fs::remove_file(stories.join("E1S1.md")).unwrap();

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["code"] == "dangling_relation" && f["path"] == "docs/specs/stories/E1S2.md"),
        "no dangling_relation reported without a rebuild: {val}"
    );
}

#[test]
fn test_get_still_resolves_the_survivor_after_the_cached_duplicate_is_deleted() {
    // The natural response to `duplicate_planning_id` is to delete the copy. When that is the
    // file the cache points at, the entity used to vanish while its twin sat on disk.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    write_story(&stories, "E1S1");
    fs::copy(stories.join("E1S1.md"), stories.join("E1S1-copy.md")).unwrap();

    // Boots once so the cache points at the sorted-last file, `E1S1.md`.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);

    fs::remove_file(stories.join("E1S1.md")).unwrap();

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["get", "E1S1", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        val["id"], "E1S1",
        "the entity must still resolve from the file still on disk: {val}"
    );
    assert_eq!(val["stale"], false);
}

#[test]
fn test_validate_reports_read_error_on_the_command_that_rebuilds_the_cache() {
    // `validate` used to exit 0 here: a rebuild truncated `findings` and swallowed the
    // unreadable file, so the command that triggered it saw an empty findings table while the
    // next command reported the file.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    write_story(&stories, "E1S1");
    let path = stories.join("E1S1.md");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    }
    if fs::read_to_string(&path).is_ok() {
        eprintln!("skipping: files are readable regardless of mode (running as root?)");
        return;
    }
    // No cache at all, so this command rebuilds rather than sweeps.
    fs::remove_dir_all(root.join(".qdev/cache")).unwrap();

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert!(
        val["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "read_error" && f["path"] == "docs/specs/stories/E1S1.md"),
        "the rebuild must report the unreadable file: {val}"
    );
}

#[test]
fn test_validate_derives_no_computed_finding_from_a_stale_row() {
    // A computed finding must not outlive the content it describes: the file no longer declares
    // `target_modules`, and a rebuild of the same tree reports only the parse failure.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    write_story(&stories, "E1S1");
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["update", "E1S1", "--field", "target_modules=[\"ghost\"]"])
        .assert()
        .success();

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert!(
        val["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["code"] == "target_module_not_registered"),
        "precondition: the finding is reported while the file still says it: {val}"
    );

    // Edit the module away and make the file unparseable in the same edit.
    let edited = read_story(&stories, "E1S1")
        .replace("target_modules:\n- ghost\n", "")
        .replace("target_modules: [\"ghost\"]\n", "")
        + "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> b\n";
    fs::write(stories.join("E1S1.md"), edited).unwrap();

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let findings = val["findings"].as_array().unwrap();
    assert!(
        findings.iter().any(|f| f["code"] == "merge_conflict"),
        "the actionable parse failure must be reported: {val}"
    );
    assert!(
        !findings
            .iter()
            .any(|f| f["code"] == "target_module_not_registered"),
        "no finding may be derived from the retained pre-edit row: {val}"
    );

    // Reads are untouched: the stale entity is still returned, flagged.
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["get", "E1S1", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["stale"], true, "get must still return the stale row");
}

/// The spec's own reproduction, end to end: a deferred-work row whose *origin story* is
/// merge-conflicted. The retained row is stale, so it is absent for the purpose of deriving a
/// finding and the orphan is reported — and reported identically by a sweep and by
/// `sync --rebuild` on the identical tree, which is the answer that used to depend on hydration
/// history.
#[test]
fn test_validate_orphan_deferred_work_agrees_after_sweep_and_rebuild_with_a_stale_origin() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories = root.join("docs/specs/stories");
    write_story(&stories, "E1S1");
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(
        dw_dir.join("DW-1111.md"),
        r#"---
id: DW-1111
title: "Deferred"
status: open
origin_story_id: E1S1
target_module: foundation
safety_risk: negligible
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
Body
"#,
    )
    .unwrap();

    // `(code, path, message)`, matching the core convergence test: a divergence confined to a
    // message is still a divergence, and comparing only code and path would miss it.
    let validate = |expect_failure: bool| -> Vec<(String, String, String)> {
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd.current_dir(root).args(["validate", "--json"]).assert();
        let assert = if expect_failure {
            assert.failure().code(1)
        } else {
            assert.success()
        };
        let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        let mut codes: Vec<(String, String, String)> = val["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    f["code"].as_str().unwrap().to_string(),
                    f["path"].as_str().unwrap().to_string(),
                    f["message"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect();
        codes.sort();
        codes
    };

    // Precondition: with the origin story parsing, this check reports nothing. Filtered to the
    // code under test rather than asserting the whole list is empty, which would couple this
    // test to every unrelated check's behaviour on the fixture.
    assert!(
        !validate(false)
            .iter()
            .any(|(code, _, _)| code == "orphan_deferred_work"),
        "precondition: a live origin story suppresses the orphan"
    );

    // Conflict the origin story. The next sweep retains its row, flagged stale.
    fs::write(
        stories.join("E1S1.md"),
        read_story(&stories, "E1S1") + "<<<<<<< HEAD\nx\n=======\ny\n>>>>>>> b\n",
    )
    .unwrap();

    let swept = validate(true);
    assert!(
        swept
            .iter()
            .any(|(code, path, _)| code == "orphan_deferred_work"
                && path == "docs/state/dw/DW-1111.md"),
        "a stale origin story must not suppress the orphan: {swept:?}"
    );

    // The rebuild has no row for that story at all, and must reach the same answer.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sync", "--rebuild", "--json"])
        .assert()
        .success();
    assert_eq!(
        swept,
        validate(true),
        "sweep and rebuild must report the same findings on the identical tree"
    );

    // And the derivation rule does not leak into reads.
    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["get", "DW-1111", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["id"], "DW-1111");
}

#[test]
fn test_validate_reports_duplicate_planning_id_in_state_dir() {
    // Duplicate-id detection must cover every directory hydration reads, not `specs_dir` alone.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let sprints = root.join("docs/state/sprints");
    fs::create_dir_all(&sprints).unwrap();
    let body = r#"---
id: sprint-1
title: "Sprint One"
status: active
version: 1
started_at: "2026-09-01T00:00:00Z"
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
Body
"#;
    fs::write(sprints.join("sprint-1.md"), body).unwrap();
    fs::write(sprints.join("sprint-1-copy.md"), body).unwrap();

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let dupes: Vec<&Value> = val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"] == "duplicate_planning_id")
        .collect();
    assert_eq!(
        dupes.len(),
        2,
        "one finding per participating path in state_dir: {val}"
    );
}

/// Widening the duplicate scan to `state_dir` made a `--fix-ids` branch reachable that nothing
/// exercised: a sprint / deferred-work / decision id is not sequentially renumberable, so the
/// group is reported and skipped rather than repaired. In JSON mode that must still be exactly
/// one parseable document — the branch takes care not to `emit_error` for this reason.
#[test]
fn test_fix_ids_skips_a_duplicate_it_cannot_renumber() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let sprints = root.join("docs/state/sprints");
    fs::create_dir_all(&sprints).unwrap();
    let sprint = r#"---
id: sprint-1
title: First sprint
status: planning
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Goal
- Ship.
"#;
    fs::write(sprints.join("sprint-1.md"), sprint).unwrap();
    fs::write(sprints.join("sprint-1-copy.md"), sprint).unwrap();

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
    let stdout = assert.get_output().stdout.clone();

    // Exactly one JSON document, whatever the outcome.
    let val: Value = serde_json::from_slice(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be one JSON document: {e}\n{stdout:?}"));
    assert!(
        val["renumbered"].as_array().unwrap().is_empty(),
        "a non-renumberable id must not be renumbered: {val}"
    );
    assert_eq!(
        val["skipped"].as_array().unwrap().len(),
        1,
        "the duplicate that could not be repaired must be reported as skipped: {val}"
    );

    // Both files are untouched, and the duplicate is still reported.
    assert_eq!(
        fs::read_to_string(sprints.join("sprint-1.md")).unwrap(),
        sprint
    );
    assert_eq!(
        fs::read_to_string(sprints.join("sprint-1-copy.md")).unwrap(),
        sprint
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let codes: Vec<&str> = val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert!(
        codes.contains(&"duplicate_planning_id"),
        "the unrepaired duplicate must still be reported: {codes:?}"
    );
}

// ---------------------------------------------------------------------------
// One rule for which ids are in use
// ---------------------------------------------------------------------------

/// Option A (Decision 2026-09-10, Simon): a duplicate whose file is not in its kind directory is
/// refused. Renumbering it in place would write a correctly named file in a directory no writer
/// resolves — a repair the run did not achieve — and moving a user's file is not a decision the
/// tool takes silently. So the entry is skipped with the expected path named, nothing is renamed,
/// and the duplicate is still reported afterwards.
#[test]
fn test_fix_ids_refuses_a_duplicate_outside_its_kind_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    let off_convention = stories_dir.join("sub/E1S1.md");
    write_story(&stories_dir.join("sub"), "E1S1");

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
    let out = assert.get_output();
    let val: Value = serde_json::from_slice(&out.stdout).unwrap();

    assert!(val["renumbered"].as_array().unwrap().is_empty(), "{val}");
    assert_eq!(
        val["skipped"].as_array().unwrap(),
        &vec![Value::from("docs/specs/stories/sub/E1S1.md")],
        "the refused entry must be reported as skipped: {val}"
    );
    // The expected path is named, so the refusal is actionable.
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("docs/specs/stories/E1S1.md"),
        "the expected path must be named: {stderr}"
    );

    // Nothing was renamed, in either directory.
    assert!(off_convention.is_file());
    assert!(read_story(&stories_dir, "E1S1").contains("id: E1S1\n"));
    assert!(!stories_dir.join("E1S2.md").exists());
    assert!(!stories_dir.join("sub/E1S2.md").exists());

    // And the duplicate is still reported, rather than claimed as repaired.
    let mut after = Command::cargo_bin("qdev").unwrap();
    let after_assert = after
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let after_val: Value = serde_json::from_slice(&after_assert.get_output().stdout).unwrap();
    let dups: Vec<&Value> = after_val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"] == "duplicate_planning_id")
        .collect();
    assert_eq!(dups.len(), 2, "{after_val}");
}

/// A duplicate whose file *is* in its kind directory is still repaired — the refusal above is
/// scoped to the directory half of the convention, not a blanket decline.
#[test]
fn test_fix_ids_still_repairs_a_duplicate_in_its_kind_directory() {
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
    assert_eq!(renumbered.len(), 1, "{val}");
    assert_eq!(renumbered[0]["old_id"], "E1S1");
    assert_eq!(renumbered[0]["new_id"], "E1S2");

    // The two allocators agree: `create story` asks the same `ids_in_use` function `--fix-ids`
    // just allocated from, so it cannot hand back the id that renumber has taken.
    let mut create = Command::cargo_bin("qdev").unwrap();
    let create_assert = create
        .current_dir(root)
        .args(["create", "story", "E1", "--json"])
        .assert()
        .success();
    let create_val: Value = serde_json::from_slice(&create_assert.get_output().stdout).unwrap();
    assert_eq!(create_val["id"], "E1S3", "{create_val}");
}

/// A `.MD` file whose name *does* carry its id is a legal name, not a repairable defect: the
/// extension is matched case-insensitively by the identity rule, so the write path resolves it
/// on every host and no warning fires for it. The warning used to fire here while the write path
/// resolved the file anyway on macOS — a claim that was false on the platform it was read on.
#[test]
fn test_uppercase_md_extension_carries_its_id_and_draws_no_warning() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("E1S9.MD"),
        r#"---
id: E1S9
title: "Story E1S9"
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
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let off: Vec<&Value> = val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"] == "entity_file_off_convention")
        .collect();
    assert!(off.is_empty(), "{val}");

    // And the write path agrees: the edit lands in the file that exists, and no file under the
    // canonical spelling is conjured beside it.
    let mut update = Command::cargo_bin("qdev").unwrap();
    update
        .current_dir(root)
        .args(["update", "E1S9", "--status", "ready", "--json"])
        .assert()
        .success();
    assert!(fs::read_to_string(stories_dir.join("E1S9.MD"))
        .unwrap()
        .contains("status: ready"));

    // `relate` reports the path the file actually has, not a spelling no file carries — which
    // is also the path it writes to the cache.
    write_story(&stories_dir, "E1S8");
    let mut relate = Command::cargo_bin("qdev").unwrap();
    let relate_assert = relate
        .current_dir(root)
        .args(["relate", "E1S9", "depends_on", "E1S8"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&relate_assert.get_output().stdout).to_string();
    assert!(
        stdout.contains("docs/specs/stories/E1S9.MD"),
        "relate must report the real path: {stdout}"
    );
}

/// The extension's case is legal; the rest of the name still has to carry the id. A `.MD` file
/// named for nothing draws the warning exactly as its `.md` twin does — the gate decides which
/// files the convention applies to, not what the convention says.
#[test]
fn test_off_convention_warning_covers_uppercase_md_extension() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(
        stories_dir.join("notes.MD"),
        r#"---
id: E1S9
title: "Story E1S9"
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
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let off: Vec<&Value> = val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["code"] == "entity_file_off_convention")
        .collect();
    assert_eq!(off.len(), 1, "{val}");
    assert_eq!(off[0]["severity"], "warning");
    assert_eq!(off[0]["path"], "docs/specs/stories/notes.MD");
    assert!(off[0]["message"]
        .as_str()
        .unwrap()
        .contains("docs/specs/stories/E1S9.md"));
}

/// The keeper is chosen by sort order, which says nothing about the identity rule — so an
/// off-convention *keeper* escaped the refusal entirely: the group's other files were renumbered,
/// the run reported a successful repair, and the id was left owned by a file no writer can
/// resolve. `Archive/` sorts before `E1S1.md`, which is the ordering the first version of this
/// suite could not produce.
#[test]
fn test_fix_ids_refuses_a_group_whose_keeper_is_outside_its_kind_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let stories_dir = root.join("docs/specs/stories");
    let archive = stories_dir.join("Archive");
    fs::create_dir_all(&archive).unwrap();
    write_story(&archive, "E1S1");
    write_story(&stories_dir, "E1S1");

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

    assert!(
        val["renumbered"].as_array().unwrap().is_empty(),
        "a group whose keeper cannot be resolved must not be half-repaired: {val}"
    );
    assert_eq!(
        val["skipped"].as_array().unwrap().len(),
        2,
        "both the keeper and the candidate belong in skipped: {val}"
    );

    // Nothing was renamed, and the duplicate is still reported.
    assert!(archive.join("E1S1.md").is_file());
    assert!(stories_dir.join("E1S1.md").is_file());
    assert!(!stories_dir.join("E1S2.md").exists());

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let codes: Vec<&str> = val["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap())
        .collect();
    assert!(
        codes.contains(&"duplicate_planning_id"),
        "the unrepaired duplicate must still be reported: {codes:?}"
    );
}
