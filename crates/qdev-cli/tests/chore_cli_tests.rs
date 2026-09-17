//! End-to-end tests for `qdev chore start` / `qdev chore commit` (Story 2.10). Every run is a
//! real command against a real git repository: the whole story is about what actually gets
//! committed, so nothing here is mocked.

use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use serde_json::{json, Value};
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

    // A git repository with identity configured and signing off: without it `git commit`
    // fails on machines with `commit.gpgSign = true`.
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    fs::write(root.join("README.md"), "# Readme\n\nFix this typo.\n").unwrap();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(root.join("docs/guide.md"), "Guide.\n").unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);
}

/// Runs git in the workspace, panicking on failure: a git that will not run is a broken
/// fixture, not a skipped case.
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

fn qdev(root: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(args);
    cmd.output().expect("qdev should run")
}

fn json_of(root: &Path, args: &[&str]) -> (i32, Value) {
    let out = qdev(root, args);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out
        .status
        .code()
        .unwrap_or_else(|| panic!("qdev {:?} was killed by a signal", args));
    let value: Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout was not JSON for {:?}: {}\n{}", args, e, stdout));
    (code, value)
}

fn validate_against_schema(schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("schema must compile");
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "instance failed schema validation: {:?}",
        errors
    );
}

/// Loads a payload schema the way the CLI advertises it, so a schema that is registered but
/// unreadable (or the reverse) fails here rather than silently.
fn load_schema(root: &Path, name: &str) -> Value {
    let out = qdev(root, &["schema", "payload", name, "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(
        out.status.code(),
        Some(0),
        "`qdev schema payload {}` failed: {}",
        name,
        stdout
    );
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("schema payload {} was not JSON: {}\n{}", name, e, stdout))
}

/// Runs qdev and returns (exit code, combined stdout+stderr) — text-mode errors are written to
/// stderr per AD-13, so a check that only reads stdout would miss every error assertion.
fn run(root: &Path, args: &[&str]) -> (i32, String) {
    let out = qdev(root, args);
    let mut combined = String::from_utf8_lossy(&out.stdout).to_string();
    combined.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.code().unwrap(), combined)
}

// ---------------------------------------------------------------------------
// Story 2.10 acceptance criterion 1
// ---------------------------------------------------------------------------

#[test]
fn records_a_chore_with_path_globs_and_a_title() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let out = qdev(
        root,
        &[
            "chore",
            "start",
            "fix readme typo",
            "--paths",
            "README.md",
            "--json",
        ],
    );
    let code = out.status.code().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(code, 0, "stdout: {}", stdout);

    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["id"], json!("chore-fix-readme-typo"));
    assert_eq!(value["title"], json!("fix readme typo"));
    assert_eq!(value["paths"], json!(["README.md"]));
    assert_eq!(value["status"], json!("open"));

    // D-1: the record is written to the gitignored .qdev/chores/ directory.
    let record = root.join(".qdev/chores/chore-fix-readme-typo.json");
    assert!(
        record.exists(),
        "the record must live at {}",
        record.display()
    );
    assert!(
        !git(root, &["status", "--porcelain"]).contains(".qdev/chores"),
        "the record directory must stay gitignored"
    );

    // ...and round-trips against the payload schema.
    validate_against_schema(&load_schema(root, "chore"), &value);
}

// ---------------------------------------------------------------------------
// Acceptance criterion 2
// ---------------------------------------------------------------------------

#[test]
fn stages_and_commits_only_the_allowlisted_change() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    qdev(
        root,
        &["chore", "start", "fix readme typo", "--paths", "README.md"],
    );
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let (code, stdout) = run(root, &["chore", "commit"]);
    assert_eq!(code, 0, "stdout: {}", stdout);
    assert!(
        stdout.contains("NOT INCLUDED — not staged: src/main.rs"),
        "the excluded path must be printed, and labelled as the unstaged edit it is:\n{}",
        stdout
    );

    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("README.md") && !files.contains("src/main.rs"),
        "only the allowlisted change may be committed:\n{}",
        files
    );
    assert_eq!(
        git(root, &["log", "-1", "--format=%s"]),
        "chore: fix readme typo",
        "the commit message is `chore: <title>`"
    );
    assert!(
        git(root, &["status", "--porcelain"]).contains("src/main.rs"),
        "the out-of-allowlist change must still be there afterwards"
    );
}

