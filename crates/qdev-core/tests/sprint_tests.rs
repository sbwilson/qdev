#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::Path;

use qdev_core::config::{Config, StorageConfig};
use qdev_core::errors::ExitCode;
use qdev_core::query::{query_entity, QueryOptions};
use qdev_core::sprint::{
    assign_to_sprint, close_sprint, normalize_sprint_id, open_sprint, resolve_sprint_selection,
    sprint_entity_id, SprintAssignOptions, SprintCloseOptions, SprintOpenOptions,
};
use qdev_core::store::{ensure_cache, SqliteStore, Store};
use qdev_core::validate::find_duplicate_active_sprint_assignments;
use qdev_core::{Author, EntityKind};
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

    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/state/sprints")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    ensure_cache(root, &StorageConfig::default()).unwrap();
}

fn test_author() -> Author {
    Author {
        author_type: "human".to_string(),
        id: "simon".to_string(),
    }
}

fn write_story(root: &Path, store: &SqliteStore, id: &str, title: &str, status: &str) {
    let dir = root.join("docs/specs/stories");
    let content = format!(
        r#"---
id: {id}
title: "{title}"
status: {status}
version: 1
owners:
  - simon
appetite: small
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# {id}: {title}
"#
    );
    let path = dir.join(format!("{}.md", id));
    fs::write(&path, &content).unwrap();

    let entity_rec = qdev_core::EntityRecord {
        id: id.to_string(),
        kind: EntityKind::Story,
        title: Some(title.to_string()),
        status: Some(status.to_string()),
        owners: Some(r#"["simon"]"#.to_string()),
        source_path: format!("docs/specs/stories/{}.md", id),
        content_hash: qdev_core::sha256_digest(content.as_bytes()),
        version: 1,
        created_by: Some(test_author()),
        updated_by: Some(test_author()),
        updated_at: "2026-09-15T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: Some("small".to_string()),
        safety_class: None,
        target_modules: None,
    };
    store.upsert_entity(&entity_rec).unwrap();
}

#[test]
fn test_normalize_sprint_id() {
    assert_eq!(normalize_sprint_id("6").unwrap(), 6);
    assert_eq!(normalize_sprint_id("sprint-6").unwrap(), 6);
    assert_eq!(normalize_sprint_id("Sprint-6").unwrap(), 6);
    assert_eq!(normalize_sprint_id("  sprint-42  ").unwrap(), 42);

    assert!(normalize_sprint_id("").is_err());
    assert!(normalize_sprint_id("0").is_err());
    assert!(normalize_sprint_id("-1").is_err());
    assert!(normalize_sprint_id("sprint-0").is_err());
    assert!(normalize_sprint_id("abc").is_err());
    assert_eq!(sprint_entity_id(6), "sprint-6");
}

#[test]
fn test_open_sprint_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    let options = SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "The Rust Core Port",
        release: Some("0.1.0"),
        author: &author,
    };

    let result = open_sprint(&options).expect("open_sprint should succeed");
    assert_eq!(result.sprint_id, 6);
    assert_eq!(result.entity_id, "sprint-6");
    assert_eq!(result.title, "The Rust Core Port");
    assert_eq!(result.status, "active");
    assert_eq!(result.release.as_deref(), Some("0.1.0"));

    let sprint_file = root.join("docs/state/sprints/sprint-6.md");
    assert!(sprint_file.is_file(), "Sprint markdown file must exist");
    let content = fs::read_to_string(&sprint_file).unwrap();
    assert!(content.contains("id: sprint-6"));
    assert!(content.contains("status: active"));
    assert!(content.contains("release: 0.1.0"));
    assert!(content.contains("assignments: []"));

    // Verify cache tables
    let cached_sprint = store.get_sprint(6).unwrap().expect("sprint in cache");
    assert_eq!(cached_sprint.id, 6);
    assert_eq!(cached_sprint.status.as_deref(), Some("active"));
    assert_eq!(cached_sprint.release_version.as_deref(), Some("0.1.0"));

    let cached_entity = store
        .get_entity("sprint-6")
        .unwrap()
        .expect("entity in cache");
    assert_eq!(cached_entity.kind, EntityKind::Sprint);
    assert_eq!(cached_entity.status.as_deref(), Some("active"));
}

#[test]
fn test_open_sprint_empty_title_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    let options = SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "   ",
        release: None,
        author: &author,
    };

    let err = open_sprint(&options).expect_err("Empty title must fail");
    assert_eq!(err.code(), "empty_title");
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
}

#[test]
fn test_open_sprint_duplicate_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    let options = SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    };

    open_sprint(&options).expect("first open must succeed");

    let err = open_sprint(&options).expect_err("duplicate open must fail");
    assert_eq!(err.code(), "sprint_already_exists");
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
}

