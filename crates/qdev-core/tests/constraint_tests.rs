use std::fs;
use std::path::Path;

use qdev_core::rusqlite;
use qdev_core::write::Author;
use qdev_core::{
    allocate_next_constraint_id_in, apply_constraint_add, apply_constraint_remove,
    ConstraintAddOptions, ConstraintKind, ConstraintRemoveOptions, ExitCode,
    StorageConfig,
};
use tempfile::TempDir;

fn setup_test_story(root: &Path, id: &str, status: &str, version: u64, constraints_yaml: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    let constraints_block = if constraints_yaml.is_empty() {
        String::new()
    } else {
        format!("constraints:\n{constraints_yaml}")
    };
    fs::write(
        dir.join(format!("{id}.md")),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: {status}
version: {version}
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{constraints_block}---

## Acceptance Criteria
- Verify constraint behavior.
"#
        ),
    )
    .unwrap();
}

fn setup_test_epic(root: &Path, id: &str, status: &str, version: u64, constraints_yaml: &str) {
    let dir = root.join("docs/specs/epics");
    fs::create_dir_all(&dir).unwrap();
    let constraints_block = if constraints_yaml.is_empty() {
        String::new()
    } else {
        format!("constraints:\n{constraints_yaml}")
    };
    fs::write(
        dir.join(format!("{id}.md")),
        format!(
            r#"---
id: {id}
title: "Epic {id}"
status: {status}
version: {version}
owners:
  - simon
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{constraints_block}---

## Summary
- Epic summary.
"#
        ),
    )
    .unwrap();
}

#[test]
fn test_constraint_id_allocation_monotonic() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    setup_test_story(root, "E12S4", "draft", 1, "");

    let st = StorageConfig::default();

    // 1. Initial allocation for no_go should be NG-1
    let id1 = allocate_next_constraint_id_in(root, &st, "E12S4", ConstraintKind::NoGo, None).unwrap();
    assert_eq!(id1.to_string(), "E12S4/NG-1");

    // 2. Initial allocation for rabbit_hole should be RH-1
    let id2 = allocate_next_constraint_id_in(root, &st, "E12S4", ConstraintKind::RabbitHole, None).unwrap();
    assert_eq!(id2.to_string(), "E12S4/RH-1");

    // 3. Initial allocation for appetite should be APP-1
    let id3 = allocate_next_constraint_id_in(root, &st, "E12S4", ConstraintKind::Appetite, None).unwrap();
    assert_eq!(id3.to_string(), "E12S4/APP-1");

    // 4. Initial allocation for Epic E12
    setup_test_epic(root, "E12", "planning", 1, "");
    let id_epic = allocate_next_constraint_id_in(root, &st, "E12", ConstraintKind::RabbitHole, None).unwrap();
    assert_eq!(id_epic.to_string(), "E12/RH-1");
}

#[test]
fn test_parse_constraint_seq_rejects_leading_zeroes() {
    use qdev_core::parse_constraint_seq;

    assert_eq!(parse_constraint_seq("NG-1", None, ConstraintKind::NoGo), Some(1));
    assert_eq!(parse_constraint_seq("NG-10", None, ConstraintKind::NoGo), Some(10));
    assert_eq!(parse_constraint_seq("E12S4/NG-1", Some("E12S4"), ConstraintKind::NoGo), Some(1));

    // Rejects leading zeroes e.g. 01, 007, 0
    assert_eq!(parse_constraint_seq("NG-01", None, ConstraintKind::NoGo), None);
    assert_eq!(parse_constraint_seq("NG-0", None, ConstraintKind::NoGo), None);
    assert_eq!(parse_constraint_seq("E12S4/NG-05", Some("E12S4"), ConstraintKind::NoGo), None);
}

#[test]
fn test_apply_constraint_add_success_and_cache_sync() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_test_story(root, "E12S4", "draft", 1, "");

    let author = Author::new("human", "simon");
    let opts = ConstraintAddOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_id: "E12S4".to_string(),
        kind: ConstraintKind::NoGo,
        text: "Do not touch frame buffers".to_string(),
        if_version: Some(1),
        author: author.clone(),
    };

    let res = apply_constraint_add(&opts).expect("apply_constraint_add should succeed");
    assert_eq!(res.id, "E12S4/NG-1");
    assert_eq!(res.relative_id, "NG-1");
    assert_eq!(res.owner_id, "E12S4");
    assert_eq!(res.kind, "no_go");
    assert_eq!(res.text, "Do not touch frame buffers");
    assert_eq!(res.old_version, 1);
    assert_eq!(res.new_version, 2);
    assert_eq!(res.rel_path, "docs/specs/stories/E12S4.md");

    // Check frontmatter on disk
    let disk_content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(disk_content.contains("version: 2"));
    assert!(disk_content.contains("id: NG-1"));
    assert!(disk_content.contains("kind: no_go"));
    assert!(disk_content.contains("text: \"Do not touch frame buffers\"") || disk_content.contains("text: Do not touch frame buffers"));

    // Check SQLite cache
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    assert!(cache_db_path.exists());
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();

    let constraint_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constraints WHERE id = 'E12S4/NG-1' AND owner_id = 'E12S4' AND kind = 'no_go';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(constraint_count, 1);

    let cached_version: u64 = conn
        .query_row(
            "SELECT version FROM entities WHERE id = 'E12S4';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cached_version, 2);

    // Now add a second constraint of the same kind: should allocate NG-2
    let opts2 = ConstraintAddOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_id: "E12S4".to_string(),
        kind: ConstraintKind::NoGo,
        text: "Do not use unsafe".to_string(),
        if_version: Some(2),
        author: author.clone(),
    };

    let res2 = apply_constraint_add(&opts2).expect("apply_constraint_add should succeed for second constraint");
    assert_eq!(res2.id, "E12S4/NG-2");
    assert_eq!(res2.new_version, 3);

    let total_constraints: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constraints WHERE owner_id = 'E12S4';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(total_constraints, 2);
}

