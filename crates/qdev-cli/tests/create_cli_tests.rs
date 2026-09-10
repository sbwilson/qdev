use assert_cmd::Command;
use predicates::prelude::*;
use qdev_core::rusqlite;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

/// `qdev init` a workspace the tests can then create stories in.
fn init_workspace(root: &std::path::Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args([
            "init",
            "--non-interactive",
            "--name",
            "CreateTestProject",
            "--developer",
            "alice",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();
}

#[test]
fn test_create_story_clean_dir() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();

    // Text prints allocated ID and path
    assert!(stdout_str.contains("E12S1"));
    assert!(stdout_str.contains("docs/specs/stories/E12S1.md"));

    let created_file = root.join("docs/specs/stories/E12S1.md");
    assert!(created_file.is_file());

    let content = fs::read_to_string(&created_file).unwrap();
    assert!(content.contains("id: E12S1"));
    assert!(content.contains("status: draft"));
    assert!(content.contains("version: 1"));
    assert!(content.contains("created_by:\n  type: human"));
    assert!(content.contains("updated_by:\n  type: human"));
    assert!(content.contains("## Acceptance Criteria"));
}

#[test]
fn test_create_story_sequential() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join("E12S1.md"), "existing 1").unwrap();
    fs::write(stories_dir.join("E12S2.md"), "existing 2").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("E12S3"));

    let created_file = stories_dir.join("E12S3.md");
    assert!(created_file.is_file());
}

/// Allocation leaves a gap. `create story` takes one past the highest number in the shared
/// in-use id set, rather than the lowest free one: an id is a citation target, so reusing a
/// deleted story's number would silently redirect every reference to it. The set is shared with
/// `--fix-ids`, so the two cannot disagree about which ids are taken.
#[test]
fn test_create_story_with_gaps() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join("E12S1.md"), "existing 1").unwrap();
    fs::write(stories_dir.join("E12S4.md"), "existing 4").unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0)
        // One past the highest, not the lowest free: an id is a citation target, so a gap is
        // left rather than a deleted story's id being handed to a new one.
        .stdout(predicate::str::contains("E12S5"));

    assert!(stories_dir.join("E12S5.md").is_file());
    assert!(
        !stories_dir.join("E12S2.md").exists(),
        "the gap must be left alone"
    );
    // Neither occupied id is handed out again.
    assert_eq!(
        fs::read_to_string(stories_dir.join("E12S1.md")).unwrap(),
        "existing 1"
    );
    assert_eq!(
        fs::read_to_string(stories_dir.join("E12S4.md")).unwrap(),
        "existing 4"
    );
}

#[test]
fn test_create_story_json_envelope() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["create", "story", "E12", "--json"])
        .assert()
        .success()
        .code(0);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["id"], "E12S1");
    assert_eq!(val["path"], "docs/specs/stories/E12S1.md");

    let created_file = root.join("docs/specs/stories/E12S1.md");
    assert!(created_file.is_file());
}

#[test]
fn test_create_story_sprint_prefix_rejection_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "S5E2"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Sprint-prefixed"));
}

#[test]
fn test_create_story_sprint_prefix_rejection_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["create", "story", "S5E2", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON error envelope on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("Sprint-prefixed"));
}

#[test]
fn test_create_story_non_epic_arg_text() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "AD-43"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("must be an Epic ID"));
}

#[test]
fn test_create_story_non_epic_arg_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["create", "story", "AD-43", "--json"])
        .assert()
        .failure()
        .code(2);

    let output = assert.get_output();
    let stdout_str = std::str::from_utf8(&output.stdout).unwrap();
    let val: Value = serde_json::from_str(stdout_str).expect("Valid JSON error envelope on stdout");

    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["error"]["code"], "usage_error");
    assert!(val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("must be an Epic ID"));
}

