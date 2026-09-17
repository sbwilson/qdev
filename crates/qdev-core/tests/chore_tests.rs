//! Unit tests for the Story 2.10 chore module: allowlist derivation, glob selection over real
//! `git status` output, the `DEC-` record, the decided edge cases (D-1 … D-5), and the ways a
//! chore that never gets committed can still be settled.

use std::fs;
use std::path::Path;

use qdev_core::chore::{
    abort_chore, chore_dir, close_chore, commit_chore, derive_chore_id, find_open_chore,
    list_chore_records, start_chore, CommitChoreInput, FinishChoreInput, StartChoreInput,
};
use qdev_core::config::StorageConfig;
use qdev_core::decision::{log_decision, DecisionInput};
use qdev_core::write::Author;
use tempfile::TempDir;

fn author() -> Author {
    Author::new("human", "simon")
}

/// Runs git with the workspace as cwd, failing loudly: every chore test depends on real git
/// behaviour, so a git that will not run is a broken fixture, not a skipped case.
fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
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

/// A git repository with a committed base, an initialised qdev workspace, and the two files the
/// story's acceptance criteria use.
fn setup_workspace(root: &Path) {
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/state/decisions")).unwrap();
    fs::create_dir_all(root.join(".qdev/chores")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::write(
        root.join("qdev.toml"),
        "[project]\nname = \"TestProject\"\n\n[identity]\ndeveloper_id = \"simon\"\nteams = [\"core-platform\"]\n\n[storage]\nspecs_dir = \"docs/specs\"\nstate_dir = \"docs/state\"\ncache_dir = \".qdev/cache\"\n",
    )
    .unwrap();
    qdev_core::ensure_cache(root, &StorageConfig::default()).unwrap();

    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    fs::write(root.join("README.md"), "# Readme\n\nFix this typo later.\n").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
}

fn start(root: &Path, title: &str, paths: &[&str]) -> qdev_core::ChoreRecord {
    start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        title: title.to_string(),
        paths: paths.iter().map(|p| p.to_string()).collect(),
        author: author(),
        alongside: false,
    })
    .unwrap_or_else(|e| panic!("start_chore('{}') failed: {}", title, e))
}

fn commit(root: &Path, strict: bool) -> Result<qdev_core::ChoreCommitResult, qdev_core::QdevError> {
    commit_chore(&CommitChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        author: author(),
        strict,
    })
}

/// Settles the open chore: `abort` when `abandon` is set, `close` otherwise.
fn finish(
    root: &Path,
    abandon: bool,
    reason: Option<&str>,
) -> Result<qdev_core::ChoreRecord, qdev_core::QdevError> {
    let storage = StorageConfig::default();
    let input = FinishChoreInput {
        workspace_root: root,
        storage: Some(&storage),
        reason: reason.map(|r| r.to_string()),
        author: author(),
    };
    if abandon {
        abort_chore(&input)
    } else {
        close_chore(&input)
    }
}

// ---------------------------------------------------------------------------
// Identifier derivation
// ---------------------------------------------------------------------------

#[test]
fn derive_id_slugifies_the_title() {
    assert_eq!(
        derive_chore_id("fix readme typo"),
        "chore-fix-readme-typo",
        "spaces become single dashes"
    );
    assert_eq!(
        derive_chore_id("Fix  README -- TYPO!"),
        "chore-fix-readme-typo",
        "case folds and repeated separators collapse"
    );
    assert_eq!(
        derive_chore_id("!!!"),
        "chore-untitled",
        "an all-separator title still yields a usable id"
    );
}

#[test]
fn a_long_title_is_capped_and_still_yields_a_usable_id() {
    // `derive_chore_id`'s doc promises a cap; without it a 300-char title fails with
    // `io_error: File name too long`.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let long = start(
        root,
        &format!("fix {}", "thing ".repeat(60)),
        &["README.md"],
    );
    assert!(
        long.id.chars().count() <= "chore-".len() + 60,
        "a long title must not overflow the record file name: {}",
        long.id
    );
    assert_ne!(
        long.id, "chore-untitled",
        "a long title is not an empty one"
    );
    assert!(root.join(format!(".qdev/chores/{}.json", long.id)).exists());
}

