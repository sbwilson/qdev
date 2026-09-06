use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

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
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("E12S3"));

    let created_file = stories_dir.join("E12S3.md");
    assert!(created_file.is_file());
}

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
        .args(["create", "story", "E12"])
        .assert()
        .success()
        .code(0)
        .stdout(predicate::str::contains("E12S5"));

    let created_file = stories_dir.join("E12S5.md");
    assert!(created_file.is_file());
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
    // Create E12S1.md as a directory so allocate_next_story_id skips it (not a file),
    // allocates E12S1, but create_new(true) encounters AlreadyExists and returns Conflict (code 5)
    fs::create_dir_all(stories_dir.join("E12S1.md")).unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["create", "story", "E12"])
        .assert()
        .failure()
        .code(5);
}