#[test]
fn test_create_story_with_optional_flags() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args([
            "create",
            "story",
            "E12",
            "--title",
            "CoreResponse Buffer Layout",
            "--appetite",
            "small",
            "--module",
            "bridge",
            "--module",
            "foundation",
            "--owner",
            "simon",
            "--safety-class",
            "ClassB",
        ])
        .assert()
        .success()
        .code(0);

    let created_file = root.join("docs/specs/stories/E12S1.md");
    assert!(created_file.is_file());

    let content = fs::read_to_string(&created_file).unwrap();
    assert!(content.contains("id: E12S1"));
    assert!(content.contains("title: \"CoreResponse Buffer Layout\""));
    assert!(content.contains("appetite: small"));
    assert!(content.contains("safety_class: ClassB"));
    assert!(content.contains("target_modules: [\"bridge\",\"foundation\"]"));
    assert!(content.contains("owners: [\"simon\"]"));
}

#[test]
fn test_create_story_disjoint_epics_increment_independently() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join("E11S1.md"), "existing").unwrap();
    fs::write(stories_dir.join("E12S1.md"), "existing").unwrap();

    let mut cmd1 = Command::cargo_bin("qdev").unwrap();
    cmd1.current_dir(root)
        .args(["create", "story", "E11"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("E11S2"));

    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("E12S2"));

    assert!(stories_dir.join("E11S2.md").is_file());
    assert!(stories_dir.join("E12S2.md").is_file());
}

#[test]
fn test_create_story_attribution_from_local_config() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(
        root.join(".qdev.local.toml"),
        "[identity]\ndeveloper_id = \"alice\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0);

    let created_file = root.join("docs/specs/stories/E12S1.md");
    assert!(created_file.is_file());

    let content = fs::read_to_string(&created_file).unwrap();
    assert!(content.contains("created_by:\n  type: human\n  id: alice"));
    assert!(content.contains("updated_by:\n  type: human\n  id: alice"));
}

#[test]
fn test_create_story_title_escaped() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args([
            "create",
            "story",
            "E12",
            "--title",
            "Feature \"quoted\": [special]",
        ])
        .assert()
        .success()
        .code(0);

    let created_file = root.join("docs/specs/stories/E12S1.md");
    let content = fs::read_to_string(&created_file).unwrap();
    assert!(content.contains("title: \"Feature \\\"quoted\\\": [special]\""));
}

#[test]
fn test_create_story_file_conflict() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    // Create E12S1.md as a directory so allocate_next_story_id skips it (not a file) and still
    // allocates E12S1. `create_story` writes through `write_file_atomic`, whose rename would
    // happily clobber an occupied path, so exclusivity comes from an explicit
    // `symlink_metadata` check taken *inside* the advisory lock — that check is what this test
    // pins. A directory (rather than a file) is used because it also answers the question no
    // error kind can: `symlink_metadata` reports a file, a directory and a dangling symlink
    // alike, where the previous `OpenOptions::create_new` reported `AlreadyExists` on Unix but
    // `ERROR_ACCESS_DENIED` on Windows.
    fs::create_dir_all(stories_dir.join("E12S1.md")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12"])
        .assert()
        .failure()
        .code(5)
        .stderr(predicate::str::contains("file_exists"))
        .stderr(predicate::str::contains("already exists"));

    // The same refusal in JSON mode carries the id and path that were refused.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12", "--json"])
        .assert()
        .failure()
        .code(5);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "file_exists");
    assert_eq!(val["error"]["details"]["id"], "E12S1");
    assert_eq!(
        val["error"]["details"]["path"],
        "docs/specs/stories/E12S1.md"
    );
}