#[test]
fn a_title_in_another_script_keeps_its_letters() {
    // Dropping every non-ASCII character would collapse all such titles onto one
    // `chore-untitled` that identifies nothing.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let record = start(root, "修復錯字", &["README.md"]);
    assert_eq!(record.id, "chore-修復錯字");
    assert!(root.join(".qdev/chores/chore-修復錯字.json").exists());
}

// ---------------------------------------------------------------------------
// Starting a chore
// ---------------------------------------------------------------------------

#[test]
fn start_without_paths_is_a_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let err = start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        title: "no paths".to_string(),
        paths: vec![],
        author: author(),
        alongside: false,
    })
    .unwrap_err();
    assert_eq!(err.code(), "paths_required");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
    assert_eq!(
        find_open_chore(root, Some(&StorageConfig::default())).unwrap(),
        None,
        "a refused start must record nothing"
    );
}

#[test]
fn repeated_globs_are_deduplicated_in_the_record() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let record = start(
        root,
        "fix readme typo",
        &["README.md", "README.md", " docs/** "],
    );
    assert_eq!(
        record.paths,
        vec!["README.md".to_string(), "docs/**".to_string()],
        "a path listed twice appears once, and surrounding spaces are trimmed"
    );
}

#[test]
fn second_start_while_one_is_open_is_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "first", &["README.md"]);
    let err = start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        title: "second".to_string(),
        paths: vec!["docs/**".to_string()],
        author: author(),
        alongside: false,
    })
    .unwrap_err();
    assert_eq!(err.code(), "chore_in_progress");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::Conflict);
}

// ---------------------------------------------------------------------------
// D-1 / D-5 — where the record lives and what gets committed
// ---------------------------------------------------------------------------

#[test]
fn commit_includes_the_decision_record_even_though_it_is_out_of_the_allowlist() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    fs::write(root.join("docs/guide.md"), "tweak\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "add guide"]);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    let result = commit(root, false).unwrap();

    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("README.md"),
        "the allowlisted change must be committed:\n{}",
        files
    );
    assert!(
        files.contains("docs/state/decisions/"),
        "the DEC record must be committed even though only README.md is declared:\n{}",
        files
    );
    assert_eq!(result.included, vec!["README.md".to_string()]);
    assert!(
        !result
            .excluded
            .iter()
            .any(|e| e.path.starts_with("docs/state")),
        "the DEC record must not count as an out-of-allowlist change: {:?}",
        result.excluded
    );
}

// ---------------------------------------------------------------------------
// Allowlist selection over git status
// ---------------------------------------------------------------------------