#[test]
fn test_apply_constraint_add_epic() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_test_epic(root, "E12", "planning", 1, "");

    let author = Author::new("human", "simon");
    let opts = ConstraintAddOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_id: "E12".to_string(),
        kind: ConstraintKind::RabbitHole,
        text: "len == 0 does not mean empty".to_string(),
        if_version: None,
        author,
    };

    let res = apply_constraint_add(&opts).expect("apply_constraint_add to epic should succeed");
    assert_eq!(res.id, "E12/RH-1");
    assert_eq!(res.owner_id, "E12");
    assert_eq!(res.rel_path, "docs/specs/epics/E12.md");

    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let constraint_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constraints WHERE id = 'E12/RH-1' AND owner_id = 'E12';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(constraint_count, 1);
}

#[test]
fn test_apply_constraint_add_empty_text_refused() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_test_story(root, "E12S4", "draft", 1, "");

    let author = Author::new("human", "simon");
    let opts = ConstraintAddOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_id: "E12S4".to_string(),
        kind: ConstraintKind::NoGo,
        text: "   ".to_string(),
        if_version: None,
        author,
    };

    let err = apply_constraint_add(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_apply_constraint_add_if_version_mismatch() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_test_story(root, "E12S4", "draft", 2, "");

    let author = Author::new("human", "simon");
    let opts = ConstraintAddOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_id: "E12S4".to_string(),
        kind: ConstraintKind::NoGo,
        text: "Something".to_string(),
        if_version: Some(1),
        author,
    };

    let err = apply_constraint_add(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "version_mismatch");
}

#[test]
fn test_apply_constraint_remove_draft_without_justification() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    setup_test_story(root, "E12S4", "draft", 1, constraints_yaml);

    // Populate initial SQLite cache
    let author = Author::new("human", "simon");
    let add_opts = ConstraintAddOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_id: "E12S4".to_string(),
        kind: ConstraintKind::RabbitHole,
        text: "Secondary constraint".to_string(),
        if_version: Some(1),
        author: author.clone(),
    };
    let _ = apply_constraint_add(&add_opts).unwrap(); // Version now 2, has NG-1 and RH-1

    let remove_opts = ConstraintRemoveOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        constraint_id: "E12S4/NG-1".to_string(),
        justification: None,
        if_version: Some(2),
        author: author.clone(),
    };

    let res = apply_constraint_remove(&remove_opts).expect("remove on draft should succeed without justification");
    assert_eq!(res.id, "E12S4/NG-1");
    assert_eq!(res.owner_id, "E12S4");
    assert_eq!(res.new_version, 3);

    // Verify cache dropped NG-1 row
    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    let conn = rusqlite::Connection::open(&cache_db_path).unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constraints WHERE id = 'E12S4/NG-1';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);

    // RH-1 should still exist in cache
    let count_rh: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM constraints WHERE id = 'E12S4/RH-1';",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count_rh, 1);
}

#[test]
fn test_apply_constraint_remove_non_draft_without_justification_fails() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    setup_test_story(root, "E12S4", "ready", 1, constraints_yaml);

    let author = Author::new("human", "simon");
    let remove_opts = ConstraintRemoveOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        constraint_id: "E12S4/NG-1".to_string(),
        justification: None,
        if_version: None,
        author: author.clone(),
    };

    let err = apply_constraint_remove(&remove_opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_justification");

    // Whitespace only justification also fails
    let remove_opts_whitespace = ConstraintRemoveOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        constraint_id: "E12S4/NG-1".to_string(),
        justification: Some("   \t  ".to_string()),
        if_version: None,
        author: author.clone(),
    };
    let err2 = apply_constraint_remove(&remove_opts_whitespace).unwrap_err();
    assert_eq!(err2.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err2.code(), "needs_justification");

    // Files untouched: version remains 1
    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("version: 1"));
    assert!(content.contains("NG-1"));
}

#[test]
fn test_apply_constraint_remove_non_draft_with_justification_succeeds() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let constraints_yaml = "  - id: NG-1\n    kind: no_go\n    text: Do not touch frame buffers\n";
    setup_test_story(root, "E12S4", "in-progress", 1, constraints_yaml);

    let author = Author::new("human", "simon");
    let remove_opts = ConstraintRemoveOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        constraint_id: "E12S4/NG-1".to_string(),
        justification: Some("Architecture approved frame buffer direct access".to_string()),
        if_version: Some(1),
        author,
    };

    let res = apply_constraint_remove(&remove_opts).expect("remove on non-draft with justification should succeed");
    assert_eq!(res.id, "E12S4/NG-1");
    assert_eq!(res.new_version, 2);

    let content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert!(content.contains("version: 2"));
    assert!(!content.contains("NG-1"));
}

#[test]
fn test_apply_constraint_remove_not_found() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_test_story(root, "E12S4", "draft", 1, "");

    let author = Author::new("human", "simon");
    let remove_opts = ConstraintRemoveOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        constraint_id: "E12S4/NG-99".to_string(),
        justification: None,
        if_version: None,
        author,
    };

    let err = apply_constraint_remove(&remove_opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "entity_not_found");
}