/// A configured `[storage]` layout must be honoured everywhere, not just by the readers.
///
/// The sweep, hydration and validation always read `StorageConfig`, but creation, id allocation,
/// `init` scaffolding and the `.gitignore` entries were written against the default layout. With
/// a non-default `[storage]`, `qdev create story` exited 0, reported a path under `docs/specs/`,
/// wrote it there, and no read command could see it; id allocation scanned the same wrong
/// directory so every create restarted at `E12S1`; and the live cache was not gitignored.
#[test]
fn test_create_story_honours_configured_specs_dir() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
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

    let mut toml = fs::read_to_string(root.join("qdev.toml")).unwrap();
    toml.push_str("\n[storage]\nspecs_dir = \"planning/specs\"\n");
    fs::write(root.join("qdev.toml"), toml).unwrap();

    let mut create = Command::cargo_bin("qdev").unwrap();
    let assert = create
        .current_dir(root)
        .args(["create", "story", "E12", "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();

    assert_eq!(val["path"], "planning/specs/stories/E12S1.md");
    assert!(root.join("planning/specs/stories/E12S1.md").is_file());
    assert!(
        !root.join("docs/specs/stories/E12S1.md").exists(),
        "nothing may be written to the default layout"
    );

    // Readable back through the cache — the check that actually failed before.
    let mut list = Command::cargo_bin("qdev").unwrap();
    let list_assert = list
        .current_dir(root)
        .args(["list", "stories", "--json"])
        .assert()
        .success();
    let listed: Value = serde_json::from_slice(&list_assert.get_output().stdout).unwrap();
    let ids: Vec<&str> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["E12S1"], "the created story must be readable");

    // Allocation must continue from the configured directory, not restart at 1.
    let mut second = Command::cargo_bin("qdev").unwrap();
    let second_assert = second
        .current_dir(root)
        .args(["create", "story", "E12", "--json"])
        .assert()
        .success();
    let second_val: Value = serde_json::from_slice(&second_assert.get_output().stdout).unwrap();
    assert_eq!(second_val["id"], "E12S2");
}

/// `init` run against a workspace already configured for a non-default layout must scaffold and
/// gitignore *that* layout — otherwise the real cache is untracked only by luck and a second,
/// empty one is committed.
#[test]
fn test_init_scaffolds_and_gitignores_the_configured_layout() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(
        root.join("qdev.toml"),
        "[project]\nname = \"Pre\"\n\n[storage]\nspecs_dir = \"planning/specs\"\nstate_dir = \"planning/state\"\ncache_dir = \".qdev/altcache\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
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

    assert!(root.join("planning/specs/stories").is_dir());
    assert!(root.join("planning/state/dw").is_dir());
    assert!(root.join(".qdev/altcache").is_dir());
    assert!(
        !root.join("docs/specs/stories").exists(),
        "the default layout must not be scaffolded alongside the configured one"
    );

    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|l| l.trim() == ".qdev/altcache/"),
        "the configured cache must be gitignored, got: {gitignore}"
    );
}

/// `create story` resolves its author through `resolve_author` like every other mutation, so an
/// agent is recorded as an agent. The hand-rolled implementation wrote `type: human`
/// unconditionally and ignored `QDEV_AUTHOR_TYPE`, which made attribution — audited under
/// AD-12 — a false record rather than a cosmetic gap. Asserted by re-reading the file.
#[test]
fn test_create_story_agent_attribution_from_env() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("QDEV_AUTHOR_TYPE", "agent")
        .env("QDEV_AUTHOR_ID", "bot")
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0);

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(
        content.contains("created_by:\n  type: agent\n  id: bot"),
        "created_by must record the resolved agent author, got: {content}"
    );
    assert!(
        content.contains("updated_by:\n  type: agent\n  id: bot"),
        "updated_by must record the resolved agent author, got: {content}"
    );
}

/// `--author-type` / `--author-id` are `resolve_author`'s first source, so a single create can be
/// attributed without touching the environment.
#[test]
fn test_create_story_attribution_flags_override_env() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("QDEV_AUTHOR_TYPE", "human")
        .env("QDEV_AUTHOR_ID", "alice")
        .args([
            "create",
            "story",
            "E12",
            "--author-type",
            "agent",
            "--author-id",
            "bot",
        ])
        .assert()
        .success()
        .code(0);

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(
        content.contains("created_by:\n  type: agent\n  id: bot"),
        "the flags must win over the environment, got: {content}"
    );
    assert!(
        content.contains("updated_by:\n  type: agent\n  id: bot"),
        "the flags must win over the environment, got: {content}"
    );
}

