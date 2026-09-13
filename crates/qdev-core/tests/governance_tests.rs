use std::fs;
use std::path::Path;

use qdev_core::config::Config;
use qdev_core::governance::{
    add_team_to_entity_owners, canonical_team_string, classify_mutation,
    create_governance_override_decision, extract_entity_owners, is_user_owner,
    normalize_team_name, resolve_user_teams,
};
use qdev_core::lease::claim_story;
use qdev_core::schema::EntityKind;
use qdev_core::store::sqlite::SqliteStore;
use qdev_core::store::Store;
use qdev_core::write::Author;
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let stories_dir = root.join("docs/specs/stories");
    let epics_dir = root.join("docs/specs/epics");
    let decisions_dir = root.join("docs/state/decisions");
    let dw_dir = root.join("docs/state/deferred-work");

    fs::create_dir_all(&stories_dir).unwrap();
    fs::create_dir_all(&epics_dir).unwrap();
    fs::create_dir_all(&decisions_dir).unwrap();
    fs::create_dir_all(&dw_dir).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();

    fs::write(
        root.join("qdev.toml"),
        r#"[project]
name = "test"

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"

[identity]
developer_id = "sally"
teams = ["ui-shell"]

[teams]
ui-shell = ["sally", "sally@example.com"]
core-platform = ["alice", "bob"]
"#,
    )
    .unwrap();

    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
    qdev_core::create_schema(&conn).unwrap();
    qdev_core::stamp_cache_version(&conn).unwrap();
}

fn write_story_with_owners(
    root: &Path,
    story_id: &str,
    title: &str,
    status: &str,
    owners: &[&str],
) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let owners_yaml = if owners.is_empty() {
        String::new()
    } else {
        format!(
            "owners:\n{}",
            owners
                .iter()
                .map(|o| format!("  - \"{}\"", o))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let content = format!(
        r#"---
id: {story_id}
title: "{title}"
status: {status}
version: 1
created_by:
  type: human
  id: sally
updated_by:
  type: human
  id: sally
{owners_yaml}
---

## Acceptance Criteria
- AC1.
"#
    );
    fs::write(stories_dir.join(format!("{}.md", story_id)), content).unwrap();
}

fn write_epic_with_owners(root: &Path, epic_id: &str, title: &str, owners: &[&str]) {
    let epics_dir = root.join("docs/specs/epics");
    fs::create_dir_all(&epics_dir).unwrap();
    let owners_yaml = if owners.is_empty() {
        String::new()
    } else {
        format!(
            "owners:\n{}",
            owners
                .iter()
                .map(|o| format!("  - \"{}\"", o))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let content = format!(
        r#"---
id: {epic_id}
title: "{title}"
status: in-progress
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
{owners_yaml}
---

# Epic
"#
    );
    fs::write(epics_dir.join(format!("{}.md", epic_id)), content).unwrap();
}

#[test]
fn test_team_normalization_and_canonical() {
    assert_eq!(normalize_team_name("ui-shell"), "ui-shell");
    assert_eq!(normalize_team_name("team:ui-shell"), "ui-shell");
    assert_eq!(normalize_team_name("TEAM:core-platform "), "core-platform");
    assert_eq!(canonical_team_string("ui-shell"), "team:ui-shell");
    assert_eq!(canonical_team_string("team:ui-shell"), "team:ui-shell");
}

#[test]
fn test_resolve_user_teams() {
    let mut config = Config::default();
    config.identity.developer_id = "sally".to_string();
    config.identity.teams = vec!["team:ui-shell".to_string()];
    config.teams.teams.insert(
        "frontend".to_string(),
        vec!["sally".to_string(), "john".to_string()],
    );
    config
        .teams
        .teams
        .insert("backend".to_string(), vec!["alice".to_string()]);

    let author = Author::new("human", "sally");
    let teams = resolve_user_teams(&author, &config, None);
    assert!(teams.contains(&"ui-shell".to_string()));
    assert!(teams.contains(&"frontend".to_string()));
    assert!(!teams.contains(&"backend".to_string()));
}

#[test]
fn test_is_user_owner() {
    let mut config = Config::default();
    config.identity.developer_id = "sally".to_string();
    config.identity.teams = vec!["ui-shell".to_string()];

    let author = Author::new("human", "sally");

    // Empty owners -> anyone can edit
    assert!(is_user_owner(&author, &config, &[], None));

    // Direct developer match
    assert!(is_user_owner(
        &author,
        &config,
        &["sally".to_string()],
        None
    ));
    assert!(!is_user_owner(
        &author,
        &config,
        &["alice".to_string()],
        None
    ));

    // Team match with team: prefix
    assert!(is_user_owner(
        &author,
        &config,
        &["team:ui-shell".to_string()],
        None
    ));
    assert!(is_user_owner(
        &author,
        &config,
        &["ui-shell".to_string()],
        None
    ));
    assert!(!is_user_owner(
        &author,
        &config,
        &["team:core-platform".to_string()],
        None
    ));
}

#[test]
fn test_classify_in_lease_mutation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);

    fs::create_dir_all(root.join(".git")).unwrap();
    let author = Author::new("human", "sally");
    claim_story(root, "E12S4", &author, None, None).unwrap();

    let config = qdev_core::load_config(root).unwrap().config;
    let classification = classify_mutation(
        root,
        "E12S4",
        Some(EntityKind::Story),
        &author,
        &config,
        None,
    )
    .unwrap();

    assert!(!classification.is_out_of_lease);
    assert!(!classification.is_cross_team);
    assert!(classification.is_exempt);
}

#[test]
fn test_classify_out_of_lease_and_cross_team() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic_with_owners(root, "E12", "Epic 12", &["team:core-platform"]);

    fs::create_dir_all(root.join(".git")).unwrap();
    let author = Author::new("human", "sally");
    claim_story(root, "E12S4", &author, None, None).unwrap();

    let config = qdev_core::load_config(root).unwrap().config;
    let classification = classify_mutation(
        root,
        "E12",
        Some(EntityKind::Epic),
        &author,
        &config,
        None,
    )
    .unwrap();

    assert!(classification.is_out_of_lease);
    assert!(classification.is_cross_team);
    assert!(!classification.is_exempt);
    assert_eq!(
        classification.active_lease.as_ref().map(|l| l.story_id.as_str()),
        Some("E12S4")
    );
}