// ---------------------------------------------------------------------------
// Acceptance criterion 3
// ---------------------------------------------------------------------------

#[test]
fn prints_that_nothing_under_the_allowlist_changed() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(
        root,
        &["chore", "start", "fix readme typo", "--paths", "README.md"],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let (code, stdout) = run(root, &["chore", "commit"]);
    assert_eq!(code, 1, "D-3: exit 1, not 3");
    assert!(
        stdout.contains("Nothing under the declared allowlist"),
        "the explanation must be printed:\n{}",
        stdout
    );
    assert_eq!(
        git(root, &["log", "--oneline"]).lines().count(),
        1,
        "a base commit and nothing else"
    );
}

// ---------------------------------------------------------------------------
// Acceptance criteria 4 & 5: no active chore
// ---------------------------------------------------------------------------

#[test]
fn commit_without_an_active_chore_is_a_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(root, &["chore", "commit"]);
    assert_eq!(code, 2);
    assert!(stdout.contains("no_active_chore"), "got: {}", stdout);

    // D-4: and again right after a successful commit, because the chore is closed.
    let (code, stdout) = run(
        root,
        &["chore", "start", "fix readme typo", "--paths", "README.md"],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    let (code, stdout) = run(root, &["chore", "commit"]);
    assert_eq!(code, 0, "got: {}", stdout);

    let (code, stdout) = run(root, &["chore", "commit"]);
    assert_eq!(code, 2);
    assert!(
        stdout.contains("no_active_chore"),
        "a committed chore must not look still open:\n{}",
        stdout
    );
}

// ---------------------------------------------------------------------------
// Acceptance criterion 6 / D-5: the decision record
// ---------------------------------------------------------------------------

#[test]
fn records_the_chore_as_a_decision_with_paths_and_author() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "start",
            "fix readme typo",
            "--paths",
            "README.md",
            "--author-id",
            "simon",
            "--json",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    let (code, stdout) = run(root, &["chore", "commit", "--json"]);
    assert_eq!(code, 0, "got: {}", stdout);
    let value: Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{}\n{}", e, stdout));

    let decision_id = value["decision_id"].as_str().unwrap().to_string();
    assert!(decision_id.starts_with("DEC-"));

    // D-5: the record sits outside the allowlist and is still committed, once.
    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(
        files
            .lines()
            .filter(|l| l.starts_with("docs/state/decisions/"))
            .count(),
        1,
        "the DEC record must be in the commit:\n{}",
        files
    );

    let path = fs::read_dir(root.join("docs/state/decisions"))
        .unwrap()
        .find_map(|e| {
            let e = e.unwrap();
            (e.file_name().to_string_lossy() == format!("{}.md", decision_id)).then(|| e.path())
        })
        .expect("the DEC record must exist");
    let content = fs::read_to_string(&path).unwrap();
    // The shape `log_decision` writes: the routing fields we chose, and the ruling.
    assert!(
        content.contains("decision_type: human_ruling"),
        "{}",
        content
    );
    assert!(content.contains("topic: chore"), "{}", content);
    assert!(
        content.contains("subject_id: chore-fix-readme-typo"),
        "{}",
        content
    );
    assert!(
        content.contains("ruling: 'Committed README.md under allowlist: README.md'"),
        "{}",
        content
    );
    assert!(content.contains("  id: simon"), "{}", content);
    assert!(content.contains("  type: human"), "{}", content);
    assert!(
        content.contains("Chore: fix readme typo"),
        "the record must be titled `Chore: <title>`:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// D-4: work staged before the command ran
// ---------------------------------------------------------------------------

#[test]
fn work_staged_before_the_command_runs_is_not_silently_included() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    qdev(
        root,
        &["chore", "start", "fix readme typo", "--paths", "README.md"],
    );
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();
    git(root, &["add", "src/main.rs"]); // staged before `qdev chore commit` runs

    let out = qdev(root, &["chore", "commit"]);
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(out.status.code(), Some(0), "stdout: {}", stdout);
    assert!(
        stdout.contains("NOT INCLUDED — still staged: src/main.rs"),
        "the pre-staged change must be announced:\n{}",
        stdout
    );

    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        !files.contains("src/main.rs"),
        "it must not be committed:\n{}",
        files
    );

    // ...and it survives, still staged.
    let status = git(root, &["status", "--porcelain"]);
    assert!(
        status.contains("M  src/main.rs"),
        "the index must be left intact, got:\n{}",
        status
    );
}