/// An unrecognized author type is refused wherever it comes from, before allocation or any
/// write — silently rewriting it to `human` would write a wrong-but-plausible audit record.
#[test]
fn test_create_story_bad_author_type_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut env_cmd = Command::cargo_bin("qdev").unwrap();
    env_cmd
        .current_dir(root)
        .env("QDEV_AUTHOR_TYPE", "robot")
        .args(["create", "story", "E12"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Invalid author type 'robot'"))
        .stderr(predicate::str::contains("QDEV_AUTHOR_TYPE"));

    let mut flag_cmd = Command::cargo_bin("qdev").unwrap();
    flag_cmd
        .current_dir(root)
        .args(["create", "story", "E12", "--author-type", "robot"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Invalid author type 'robot'"))
        .stderr(predicate::str::contains("--author-type"));

    assert!(
        !root.join("docs/specs/stories/E12S1.md").exists(),
        "a refused author type must leave nothing behind"
    );
}

/// The generated frontmatter is validated against the story schema *before* it is written, so a
/// value the schema rejects is a logical failure naming the offending field and no file appears.
/// Nothing checked the hand-rolled output at all: `qdev` could create a story `qdev validate`
/// then rejected and hydration marked stale.
#[test]
fn test_create_story_schema_invalid_output_leaves_no_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["create", "story", "E12", "--appetite", "huge", "--json"])
        .assert()
        .failure()
        .code(1);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "schema_validation_failed");
    assert!(
        val["error"]["message"]
            .as_str()
            .unwrap()
            .contains("appetite"),
        "the error must name the offending field, got: {}",
        val["error"]["message"]
    );
    assert_eq!(
        val["error"]["details"]["validation_errors"][0]["path"],
        "/appetite"
    );

    assert!(
        !root.join("docs/specs/stories/E12S1.md").exists(),
        "a schema-invalid story must not be written"
    );
}

/// The created story is in the cache the moment `create` returns, with its row marked dirty, so
/// it is queryable without waiting for the next boot sweep. The tables are read directly: every
/// command re-sweeps on boot, so a later `qdev get` alone would not distinguish "upserted by
/// the write path" from "picked up by the sweep". The `qdev get` at the end covers the
/// end-to-end path, and `qdev validate` confirms the generated file is schema-clean.
#[test]
fn test_create_story_upserts_cache_row_and_marks_it_dirty() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut init = Command::cargo_bin("qdev").unwrap();
    init.current_dir(root)
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

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12", "--title", "Buffer Layout"])
        .assert()
        .success();

    let conn = rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();

    let (title, status, source_path, version, stale): (String, String, String, i64, i64) = conn
        .query_row(
            "SELECT title, status, source_path, version, stale FROM entities WHERE id = 'E12S1';",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("the created story must have an entities row without a sweep");
    assert_eq!(title, "Buffer Layout");
    assert_eq!(status, "draft");
    assert_eq!(source_path, "docs/specs/stories/E12S1.md");
    assert_eq!(version, 1);
    assert_eq!(stale, 0, "a freshly created story is not stale");

    let (epic_id, seq): (String, i64) = conn
        .query_row(
            "SELECT epic_id, seq FROM stories WHERE id = 'E12S1';",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the kind-specific stories row must be upserted too");
    assert_eq!(epic_id, "E12");
    assert_eq!(seq, 1);

    let dirty: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM dirty_entities WHERE id = 'E12S1';",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(dirty, 1, "the created row must be marked dirty");
    drop(conn);

    let mut get = Command::cargo_bin("qdev").unwrap();
    let get_assert = get
        .current_dir(root)
        .args(["get", "story", "E12S1", "--json"])
        .assert()
        .success();
    let got: Value = serde_json::from_slice(&get_assert.get_output().stdout).unwrap();
    assert_eq!(got["id"], "E12S1");
    assert_eq!(got["title"], "Buffer Layout");
    assert_eq!(got["stale"], false);

    let mut validate = Command::cargo_bin("qdev").unwrap();
    let validate_assert = validate
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .success();
    let findings: Value = serde_json::from_slice(&validate_assert.get_output().stdout).unwrap();
    assert_eq!(
        findings["findings"].as_array().unwrap().len(),
        0,
        "a created story must be schema-clean: {findings}"
    );
}

/// `create story` takes the same advisory lock as every other write, so a create racing a
/// concurrent writer waits and then refuses rather than writing unserialized. The old
/// implementation took no lock at all.
#[test]
fn test_create_story_advisory_lock_timeout_exits_5() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();

    let mut init = Command::cargo_bin("qdev").unwrap();
    init.current_dir(&root)
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

    let lock_path = root.join(".qdev/cache/write.lock");
    let acquired = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let acquired_clone = acquired.clone();
    let release_clone = release.clone();

    let holder = std::thread::spawn(move || {
        let _guard =
            qdev_core::acquire_write_lock(&lock_path, Duration::from_millis(5000)).unwrap();
        acquired_clone.store(true, Ordering::SeqCst);
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_millis(8000) && !release_clone.load(Ordering::SeqCst)
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    });

    while !acquired.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(&root)
        .args(["create", "story", "E12", "--json"])
        .timeout(Duration::from_secs(20))
        .assert()
        .failure()
        .code(5);

    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "lock_timeout");
    assert!(
        !root.join("docs/specs/stories/E12S1.md").exists(),
        "a lock timeout must not write the story"
    );

    release.store(true, Ordering::SeqCst);
    let _ = holder.join();
}