#[test]
fn test_classify_out_of_lease_same_team() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);
    write_epic_with_owners(root, "E12", "Epic 12", &["team:ui-shell"]);

    fs::create_dir_all(root.join(".git")).unwrap();
    let author = Author::new("human", "sally");
    claim_story(root, "E12S4", &author, None, None).unwrap();

    let config = qdev_core::load_config(root).unwrap().config;
    let classification = classify_mutation(
        root,
        "E12",
        Some(EntityKind::Epic),
        &author,
        &config,
        None,
    )
    .unwrap();

    assert!(classification.is_out_of_lease);
    assert!(!classification.is_cross_team);
}

#[test]
fn test_classify_exempt_entities() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);

    fs::create_dir_all(root.join(".git")).unwrap();
    let author = Author::new("human", "sally");
    claim_story(root, "E12S4", &author, None, None).unwrap();

    let config = qdev_core::load_config(root).unwrap().config;

    // 1. Decision is exempt from lease
    let dec_class = classify_mutation(
        root,
        "DEC-01a2",
        Some(EntityKind::Decision),
        &author,
        &config,
        None,
    )
    .unwrap();
    assert!(!dec_class.is_out_of_lease);
    assert!(dec_class.is_exempt);

    // 2. Child constraint of leased story is exempt
    let constraint_class = classify_mutation(
        root,
        "E12S4/C1",
        None,
        &author,
        &config,
        None,
    )
    .unwrap();
    assert!(!constraint_class.is_out_of_lease);
    assert!(constraint_class.is_exempt);

    // 3. Child constraint of OTHER story is NOT exempt
    let other_constraint_class = classify_mutation(
        root,
        "E12S5/C1",
        None,
        &author,
        &config,
        None,
    )
    .unwrap();
    assert!(other_constraint_class.is_out_of_lease);
    assert!(!other_constraint_class.is_exempt);
}

#[test]
fn test_create_governance_override_decision() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let author = Author::new("human", "sally");

    // Empty justification rejected
    let err = create_governance_override_decision(
        root,
        None,
        "E12",
        "cross_team_override",
        "   ",
        &author,
        None,
    )
    .unwrap_err();
    assert_eq!(err.code(), "needs_justification");

    // Valid override creates DEC record
    let dec_id = create_governance_override_decision(
        root,
        None,
        "E12",
        "cross_team_override",
        "Architect approved cross-boundary update",
        &author,
        None,
    )
    .unwrap();

    assert!(dec_id.starts_with("DEC-"));

    // Verify DEC file written to state/decisions
    let dec_file = root.join(format!("docs/state/decisions/{}.md", dec_id));
    assert!(dec_file.is_file());
    let content = fs::read_to_string(&dec_file).unwrap();
    assert!(content.contains("decision_type: cross_team_override"));
    assert!(content.contains("subject_id: E12"));
    assert!(content.contains("ruling: Architect approved cross-boundary update"));

    // Verify decision indexed in SQLite cache
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db).unwrap();
    let dec_record = store.get_decision(&dec_id).unwrap().expect("decision indexed");
    assert_eq!(dec_record.subject_id, "E12");
    assert_eq!(
        dec_record.decision_type.as_deref(),
        Some("cross_team_override")
    );
    assert_eq!(
        dec_record.ruling.as_deref(),
        Some("Architect approved cross-boundary update")
    );
}