// ---------------------------------------------------------------------------
// Strict mode
// ---------------------------------------------------------------------------

#[test]
fn strict_refuses_a_change_outside_the_allowlist() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    qdev(
        root,
        &["chore", "start", "fix readme typo", "--paths", "README.md"],
    );
    fs::write(root.join("README.md"), "# Readme\n\nFixed.\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let (code, stdout) = run(root, &["chore", "commit", "--strict"]);
    assert_eq!(code, 3, "got: {}", stdout);
    assert!(stdout.contains("out_of_allowlist"), "got: {}", stdout);
    assert!(
        git(root, &["status", "--porcelain"]).contains("src/main.rs"),
        "nothing may be committed in strict mode"
    );
}

// ---------------------------------------------------------------------------
// Matrix row: "Malformed/empty glob list"
// ---------------------------------------------------------------------------

#[test]
fn a_chore_start_with_no_paths_is_refused_with_paths_required() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // The matrix promises `paths_required`, so the flag must not be refused by clap before the
    // core check runs — the record is written by `start_chore`, which is where the code lives.
    let (code, stdout) = run(root, &["chore", "start", "t", "--json"]);
    assert_eq!(code, 2, "got: {}", stdout);
    assert!(
        stdout.contains("paths_required"),
        "expected `paths_required`, got: {}",
        stdout
    );

    let dir = root.join(".qdev/chores");
    let left = fs::read_dir(&dir).map(|rd| rd.count()).unwrap_or(0);
    assert_eq!(left, 0, "a refused start must record nothing");
}

// ---------------------------------------------------------------------------
// The other strict configurations, and the argument-order footgun
// ---------------------------------------------------------------------------