/// `qdev create story` is exempt from the workspace check, so it can run in a bare directory.
/// It must not leave a cache database behind there: `upsert_cache_and_mark_dirty` would create a
/// 16-table, unstamped `cache.sqlite`, and a later `qdev init` reads that as a v0 cache needing
/// a confirmed migration — so create-then-init would exit 3 demanding `--yes`.
#[test]
fn test_create_story_outside_a_workspace_leaves_no_cache_and_init_still_works() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12", "--title", "Outside"])
        .assert()
        .success()
        .code(0);

    assert!(
        root.join("docs/specs/stories/E12S1.md").is_file(),
        "the story file is still written"
    );
    assert!(
        !root.join(".qdev").exists(),
        "no .qdev tree may be created outside a workspace — neither the cache (an unstamped one \
         makes the next `qdev init` demand a confirmed migration) nor the advisory lock"
    );

    // And `init` afterwards is not forced into a confirmed migration.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
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
}

/// A non-default `[storage] cache_dir` must be honoured by the new write path, which derives
/// both the advisory lock and the cache database from it. This is the bug class `8220a63` killed
/// for `specs_dir`: with the path hardcoded, the `is_file()` guard finds nothing at the default
/// location, the upsert is silently skipped, and the story is invisible until the next sweep.
#[test]
fn test_create_story_honours_configured_cache_dir() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    // Configure the layout *before* init, so the workspace is scaffolded against it and no
    // default-location cache is ever created.
    fs::write(
        root.join("qdev.toml"),
        "[project]\nname = \"Alt\"\n\n[storage]\ncache_dir = \".qdev/altcache\"\n",
    )
    .unwrap();
    init_workspace(root);

    // Boot once so the configured cache exists, the way any real workspace reaches this state.
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["status"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0);

    let alt_db = root.join(".qdev/altcache/cache.sqlite");
    assert!(
        alt_db.is_file(),
        "the configured cache must be the one used"
    );
    assert!(
        !root.join(".qdev/cache/cache.sqlite").exists(),
        "no cache may be created at the default location"
    );
    assert!(
        root.join(".qdev/altcache/write.lock").is_file(),
        "the advisory lock must live under the configured cache_dir"
    );

    let conn = rusqlite::Connection::open(&alt_db).unwrap();
    let in_entities: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM entities WHERE id = 'E12S1' AND stale = 0;",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let in_stories: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM stories WHERE id = 'E12S1';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let is_dirty: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM dirty_entities WHERE id = 'E12S1';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        in_entities && in_stories && is_dirty,
        "the row must land in the configured cache: entities={in_entities} stories={in_stories} dirty={is_dirty}"
    );
}

/// Attribution is audited, so a value that cannot be a real developer or agent name is refused
/// rather than escaped into the record. A newline used to reach the YAML renderer, which emitted
/// it as a block scalar spliced into one `key: value` line — a document that no longer parsed,
/// surfacing as a confusing `schema_validation_failed` about qdev's own output.
#[test]
fn test_create_story_rejects_a_multiline_author_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("QDEV_AUTHOR_TYPE", "agent")
        .env("QDEV_AUTHOR_ID", "bot\nstatus: done")
        .args(["create", "story", "E12"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("single line"));

    assert!(
        !root.join("docs/specs/stories/E12S1.md").exists(),
        "nothing may be written for a refused author"
    );
}

