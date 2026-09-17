#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::Path;

use qdev_core::decision::{log_decision, DecisionInput, VALID_DECISION_TYPES};
use qdev_core::schema::{extract_frontmatter, validate_value_detailed, EntityKind};
use qdev_core::store::{SqliteStore, Store};
use qdev_core::{ensure_cache, Author, StorageConfig};
use tempfile::TempDir;

fn setup_test_workspace(root: &Path) {
    let qdev_toml = root.join("qdev.toml");
    fs::write(
        qdev_toml,
        r#"
[project]
name = "TestProject"

[identity]
developer_id = "simon"
teams = ["core-platform"]

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"
"#,
    )
    .unwrap();

    let stories_dir = root.join("docs/specs/stories");
    let epics_dir = root.join("docs/specs/epics");
    let adrs_dir = root.join("docs/specs/adrs");
    let decisions_dir = root.join("docs/state/decisions");
    let cache_dir = root.join(".qdev/cache");

    fs::create_dir_all(&stories_dir).unwrap();
    fs::create_dir_all(&epics_dir).unwrap();
    fs::create_dir_all(&adrs_dir).unwrap();
    fs::create_dir_all(&decisions_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    ensure_cache(root, &StorageConfig::default()).unwrap();
}

fn write_story_file(root: &Path, id: &str, title: &str) -> String {
    let content = format!(
        r#"---
id: {id}
title: "{title}"
status: ready
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# {title}

Spec content.
"#
    );
    let path = root.join("docs/specs/stories").join(format!("{}.md", id));
    fs::write(&path, &content).unwrap();
    content
}

fn write_epic_file(root: &Path, id: &str, title: &str) -> String {
    let content = format!(
        r#"---
id: {id}
title: "{title}"
status: ready
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# {title}

Epic content.
"#
    );
    let path = root.join("docs/specs/epics").join(format!("{}.md", id));
    fs::write(&path, &content).unwrap();
    content
}

fn write_adr_file(root: &Path, id: &str, title: &str) -> String {
    let content = format!(
        r#"---
id: {id}
title: "{title}"
status: active
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# {title}

ADR content.
"#
    );
    let path = root.join("docs/specs/adrs").join(format!("{}.md", id));
    fs::write(&path, &content).unwrap();
    content
}

#[test]
fn test_valid_decision_types_constant() {
    assert_eq!(
        VALID_DECISION_TYPES,
        &[
            "human_ruling",
            "agent_assumption",
            "cross_team_override",
            "pivot",
            "review_rejection",
            "lease_override",
            "story_abandoned",
            "story_superseded",
        ]
    );
}

#[test]
fn test_happy_path_manual_decision_logging() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let original_story = write_story_file(root, "E12S4", "Buffer Manager");
    let storage = StorageConfig::default();
    let _ = ensure_cache(root, &storage).unwrap();

    let input = DecisionInput {
        subject_id: "E12S4".to_string(),
        decision_type: "human_ruling".to_string(),
        topic: Some("Buffer sizing".to_string()),
        context: None,
        ruling: "Fixed 4 MB pool".to_string(),
        author: Author::new("human", "simon"),
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    let payload = log_decision(root, Some(&storage), &input).unwrap();

    // 1. Verify allocated ID format
    assert!(payload.id.starts_with("DEC-"));
    assert!(payload.id.len() >= 8); // "DEC-" + 4+ hex chars
    assert_eq!(payload.subject_id, "E12S4");
    assert_eq!(payload.decision_type, "human_ruling");
    assert_eq!(payload.topic.as_deref(), Some("Buffer sizing"));
    assert_eq!(payload.context, "Decision on E12S4");
    assert_eq!(payload.ruling, "Fixed 4 MB pool");
    assert_eq!(payload.author, Author::new("human", "simon"));

    // 2. Verify file on disk
    let dec_file_path = root.join(&payload.path);
    assert!(dec_file_path.is_file());
    let file_content = fs::read_to_string(&dec_file_path).unwrap();

    // 3. Verify frontmatter and body format
    let fm = extract_frontmatter(&file_content).unwrap();
    assert_eq!(fm["id"], payload.id);
    assert_eq!(fm["title"], "Buffer sizing");
    assert_eq!(fm["status"], "active");
    assert_eq!(fm["version"], 1);
    assert_eq!(fm["subject_id"], "E12S4");
    assert_eq!(fm["decision_type"], "human_ruling");
    assert_eq!(fm["topic"], "Buffer sizing");
    assert_eq!(fm["context"], "Decision on E12S4");
    assert_eq!(fm["ruling"], "Fixed 4 MB pool");
    assert_eq!(fm["created_by"]["type"], "human");
    assert_eq!(fm["created_by"]["id"], "simon");
    assert_eq!(fm["updated_by"]["type"], "human");
    assert_eq!(fm["updated_by"]["id"], "simon");

    validate_value_detailed(EntityKind::Decision, &fm).expect("Schema validation passes");

    assert!(
        file_content.contains("# Buffer sizing\n\nContext: Decision on E12S4\n\nFixed 4 MB pool\n")
    );

    // 4. Verify SQLite cache synchronization
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db).unwrap();

    let entity = store
        .get_entity(&payload.id)
        .unwrap()
        .expect("entity record indexed");
    assert_eq!(entity.id, payload.id);
    assert_eq!(entity.kind, EntityKind::Decision);
    assert_eq!(entity.title.as_deref(), Some("Buffer sizing"));
    assert_eq!(entity.status.as_deref(), Some("active"));

    let dec_record = store
        .get_decision(&payload.id)
        .unwrap()
        .expect("decision record indexed");
    assert_eq!(dec_record.id, payload.id);
    assert_eq!(dec_record.subject_id, "E12S4");
    assert_eq!(dec_record.decision_type.as_deref(), Some("human_ruling"));
    assert_eq!(dec_record.topic.as_deref(), Some("Buffer sizing"));
    assert_eq!(dec_record.context.as_deref(), Some("Decision on E12S4"));
    assert_eq!(dec_record.ruling.as_deref(), Some("Fixed 4 MB pool"));
    assert_eq!(dec_record.author_type.as_deref(), Some("human"));
    assert_eq!(dec_record.author_id.as_deref(), Some("simon"));

    // 5. Verify subject entity file was NOT modified
    let current_story = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    assert_eq!(current_story, original_story);
}