#[test]
fn strict_refuses_a_pre_staged_change_outside_the_allowlist() {
    // Matrix row "Already-staged out-of-allowlist change … `--strict` → exit 3 (D-4)".
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(root, &["chore", "start", "t", "--paths", "README.md"]);
    assert_eq!(code, 0, "got: {}", stdout);
    fs::write(root.join("README.md"), "# Fixed\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();
    git(root, &["add", "src/main.rs"]);

    let (code, stdout) = run(root, &["chore", "commit", "--strict"]);
    assert_eq!(
        code, 3,
        "an index-only change must still refuse: {}",
        stdout
    );
    assert!(stdout.contains("out_of_allowlist"), "got: {}", stdout);
    assert!(
        git(root, &["status", "--porcelain"]).contains("M  src/main.rs"),
        "the staged change must be left where it was"
    );
}

#[test]
fn strict_refuses_when_only_out_of_allowlist_paths_changed() {
    // The case the Implementation Notes record: strict is checked first, or the refusal the
    // user asked for would be swallowed by `nothing_to_commit`.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(root, &["chore", "start", "t", "--paths", "README.md"]);
    assert_eq!(code, 0, "got: {}", stdout);
    fs::write(root.join("src/main.rs"), "fn main() { changed() }\n").unwrap();

    let (code, stdout) = run(root, &["chore", "commit", "--strict"]);
    assert_eq!(code, 3, "got: {}", stdout);
    assert!(stdout.contains("out_of_allowlist"), "got: {}", stdout);
}

#[test]
fn strict_succeeds_when_everything_changed_is_under_the_allowlist() {
    // D-5: the DEC record must not count against the allowlist under `--strict`.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(root, &["chore", "start", "t", "--paths", "README.md"]);
    assert_eq!(code, 0, "got: {}", stdout);
    fs::write(root.join("README.md"), "# Fixed\n").unwrap();

    let (code, stdout) = run(root, &["chore", "commit", "--strict"]);
    assert_eq!(code, 0, "got: {}", stdout);
    let files = git(root, &["show", "--name-only", "--format=", "HEAD"]);
    let decided: Vec<&str> = files
        .lines()
        .filter(|l| l.starts_with("docs/state/decisions/"))
        .collect();
    assert_eq!(
        files,
        format!(
            "README.md\ndocs/state/decisions/{}.md",
            decided[0]
                .trim_start_matches("docs/state/decisions/")
                .trim_end_matches(".md")
        ),
        "the change and its DEC record, and nothing else"
    );
}

#[test]
fn a_rename_under_the_allowlist_is_committed_as_a_rename() {
    // Design Notes: "a renamed file cannot escape the allowlist by changing names".
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(
        root,
        &["chore", "start", "rename the guide", "--paths", "docs/**"],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    git(root, &["mv", "docs/guide.md", "docs/old-guide.md"]);

    let (code, stdout) = run(root, &["chore", "commit"]);
    assert_eq!(code, 0, "got: {}", stdout);
    let status = git(root, &["show", "--format=", "--name-status", "HEAD"]);
    let lines: Vec<&str> = status.lines().collect();
    assert_eq!(
        lines[0], "R100\tdocs/guide.md\tdocs/old-guide.md",
        "the rename must be committed as a rename:\n{}",
        status
    );
    assert!(
        lines[1].starts_with("A\tdocs/state/decisions/DEC-") && lines.len() == 2,
        "the change and its DEC record, and nothing else:\n{}",
        status
    );
}

#[test]
fn flags_may_come_before_the_title() {
    // `--paths` used to be open-ended, so it swallowed the positional title.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(root, &["chore", "start", "--paths", "README.md", "t"]);
    assert_eq!(code, 0, "flags before the title must work: {}", stdout);
}

// ---------------------------------------------------------------------------
// Lease interlock
// ---------------------------------------------------------------------------

#[test]
fn starting_a_chore_while_a_story_lease_is_held_needs_alongside() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    fs::create_dir_all(root.join(".qdev/leases")).unwrap();
    fs::write(
        root.join(".qdev/leases/E12S4.json"),
        r#"{"story_id":"E12S4","holder":"agent-1","author_type":"agent","worktree_path":"/tmp/x","branch":"feature/x","started_at":"2026-09-17T10:00:00Z","session_token":"qs_E12S4_ab12"}"#,
    )
    .unwrap();

    let (code, stdout) = run(root, &["chore", "start", "typo", "--paths", "README.md"]);
    assert_eq!(code, 3);
    assert!(stdout.contains("lease_held"), "got: {}", stdout);
    assert!(!root.join(".qdev/chores/chore-typo.json").exists());

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "start",
            "typo",
            "--paths",
            "README.md",
            "--alongside",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    assert!(root.join(".qdev/chores/chore-typo.json").exists());
    // The lease is untouched.
    assert!(git(root, &["status", "--porcelain"]).is_empty());
}

// ---------------------------------------------------------------------------
// Repeated and wildcard patterns
// ---------------------------------------------------------------------------

#[test]
fn repeated_globs_and_directory_patterns_work() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "start",
            "docs tidy",
            "--paths",
            "docs/**",
            "--paths",
            "docs/**",
            "--json",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        value["paths"],
        json!(["docs/**"]),
        "a pattern given twice appears once"
    );

    fs::create_dir_all(root.join("docs/specs/adr")).unwrap();
    fs::write(root.join("docs/specs/adr/one.md"), "one\n").unwrap();
    fs::write(root.join("docs/specs/two.md"), "two\n").unwrap();

    let (code, value) = json_of(root, &["chore", "commit", "--json"]);
    // guard: the schema covers both commands' shapes
    validate_against_schema(&load_schema(root, "chore"), &value);
    assert_eq!(code, 0, "{:?}", value);
    assert_eq!(
        value["included"],
        json!(["docs/specs/adr/one.md", "docs/specs/two.md"]),
        "the directory pattern must reach everything beneath it"
    );
}