#[test]
fn test_add_team_to_entity_owners() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic_with_owners(root, "E12", "Epic 12", &["team:core-platform"]);

    let author = Author::new("human", "sally");

    // Fails with version_mismatch if if_version doesn't match current version 1
    let conflict_err = add_team_to_entity_owners(root, None, "E12", "team:ui-shell", &author, Some(999)).unwrap_err();
    assert_eq!(conflict_err.code, "version_mismatch");

    let new_owners = add_team_to_entity_owners(root, None, "E12", "team:ui-shell", &author, Some(1)).unwrap();
    assert_eq!(new_owners, vec!["team:core-platform", "team:ui-shell"]);

    // Check frontmatter updated and version bumped to 2
    let epic_file = root.join("docs/specs/epics/E12.md");
    let content = fs::read_to_string(&epic_file).unwrap();
    assert!(content.contains("version: 2"));
    assert!(content.contains("team:ui-shell"));

    // Check extraction
    let extracted = extract_entity_owners(root, "E12", None, None);
    assert_eq!(extracted, vec!["team:core-platform", "team:ui-shell"]);

    // Adding an individual user without team: prefix
    let user_owners = add_team_to_entity_owners(root, None, "E12", "bob", &author, Some(2)).unwrap();
    assert_eq!(user_owners, vec!["team:core-platform", "team:ui-shell", "bob"]);

    // Adding same team again is idempotent and does not error
    let idempotent_owners =
        add_team_to_entity_owners(root, None, "E12", "team:ui-shell", &author, None).unwrap();
    assert_eq!(
        idempotent_owners,
        vec!["team:core-platform", "team:ui-shell", "bob"]
    );
}

fn write_dw(root: &Path, id: &str, origin_story_id: &str) {
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    let content = format!(
        r#"---
id: {id}
title: "Deferred work {id}"
status: open
version: 1
created_by:
  type: human
  id: sally
updated_by:
  type: human
  id: sally
origin_story_id: {origin_story_id}
target_module: "crates/qdev-core"
safety_risk: low
rationale: "Testing exemption"
gate: "manual"
---

# Details
"#
    );
    fs::write(dw_dir.join(format!("{}.md", id)), content).unwrap();
}

#[test]
fn test_originating_deferred_work_exemption() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);

    let author = Author::new("human", "sally");
    claim_story(root, "E12S4", &author, None, None).unwrap();

    // Write DW originating from leased story
    write_dw(root, "DW-001", "E12S4");
    // Write DW originating from another story
    write_dw(root, "DW-002", "E12S5");

    let cfg = Config::default();

    // DW originating from leased story is exempt
    let c1 = classify_mutation(root, "DW-001", Some(EntityKind::DeferredWork), &author, &cfg, None).unwrap();
    assert!(c1.is_exempt);
    assert!(!c1.is_out_of_lease);

    // DW originating from another story is not exempt
    let c2 = classify_mutation(root, "DW-002", Some(EntityKind::DeferredWork), &author, &cfg, None).unwrap();
    assert!(!c2.is_exempt);
    assert!(c2.is_out_of_lease);
}

#[test]
fn test_cross_worktree_lease_detection() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S5", "Story 5", "ready", &["team:ui-shell"]);

    // Create .git/qdev/leases/E12S5.json simulating another worktree's lease
    let lease_dir = root.join(".git/qdev/leases");
    fs::create_dir_all(&lease_dir).unwrap();
    let lease = qdev_core::lease::StoryLease {
        story_id: "E12S5".to_string(),
        holder: "foreign-dev".to_string(),
        author_type: "human".to_string(),
        worktree_path: "/path/to/foreign-worktree".to_string(),
        branch: "foreign-branch".to_string(),
        started_at: "2026-09-13T00:00:00Z".to_string(),
        session_token: "tok123".to_string(),
    };
    fs::write(lease_dir.join("E12S5.json"), serde_json::to_string(&lease).unwrap()).unwrap();

    let author = Author::new("human", "sally");
    let cfg = Config::default();

    // Current workspace holds no lease, but E12S5 is leased by foreign worktree
    let c = classify_mutation(root, "E12S5", Some(EntityKind::Story), &author, &cfg, None).unwrap();
    assert!(c.is_out_of_lease);
    assert_eq!(
        c.active_lease.as_ref().map(|l| l.worktree_path.as_str()),
        Some("/path/to/foreign-worktree")
    );
}

#[test]
fn test_child_constraint_owner_inheritance() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_with_owners(root, "E12S4", "Story 4", "in-progress", &["team:ui-shell"]);

    // Direct constraint identifier E12S4/NG-1 has no file or owners of its own, inherits from parent E12S4
    let owners = extract_entity_owners(root, "E12S4/NG-1", None, None);
    assert_eq!(owners, vec!["team:ui-shell"]);
}