#[test]
fn test_decision_with_custom_context_and_agent_author() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    write_adr_file(root, "AD-43", "Serialization Architecture");
    let storage = StorageConfig::default();
    let _ = ensure_cache(root, &storage).unwrap();

    let input = DecisionInput {
        subject_id: "AD-43".to_string(),
        decision_type: "agent_assumption".to_string(),
        topic: Some("Serialization".to_string()),
        context: Some("Benchmarked 30% faster".to_string()),
        ruling: "Use rmp-serde".to_string(),
        author: Author::new("agent", "claude-opt"),
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    let payload = log_decision(root, Some(&storage), &input).unwrap();
    assert_eq!(payload.context, "Benchmarked 30% faster");
    assert_eq!(payload.ruling, "Use rmp-serde");
    assert_eq!(payload.decision_type, "agent_assumption");

    let dec_file_path = root.join(&payload.path);
    let file_content = fs::read_to_string(&dec_file_path).unwrap();
    let fm = extract_frontmatter(&file_content).unwrap();

    assert_eq!(fm["subject_id"], "AD-43");
    assert_eq!(fm["context"], "Benchmarked 30% faster");
    assert_eq!(fm["created_by"]["type"], "agent");
    assert_eq!(fm["created_by"]["id"], "claude-opt");

    assert!(file_content.contains("Context: Benchmarked 30% faster\n\nUse rmp-serde\n"));
}