// ---------------------------------------------------------------------------
// D-1: `qdev get chore` must not be built
// ---------------------------------------------------------------------------

#[test]
fn chore_is_not_an_entity_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(root, &["chore", "start", "x", "--paths", "README.md"]);
    assert_eq!(code, 0, "got: {}", stdout);
    let (code, stdout) = run(root, &["get", "chore", "chore-x", "--json"]);
    assert_eq!(code, 2, "got: {}", stdout);
    assert!(
        stdout.contains("Unknown schema kind 'chore'"),
        "`chore` must not resolve as an entity kind, got: {}",
        stdout
    );
}

// ---------------------------------------------------------------------------
// Settling a chore that is never going to be committed
// ---------------------------------------------------------------------------

#[test]
fn a_mistyped_allowlist_is_a_way_out_not_a_dead_end() {
    // The whole reason `close` exists: with an allowlist that matches nothing, `commit` refuses
    // (`nothing_to_commit`) and every later `start` is refused (`chore_in_progress`) — with
    // nothing but deleting `.qdev/chores/<id>.json` by hand as the way out.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "start",
            "docs left untouched",
            "--paths",
            "docs/ghost.*",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);

    let (code, stdout) = run(root, &["chore", "commit", "--json"]);
    assert_eq!(code, 1, "got: {}", stdout);
    assert!(stdout.contains("nothing_to_commit"), "got: {}", stdout);

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "start",
            "second attempt",
            "--paths",
            "README.md",
            "--json",
        ],
    );
    assert_eq!(code, 5, "got: {}", stdout);
    assert!(stdout.contains("chore_in_progress"), "got: {}", stdout);

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "close",
            "--reason",
            "superseded by a real story",
            "--json",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    let value: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["status"], json!("closed"));
    assert_eq!(value["reason"], json!("superseded by a real story"));
    // The record shape is the same one `start` returns, so it must still validate.
    validate_against_schema(&load_schema(root, "chore"), &value);

    // The slot is free, the settled record is still listed, and nothing was committed.
    let (code, stdout) = run(
        root,
        &["chore", "start", "second attempt", "--paths", "README.md"],
    );
    assert_eq!(code, 0, "got: {}", stdout);

    let (code, stdout) = run(root, &["chore", "list"]);
    assert_eq!(code, 0, "got: {}", stdout);
    assert!(
        stdout.contains("[closed]  docs left untouched"),
        "the settled record must still be listed:\n{}",
        stdout
    );
    assert!(
        stdout.contains("[open]  second attempt"),
        "and a new chore must be open:\n{}",
        stdout
    );
    assert_eq!(
        git(root, &["log", "--oneline"]).lines().count(),
        1,
        "closing a chore commits nothing: {}",
        git(root, &["log", "--oneline"])
    );
}