/// `QDEV_AUTHOR_ID=` — a variable defined but never given a value, common in CI — is treated as
/// absent, so resolution falls through to the config identity instead of failing every mutation
/// with "Author ID cannot be empty".
#[test]
fn test_create_story_empty_author_id_falls_back_to_config_identity() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();

    fs::write(
        root.join(".qdev.local.toml"),
        "[identity]\ndeveloper_id = \"alice\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .env("QDEV_AUTHOR_ID", "   ")
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0);

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(
        content.contains("id: alice"),
        "an empty env id must fall through to the config identity, got: {content}"
    );
}

// ---------------------------------------------------------------------------
// One rule for which ids are in use: `create story` never mints a taken id
// ---------------------------------------------------------------------------

/// Writes a story file at an arbitrary path (not necessarily the conventional one).
fn write_story_at(path: &std::path::Path, id: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            "---\nid: {id}\ntitle: \"Story {id}\"\nstatus: draft\nversion: 1\n\
             created_by:\n  type: human\n  id: alice\n\
             updated_by:\n  type: human\n  id: alice\n---\n\n## Acceptance Criteria\n- AC.\n"
        ),
    )
    .unwrap();
}

fn created_story_id(root: &std::path::Path, epic: &str) -> String {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .env_remove("QDEV_AUTHOR_TYPE")
        .env_remove("QDEV_AUTHOR_ID")
        .args(["create", "story", epic, "--json"])
        .assert()
        .success();
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    val["id"].as_str().unwrap().to_string()
}

fn validate_findings(root: &std::path::Path) -> Vec<Value> {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let output = cmd
        .current_dir(root)
        .args(["validate", "--json"])
        .assert()
        .get_output()
        .stdout
        .clone();
    let val: Value = serde_json::from_slice(&output).unwrap();
    val["findings"].as_array().cloned().unwrap_or_default()
}

/// The reproduction that named this defect: a hydrated story in a *subdirectory* of the stories
/// directory. The allocator read one flat directory, so it minted that story's id a second time
/// and the following `qdev validate` reported the duplicate the create had just made.
#[test]
fn test_create_story_skips_an_id_declared_in_a_subdirectory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    write_story_at(&root.join("docs/specs/stories/sub/E1S1.md"), "E1S1");

    assert_eq!(created_story_id(root, "E1"), "E1S2");
    assert!(
        !validate_findings(root)
            .iter()
            .any(|f| f["code"] == "duplicate_planning_id"),
        "the create must not manufacture a duplicate"
    );
}

/// A file whose name differs from its id only in case. The write path resolves such a name for
/// that id, so allocating it again left the entity unresolvable ("Multiple entity files match").
#[test]
fn test_create_story_skips_an_id_whose_filename_differs_only_in_case() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    write_story_at(&root.join("docs/specs/stories/e1s1-buffer.md"), "E1S1");

    assert_eq!(created_story_id(root, "E1"), "E1S2");
    assert!(!validate_findings(root)
        .iter()
        .any(|f| f["code"] == "duplicate_planning_id"));
}

/// A file named for an id whose frontmatter will not parse declares nothing, but still occupies
/// its name: `create_story`'s own occupancy check would refuse the create with `file_exists` for
/// an id the user never chose.
#[test]
fn test_create_story_skips_an_id_carried_by_an_unparseable_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    fs::write(
        root.join("docs/specs/stories/E1S1.md"),
        "---\nid: E1S1\nbroken: [\n---\n",
    )
    .unwrap();

    assert_eq!(created_story_id(root, "E1"), "E1S2");
}

/// The `.MD` row: hydration reads the file, so its declared id is in use.
#[test]
fn test_create_story_skips_an_id_declared_in_an_uppercase_extension_file() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    write_story_at(&root.join("docs/specs/stories/E1S1.MD"), "E1S1");

    assert_eq!(created_story_id(root, "E1"), "E1S2");
}

/// A `state_dir` id is in use exactly like a `specs_dir` one — the in-use set covers every
/// directory hydration reads.
#[test]
fn test_create_story_skips_an_id_declared_under_the_state_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_workspace(root);

    // A story-shaped id declared by a file in the state tree, which hydration also walks.
    write_story_at(&root.join("docs/state/decisions/E1S1.md"), "E1S1");

    assert_eq!(created_story_id(root, "E1"), "E1S2");
}