#[test]
fn test_decision_across_different_subject_kinds() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    write_epic_file(root, "E12", "Storage Engine Epic");
    write_adr_file(root, "AD-43", "Concurrency Design");
    write_story_file(root, "E12S4", "Async IO");

    let storage = StorageConfig::default();
    let _ = ensure_cache(root, &storage).unwrap();

    for (subject, expected_type) in [
        ("E12", "pivot"),
        ("AD-43", "agent_assumption"),
        ("E12S4", "review_rejection"),
    ] {
        let input = DecisionInput {
            subject_id: subject.to_string(),
            decision_type: expected_type.to_string(),
            topic: Some(format!("Topic for {}", subject)),
            context: None,
            ruling: format!("Ruling for {}", subject),
            author: Author::new("human", "lead"),
            title: None,
            timestamp: None,
            validate_subject: true,
        };
        let payload = log_decision(root, Some(&storage), &input).unwrap();
        assert_eq!(payload.subject_id, subject);
        assert_eq!(payload.decision_type, expected_type);
    }
}

#[test]
fn test_unresolvable_subject_fails_entity_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let storage = StorageConfig::default();

    let input = DecisionInput {
        subject_id: "NONEXISTENT".to_string(),
        decision_type: "human_ruling".to_string(),
        topic: Some("Topic".to_string()),
        context: None,
        ruling: "Ruling".to_string(),
        author: Author::new("human", "simon"),
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    let err = log_decision(root, Some(&storage), &input).unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);

    // Verify no file was created under docs/state/decisions
    let dec_dir = root.join("docs/state/decisions");
    let count = fs::read_dir(dec_dir).unwrap().count();
    assert_eq!(count, 0);
}

#[test]
fn test_invalid_decision_type_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    write_story_file(root, "E12S4", "Buffer");
    let storage = StorageConfig::default();

    let input = DecisionInput {
        subject_id: "E12S4".to_string(),
        decision_type: "invalid_type".to_string(),
        topic: Some("Topic".to_string()),
        context: None,
        ruling: "Ruling".to_string(),
        author: Author::new("human", "simon"),
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    let err = log_decision(root, Some(&storage), &input).unwrap_err();
    assert_eq!(err.code(), "usage_error");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
    assert!(err
        .message()
        .contains("Invalid decision type 'invalid_type'"));
}

#[test]
fn test_empty_or_whitespace_ruling_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    write_story_file(root, "E12S4", "Buffer");
    let storage = StorageConfig::default();

    for empty_ruling in ["", "   ", "\t\n  "] {
        let input = DecisionInput {
            subject_id: "E12S4".to_string(),
            decision_type: "human_ruling".to_string(),
            topic: Some("Topic".to_string()),
            context: None,
            ruling: empty_ruling.to_string(),
            author: Author::new("human", "simon"),
            title: None,
            timestamp: None,
            validate_subject: true,
        };

        let err = log_decision(root, Some(&storage), &input).unwrap_err();
        assert_eq!(err.code(), "usage_error");
        assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
        assert!(err.message().contains("Decision ruling cannot be empty"));
    }
}

#[test]
fn test_invalid_author_rejected() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    write_story_file(root, "E12S4", "Buffer");
    let storage = StorageConfig::default();

    let input = DecisionInput {
        subject_id: "E12S4".to_string(),
        decision_type: "human_ruling".to_string(),
        topic: Some("Topic".to_string()),
        context: None,
        ruling: "Ruling".to_string(),
        author: Author::new("invalid_author_type", "simon"),
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    let err = log_decision(root, Some(&storage), &input).unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
}

#[test]
fn test_decisions_exempt_from_story_lease_gating() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    write_story_file(root, "E12S4", "Buffer");
    let storage = StorageConfig::default();

    // Someone else claims a lease on E12S4
    let _ = qdev_core::lease::claim_story(
        root,
        "E12S4",
        &Author::new("human", "alice"),
        Some(&storage),
        None,
    )
    .unwrap();

    // Now Simon logs a decision on E12S4 without holding a lease
    let input = DecisionInput {
        subject_id: "E12S4".to_string(),
        decision_type: "human_ruling".to_string(),
        topic: Some("External review ruling".to_string()),
        context: None,
        ruling: "Approved despite alice's lease".to_string(),
        author: Author::new("human", "simon"),
        title: None,
        timestamp: None,
        validate_subject: true,
    };

    let payload = log_decision(root, Some(&storage), &input).unwrap();
    assert_eq!(payload.subject_id, "E12S4");
    assert!(payload.id.starts_with("DEC-"));
}