#[test]
fn aborting_a_chore_records_it_as_abandoned_by_whoever_ran_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let (code, stdout) = run(
        root,
        &[
            "chore",
            "start",
            "chase a flaky test",
            "--paths",
            "tests/**",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);

    let (code, value) = json_of(
        root,
        &[
            "chore",
            "abort",
            "--reason",
            "not worth the time",
            "--author-type",
            "agent",
            "--author-id",
            "bot-9",
            "--json",
        ],
    );
    assert_eq!(code, 0, "got: {:?}", value);
    assert_eq!(value["status"], json!("abandoned"));
    assert_eq!(value["reason"], json!("not worth the time"));
    assert_eq!(
        value["closed_by"],
        json!({ "type": "agent", "id": "bot-9" }),
        "whoever settled it is recorded, not whoever started it"
    );
    validate_against_schema(&load_schema(root, "chore"), &value);

    // Same record on disk, and no decision was written: only `chore commit` records a ruling.
    let record: Value = serde_json::from_str(
        &fs::read_to_string(root.join(".qdev/chores/chore-chase-a-flaky-test.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(record["status"], json!("abandoned"));
    assert_eq!(record["reason"], json!("not worth the time"));
    assert_eq!(
        fs::read_dir(root.join("docs/state/decisions"))
            .map(|rd| rd.count())
            .unwrap_or(0),
        0,
        "a settled chore writes no DEC record"
    );

    // And the listing shows it, so skipped work stays visible.
    let (code, value) = json_of(root, &["chore", "list", "--json"]);
    assert_eq!(code, 0, "got: {:?}", value);
    assert_eq!(value["record_dir"], json!(".qdev/chores"));
    assert_eq!(value["chores"][0]["status"], json!("abandoned"));
}

#[test]
fn close_and_abort_without_an_open_chore_give_no_active_chore() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    for verb in ["close", "abort"] {
        let (code, stdout) = run(root, &["chore", verb, "--json"]);
        assert_eq!(code, 2, "`chore {}` got: {}", verb, stdout);
        assert!(
            stdout.contains("no_active_chore"),
            "`chore {}` must give `no_active_chore`, got: {}",
            verb,
            stdout
        );
    }

    // `list` works with nothing recorded, and says so.
    let (code, value) = json_of(root, &["chore", "list", "--json"]);
    assert_eq!(code, 0, "got: {:?}", value);
    assert_eq!(value["chores"].as_array().unwrap().len(), 0);
    validate_against_schema(&load_schema(root, "chore"), &value);
}

#[test]
fn records_follow_the_relocated_cache_dir() {
    // D-1 with a moved cache: `init` creates and gitignores `var/chores`, so the record has to
    // land there — writing it to `.qdev/chores` would leave it git-visible and uncreated.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    fs::write(
        root.join("qdev.toml"),
        "[project]\nname = \"TestProject\"\n\n[identity]\ndeveloper_id = \"simon\"\nteams = [\"core-platform\"]\n\n[storage]\nspecs_dir = \"docs/specs\"\nstate_dir = \"docs/state\"\ncache_dir = \"var/qdev-cache\"\n",
    )
    .unwrap();
    let (code, stdout) = run(
        root,
        &[
            "init",
            "--non-interactive",
            "--name",
            "TestProject",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ],
    );
    assert_eq!(code, 0, "got: {}", stdout);

    let (code, stdout) = run(
        root,
        &["chore", "start", "relocated record", "--paths", "docs/**"],
    );
    assert_eq!(code, 0, "got: {}", stdout);
    assert!(
        stdout.contains("var/chores/chore-relocated-record.json"),
        "the record must be reported where it was written: {}",
        stdout
    );
    assert!(root.join("var/chores/chore-relocated-record.json").exists());
    assert!(!root
        .join(".qdev/chores/chore-relocated-record.json")
        .exists());

    let status = git(root, &["status", "--porcelain"]);
    assert!(
        !status.contains("chores"),
        "no record may reach git, wherever the cache lives:\n{}",
        status
    );

    // Listing reads the configured directory too.
    let (code, value) = json_of(root, &["chore", "list", "--json"]);
    assert_eq!(code, 0, "got: {:?}", value);
    assert_eq!(value["record_dir"], json!("var/chores"));
    assert_eq!(value["chores"].as_array().unwrap().len(), 1);
}