#[test]
fn test_assign_stories_to_sprint() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    })
    .unwrap();

    write_story(root, &store, "E12S4", "Story 4", "ready");
    write_story(root, &store, "E11S9", "Story 9", "ready");

    let e12s4_orig_content = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e11s9_orig_content = fs::read_to_string(root.join("docs/specs/stories/E11S9.md")).unwrap();

    let assign_opts = SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        stories: &["E12S4".to_string(), "E11S9".to_string()],
        author: &author,
    };

    let result = assign_to_sprint(&assign_opts).expect("assign should succeed");
    assert_eq!(result.sprint_id, 6);
    assert_eq!(result.assigned_stories, vec!["E12S4", "E11S9"]);

    // Verify sprint file frontmatter contains assignments
    let sprint_content = fs::read_to_string(root.join("docs/state/sprints/sprint-6.md")).unwrap();
    assert!(sprint_content.contains("story: E12S4"));
    assert!(sprint_content.contains("story: E11S9"));
    assert!(sprint_content.contains("assigned_at:"));

    // Verify SQLite sprint_assignments table
    let assignments = store.get_sprint_assignments(6).unwrap();
    assert_eq!(assignments.len(), 2);
    let mut stories: Vec<String> = assignments.into_iter().map(|a| a.story_id).collect();
    stories.sort();
    assert_eq!(stories, vec!["E11S9", "E12S4"]);

    // Story markdown files and identifiers are immutable!
    let e12s4_after = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e11s9_after = fs::read_to_string(root.join("docs/specs/stories/E11S9.md")).unwrap();
    assert_eq!(
        e12s4_orig_content, e12s4_after,
        "Story E12S4 file must remain unmodified"
    );
    assert_eq!(
        e11s9_orig_content, e11s9_after,
        "Story E11S9 file must remain unmodified"
    );
}

#[test]
fn test_assign_to_nonexistent_sprint_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    write_story(root, &store, "E12S4", "Story 4", "ready");

    let assign_opts = SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 999,
        stories: &["E12S4".to_string()],
        author: &author,
    };

    let err = assign_to_sprint(&assign_opts).expect_err("Must fail for missing sprint");
    assert_eq!(err.code(), "sprint_not_found");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_assign_nonexistent_story_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    })
    .unwrap();

    let assign_opts = SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        stories: &["E99S99".to_string()],
        author: &author,
    };

    let err = assign_to_sprint(&assign_opts).expect_err("Must fail for missing story");
    assert_eq!(err.code(), "story_not_found");
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_close_sprint_carry_over() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    // Open sprint 5 and sprint 6
    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 5,
        title: "Sprint 5",
        release: None,
        author: &author,
    })
    .unwrap();

    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    })
    .unwrap();

    // E12S4 is ready (not done), E12S5 is done
    write_story(root, &store, "E12S4", "Story 4", "ready");
    write_story(root, &store, "E12S5", "Story 5", "done");

    let e12s4_before = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e12s5_before = fs::read_to_string(root.join("docs/specs/stories/E12S5.md")).unwrap();

    // Assign both to sprint 5
    assign_to_sprint(&SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 5,
        stories: &["E12S4".to_string(), "E12S5".to_string()],
        author: &author,
    })
    .unwrap();

    // Close sprint 5 with carry-over to sprint 6
    let close_opts = SprintCloseOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 5,
        carry_over_target: Some(6),
        author: &author,
    };

    let result = close_sprint(&close_opts).expect("close should succeed");
    assert_eq!(result.sprint_id, 5);
    assert_eq!(result.status, "completed");
    assert_eq!(result.carry_over_target, Some(6));
    assert_eq!(result.carried_stories, vec!["E12S4"]); // E12S5 (done) not carried over!

    // Verify sprint 5 is completed in cache and markdown
    let sprint5 = store.get_sprint(5).unwrap().unwrap();
    assert_eq!(sprint5.status.as_deref(), Some("completed"));
    assert!(sprint5.completed_at.is_some());

    let sprint5_content = fs::read_to_string(root.join("docs/state/sprints/sprint-5.md")).unwrap();
    assert!(sprint5_content.contains("status: completed"));

    // Verify sprint 6 has carried over E12S4 with carried_from: 5
    let sprint6_content = fs::read_to_string(root.join("docs/state/sprints/sprint-6.md")).unwrap();
    assert!(sprint6_content.contains("story: E12S4"));
    assert!(sprint6_content.contains("carried_from: 5"));

    // Done story E12S5 was NOT carried over to sprint 6
    assert!(!sprint6_content.contains("story: E12S5"));

    // Verify SQLite sprint_assignments for sprint 6
    let s6_assignments = store.get_sprint_assignments(6).unwrap();
    assert_eq!(s6_assignments.len(), 1);
    assert_eq!(s6_assignments[0].story_id, "E12S4");
    assert_eq!(s6_assignments[0].carried_from, Some(5));

    // Stories remain in sprint 5 as well
    let s5_assignments = store.get_sprint_assignments(5).unwrap();
    assert_eq!(s5_assignments.len(), 2);

    // Story markdown files and IDs remain strictly immutable!
    let e12s4_after = fs::read_to_string(root.join("docs/specs/stories/E12S4.md")).unwrap();
    let e12s5_after = fs::read_to_string(root.join("docs/specs/stories/E12S5.md")).unwrap();
    assert_eq!(e12s4_before, e12s4_after);
    assert_eq!(e12s5_before, e12s5_after);
}