#[test]
fn commit_stages_only_the_allowlisted_change_and_reports_the_rest() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md", "docs/**"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let result = commit(root, false).unwrap();
    assert_eq!(result.included, vec!["README.md".to_string()]);
    assert_eq!(
        result
            .excluded
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>(),
        vec!["src/main.rs"],
        "the out-of-allowlist edit is reported, not committed"
    );

    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(
        files
            .lines()
            .filter(|l| !l.starts_with("docs/state"))
            .collect::<Vec<_>>(),
        vec!["README.md"],
        "only the allowlisted change plus the DEC record may be committed"
    );
    // The excluded edit survives in the working tree, exactly as promised.
    assert_eq!(
        fs::read_to_string(root.join("src/main.rs")).unwrap(),
        "fn main() { changed() }\n"
    );
}

#[test]
fn untracked_file_under_the_allowlist_is_committed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "add a note", &["docs/**"]);
    fs::write(root.join("docs/new.md"), "new\n").unwrap();

    let result = commit(root, false).unwrap();
    assert_eq!(result.included, vec!["docs/new.md".to_string()]);
    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(files.contains("docs/new.md"), "got:\n{}", files);
}

#[test]
fn plain_directory_pattern_covers_everything_beneath_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "docs", &["docs"]);
    fs::create_dir_all(root.join("docs/deep/nested")).unwrap();
    fs::write(root.join("docs/deep/nested/x.md"), "x\n").unwrap();

    let result = commit(root, false).unwrap();
    assert_eq!(result.included, vec!["docs/deep/nested/x.md".to_string()]);
}

// ---------------------------------------------------------------------------
// D-3 — nothing under the allowlist changed
// ---------------------------------------------------------------------------

#[test]
fn nothing_under_the_allowlist_changed_fails_and_records_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let err = commit(root, false).unwrap_err();
    assert_eq!(err.code(), "nothing_to_commit");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::LogicalFailure);

    let records = list_chore_records(root, Some(&StorageConfig::default())).unwrap();
    assert_eq!(records.len(), 1, "the chore record stays for the retry");
    let decisions = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
        .count();
    assert_eq!(
        decisions, 0,
        "a chore that committed nothing must write no DEC"
    );
}

#[test]
fn strict_refuses_when_anything_sits_outside_the_allowlist() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let err = commit(root, true).unwrap_err();
    assert_eq!(err.code(), "out_of_allowlist");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::PolicyRefusal);
    assert_eq!(
        git(root, &["log", "--oneline", "-1"])
            .split_whitespace()
            .nth(1)
            .unwrap(),
        "base",
        "HEAD must still be the base commit"
    );
}

// ---------------------------------------------------------------------------
// D-4 — a change already staged before the command runs
// ---------------------------------------------------------------------------

#[test]
fn pre_staged_out_of_allowlist_change_survives_and_is_reported() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();
    git(root, &["add", "src/main.rs"]);

    let result = commit(root, false).unwrap();
    let staged = result
        .excluded
        .iter()
        .find(|e| e.path == "src/main.rs")
        .expect("the staged change must be reported");
    assert!(staged.staged, "and must be flagged as still staged");

    let status = git(root, &["status", "--porcelain"]);
    assert!(
        status.contains("M  src/main.rs"),
        "the index must be left alone, got:\n{}",
        status
    );
}

// ---------------------------------------------------------------------------
// D-4 — committing closes the chore
// ---------------------------------------------------------------------------

#[test]
fn commit_closes_the_chore_so_rerun_reports_no_active_chore() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    commit(root, false).unwrap();

    let err = commit(root, false).unwrap_err();
    assert_eq!(err.code(), "no_active_chore");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);

    let open = find_open_chore(root, Some(&StorageConfig::default())).unwrap();
    assert!(open.is_none(), "the record must be closed, not open");
    let records = list_chore_records(root, Some(&StorageConfig::default())).unwrap();
    assert_eq!(records[0].status, "committed");
    assert!(records[0].commit.is_some(), "the sha is recorded");
}

// ---------------------------------------------------------------------------
// Decision record
// ---------------------------------------------------------------------------

#[test]
fn the_chore_is_recorded_as_a_human_ruling_titled_chore() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md", "docs/**"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    let result = commit(root, false).unwrap();

    let path = root.join(result.decision_path.as_ref().unwrap());
    let content = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {}", path.display(), e));
    assert!(
        content.contains("decision_type: human_ruling"),
        "the record must be a human ruling:\n{}",
        content
    );
    assert!(
        content.contains("topic: chore"),
        "the record's topic must be `chore`:\n{}",
        content
    );
    assert!(
        content.contains("Committed README.md under allowlist: README.md, docs/**"),
        "the ruling must name the paths:\n{}",
        content
    );
    assert!(result.decision_id.as_ref().unwrap().starts_with("DEC-"));
}

#[test]
fn logging_a_decision_for_a_missing_subject_still_works_when_not_validated() {
    // Guards the choice to call `log_decision` with `validate_subject: false`: a chore id is
    // not an entity, so the record must be written without resolving a subject.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let payload = log_decision(
        root,
        Some(&StorageConfig::default()),
        &DecisionInput {
            subject_id: "chore-fix-readme-typo".to_string(),
            decision_type: "human_ruling".to_string(),
            topic: Some("chore".to_string()),
            context: None,
            ruling: "Committed README.md".to_string(),
            author: author(),
            title: None,
            timestamp: None,
            validate_subject: false,
        },
    )
    .unwrap();
    assert!(payload.id.starts_with("DEC-"));
    assert!(
        root.join(&payload.path).exists(),
        "the record must exist at {}",
        payload.path
    );
}

#[test]
fn a_deleted_file_under_the_allowlist_is_committed_as_a_deletion() {
    // D-3 lists "an allowlisted file was deleted" as a case that must commit, not stall.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "drop the guide", &["docs/**"]);
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(
        root.join("docs/guide.md"),
        "tweak
",
    )
    .unwrap();
    git(root, &["add", "docs/guide.md"]);
    git(root, &["commit", "-m", "add guide"]);

    fs::remove_file(root.join("docs/guide.md")).unwrap();
    let result = commit(root, false).unwrap();
    assert_eq!(result.included, vec!["docs/guide.md".to_string()]);
    assert!(
        git(root, &["show", "--name-only", "--format=", "HEAD"]).contains("docs/guide.md"),
        "the deletion must be committed"
    );
    assert!(!root.join("docs/guide.md").exists());
}

// ---------------------------------------------------------------------------
// Identifier collisions
// ---------------------------------------------------------------------------

#[test]
fn a_reused_title_takes_the_next_free_id_and_keeps_the_old_record() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    commit(root, false).unwrap();

    // Same title again: the committed record stays as history, the new one takes `-2`.
    let again = start(root, "fix readme typo", &["README.md"]);
    assert_eq!(again.id, "chore-fix-readme-typo-2");
    let ids: Vec<String> = list_chore_records(root, Some(&StorageConfig::default()))
        .unwrap()
        .iter()
        .map(|r| r.id.clone())
        .collect();
    assert_eq!(
        ids,
        vec![
            "chore-fix-readme-typo".to_string(),
            "chore-fix-readme-typo-2".to_string()
        ],
        "the first record must still be there, closed"
    );
    assert_eq!(
        list_chore_records(root, Some(&StorageConfig::default())).unwrap()[0].status,
        "committed",
        "and must not have been reopened"
    );
}

// ---------------------------------------------------------------------------
// Lease interlock
// ---------------------------------------------------------------------------

#[test]
fn start_is_refused_while_a_story_lease_is_held() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    fs::create_dir_all(root.join(".qdev/leases")).unwrap();
    fs::write(
        root.join(".qdev/leases/E12S4.json"),
        r#"{"story_id":"E12S4","holder":"agent-1","author_type":"agent","worktree_path":"/tmp/x","branch":"feature/x","started_at":"2026-09-17T10:00:00Z","session_token":"qs_E12S4_ab12"}"#,
    )
    .unwrap();

    let err = start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        title: "typo".to_string(),
        paths: vec!["README.md".to_string()],
        author: author(),
        alongside: false,
    })
    .unwrap_err();
    assert_eq!(err.code(), "lease_held");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::PolicyRefusal);
    assert!(err.message.contains("E12S4") && err.message.contains("agent-1"));
    assert!(
        !root.join(".qdev/chores/chore-typo.json").exists(),
        "a refused start must record no chore"
    );
}

#[test]
fn alongside_allows_a_chore_next_to_a_story_lease() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    fs::create_dir_all(root.join(".qdev/leases")).unwrap();
    fs::write(
        root.join(".qdev/leases/E12S4.json"),
        r#"{"story_id":"E12S4","holder":"agent-1","author_type":"agent","worktree_path":"/tmp/x","branch":"feature/x","started_at":"2026-09-17T10:00:00Z","session_token":"qs_E12S4_ab12"}"#,
    )
    .unwrap();

    let record = start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        title: "typo".to_string(),
        paths: vec!["README.md".to_string()],
        author: author(),
        alongside: true,
    })
    .unwrap();
    assert_eq!(record.status, "open");
    // The lease is untouched.
    assert!(root.join(".qdev/leases/E12S4.json").exists());
    assert!(fs::read_to_string(root.join(".qdev/leases/E12S4.json"))
        .unwrap()
        .contains("agent-1"));
}

#[test]
fn commit_touches_no_story_state() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        "---\nid: E12S4\ntitle: \"Test\"\nstatus: in-progress\nversion: 1\n---\n\nbody\n",
    )
    .unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "add story"]);

    start(root, "fix readme typo", &["README.md"]);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    commit(root, false).unwrap();

    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        !files.contains("E12S4"),
        "no story file may be committed by a chore:\n{}",
        files
    );
    assert!(fs::read_to_string(root.join("docs/specs/stories/E12S4.md"))
        .unwrap()
        .contains("status: in-progress"));
}

// ---------------------------------------------------------------------------
// Settling a chore that is never going to be committed
// ---------------------------------------------------------------------------

#[test]
fn closing_a_chore_frees_the_slot_that_nothing_to_commit_had_wedged() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // The dead end `close` exists for: an allowlist that matches nothing, so `commit` refuses
    // and — while the record stays open — every later `start` is refused too.
    start(root, "docs left untouched", &["docs/ghost.*"]);
    assert_eq!(commit(root, false).unwrap_err().code(), "nothing_to_commit");
    let stuck = start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&StorageConfig::default()),
        title: "second attempt".to_string(),
        paths: vec!["README.md".to_string()],
        author: author(),
        alongside: false,
    })
    .unwrap_err();
    assert_eq!(stuck.code(), "chore_in_progress");

    let settled = finish(root, false, Some("superseded by a real story")).unwrap();
    assert_eq!(settled.status, "closed");
    assert_eq!(
        settled.reason.as_deref(),
        Some("superseded by a real story")
    );

    // The slot is free again, and the settled record is still listed — as history, not as a
    // wedge.
    let next = start(root, "second attempt", &["README.md"]);
    assert_eq!(next.id, "chore-second-attempt");

    let records = list_chore_records(root, Some(&StorageConfig::default())).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].status, "closed");
    assert_eq!(records[1].status, "open");
    assert!(find_open_chore(root, Some(&StorageConfig::default()))
        .unwrap()
        .is_some_and(|r| r.id == "chore-second-attempt"));
}

#[test]
fn aborting_a_chore_records_it_as_abandoned() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "chase a flaky test", &["tests/**"]);

    let settled = finish(root, true, Some("not worth the time")).unwrap();
    assert_eq!(settled.status, "abandoned");
    assert_eq!(settled.reason.as_deref(), Some("not worth the time"));
    assert!(
        settled.closed_at.is_some(),
        "when it was settled is recorded"
    );
    assert_eq!(
        settled.closed_by,
        Some(author()),
        "and who settled it, which is usually not whoever started it"
    );

    // A different outcome from `close`, and the same way out of the wedge.
    assert_eq!(
        list_chore_records(root, Some(&StorageConfig::default())).unwrap()[0].status,
        "abandoned"
    );
    assert_eq!(
        start(root, "next small thing", &["README.md"]).id,
        "chore-next-small-thing"
    );
}

#[test]
fn a_settled_chore_cannot_be_settled_again() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    start(root, "fix readme typo", &["README.md"]);
    let settled = finish(root, true, None).unwrap();
    assert!(
        settled.reason.is_none(),
        "no reason given is recorded as none, not as a blank excuse"
    );

    // Nothing is open any more: the same code and exit `commit` gives once the chore is
    // committed, so a repeated `close` is not mistaken for a second chore.
    let err = finish(root, false, None).unwrap_err();
    assert_eq!(err.code(), "no_active_chore");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
}

#[test]
fn records_follow_the_configured_cache_dir() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // A workspace that relocated its cache: `qdev init` creates and gitignores `var/chores`,
    // so that is where the record has to land. Writing it to the hardcoded `.qdev/chores`
    // would put it in git-visible space, in a directory nothing created.
    let relocated = StorageConfig {
        cache_dir: "var/qdev-cache".to_string(),
        ..StorageConfig::default()
    };
    assert_eq!(chore_dir(root, Some(&relocated)), root.join("var/chores"));

    let record = start_chore(&StartChoreInput {
        workspace_root: root,
        storage: Some(&relocated),
        title: "relocated record".to_string(),
        paths: vec!["docs/**".to_string()],
        author: author(),
        alongside: false,
    })
    .unwrap();

    assert!(root
        .join("var/chores")
        .join(format!("{}.json", record.id))
        .exists());
    assert!(
        !root
            .join(".qdev/chores")
            .join(format!("{}.json", record.id))
            .exists(),
        "and not in the default directory on top of it"
    );
    assert_eq!(
        find_open_chore(root, Some(&relocated))
            .unwrap()
            .map(|r| r.id),
        Some(record.id.clone()),
        "listing and commit-time lookup must read the configured directory"
    );
    assert!(list_chore_records(root, None)
        .unwrap()
        .iter()
        .all(|r| r.id != record.id));
}