#[test]
fn test_resolve_sprint_selection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    let mut config = Config::default();

    // 0 active sprints -> usage error
    let err =
        resolve_sprint_selection(&store, &config, None).expect_err("0 active sprints must fail");
    assert_eq!(err.exit_code(), ExitCode::UsageError);

    // Explicit argument overrides
    assert_eq!(
        resolve_sprint_selection(&store, &config, Some("7")).unwrap(),
        7
    );
    assert_eq!(
        resolve_sprint_selection(&store, &config, Some("sprint-7")).unwrap(),
        7
    );

    // Open single active sprint (6)
    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    })
    .unwrap();

    // With 1 active sprint and no default_sprint, resolves 6
    assert_eq!(resolve_sprint_selection(&store, &config, None).unwrap(), 6);

    // Open second active sprint (7)
    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 7,
        title: "Sprint 7",
        release: None,
        author: &author,
    })
    .unwrap();

    // Multiple active sprints and no default_sprint -> usage error
    let err = resolve_sprint_selection(&store, &config, None)
        .expect_err("ambiguous active sprints must fail");
    assert_eq!(err.exit_code(), ExitCode::UsageError);

    // With default_sprint set in config -> resolves default_sprint
    config.project.default_sprint = Some(6);
    assert_eq!(resolve_sprint_selection(&store, &config, None).unwrap(), 6);
}

#[test]
fn test_duplicate_active_sprint_validation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 5,
        title: "Sprint 5",
        release: None,
        author: &author,
    })
    .unwrap();

    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    })
    .unwrap();

    write_story(root, &store, "E12S4", "Story 4", "ready");

    assign_to_sprint(&SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 5,
        stories: &["E12S4".to_string()],
        author: &author,
    })
    .unwrap();

    assign_to_sprint(&SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        stories: &["E12S4".to_string()],
        author: &author,
    })
    .unwrap();

    let findings = find_duplicate_active_sprint_assignments(&store).unwrap();
    assert_eq!(findings.len(), 2);
    assert_eq!(findings[0].code, "duplicate_active_sprint_assignment");
    assert_eq!(findings[0].severity, "error");
    assert!(findings[0]
        .message
        .as_ref()
        .unwrap()
        .contains("multiple active sprints: sprint-5, sprint-6"));

    // When sprint 5 is closed, duplicate active sprint finding disappears
    close_sprint(&SprintCloseOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 5,
        carry_over_target: None,
        author: &author,
    })
    .unwrap();

    let findings_after = find_duplicate_active_sprint_assignments(&store).unwrap();
    assert!(
        findings_after.is_empty(),
        "No duplicate active sprints after closing sprint 5"
    );
}

#[test]
fn test_query_sprint_projection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);

    let db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&db_path).unwrap();
    let storage = StorageConfig::default();
    let author = test_author();

    open_sprint(&SprintOpenOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        title: "Sprint 6",
        release: None,
        author: &author,
    })
    .unwrap();

    write_story(root, &store, "E12S4", "Story 4", "ready");
    write_story(root, &store, "E12S5", "Story 5", "done");

    assign_to_sprint(&SprintAssignOptions {
        workspace_root: root,
        storage: &storage,
        store: &store,
        sprint: 6,
        stories: &["E12S4".to_string(), "E12S5".to_string()],
        author: &author,
    })
    .unwrap();

    // Query via "sprint-6"
    let q_opts = QueryOptions {
        expand_scratch: false,
    };
    let res = query_entity(&store, None, "sprint-6", &q_opts).unwrap();
    match res {
        qdev_core::GetResult::Entity(p) => {
            assert_eq!(p.id, "sprint-6");
            assert_eq!(p.kind, EntityKind::Sprint);
            let assignments = p.assignments.expect("assignments must be projected");
            assert_eq!(assignments.len(), 2);
            let counts = p.status_counts.expect("status_counts must be projected");
            assert_eq!(counts.get("ready"), Some(&1));
            assert_eq!(counts.get("done"), Some(&1));
        }
        _ => panic!("Expected entity projection"),
    }

    // Query via numeric "6"
    let res_num = query_entity(&store, None, "6", &q_opts).unwrap();
    match res_num {
        qdev_core::GetResult::Entity(p) => {
            assert_eq!(p.id, "sprint-6");
        }
        _ => panic!("Expected entity projection"),
    }
}
