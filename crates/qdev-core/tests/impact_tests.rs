use std::fs;
use std::path::Path;
use std::process::Command;

use qdev_core::config::{
    Config, GateConfig, GitConfig, HygieneConfig, ModuleConfig, StorageConfig,
};
use qdev_core::impact::{format_impact_text, run_impact, ImpactOptions};
use qdev_core::lease::claim_story;
use qdev_core::schema::EntityKind;
use qdev_core::store::{EntityRecord, RelationRecord, SqliteStore, Store};
use qdev_core::write::Author;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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

fn setup_repo(root: &Path) -> (Config, SqliteStore) {
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::create_dir_all(root.join(".qdev/leases")).unwrap();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/specs/requirements")).unwrap();
    fs::create_dir_all(root.join("docs/specs/hazards")).unwrap();
    fs::create_dir_all(root.join("docs/specs/adrs")).unwrap();
    fs::create_dir_all(root.join("crates/foundation/src")).unwrap();
    fs::create_dir_all(root.join("crates/bridge/src")).unwrap();

    let cache_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_path).unwrap();
    store
        .with_conn_mut(|conn| qdev_core::create_schema(conn))
        .unwrap();

    let config = Config {
        storage: StorageConfig {
            specs_dir: "docs/specs".to_string(),
            state_dir: "docs/state".to_string(),
            cache_dir: ".qdev/cache".to_string(),
        },
        git: GitConfig {
            integration_branch: "main".to_string(),
            ..Default::default()
        },
        modules: vec![
            ModuleConfig {
                id: "foundation".to_string(),
                paths: vec!["crates/foundation/**".to_string()],
                layer: Some(0),
                may_depend_on: Vec::new(),
            },
            ModuleConfig {
                id: "bridge".to_string(),
                paths: vec!["crates/bridge/**".to_string()],
                layer: Some(1),
                may_depend_on: vec!["foundation".to_string()],
            },
        ],
        gates: vec![
            GateConfig {
                id: "lint".to_string(),
                command: Some("cargo clippy".to_string()),
                timeout_ms: None,
                depends_on: Vec::new(),
                output_adapter: None,
                on_transition: Vec::new(),
                verifies: Vec::new(),
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "c-abi-round-trip".to_string(),
                command: Some("./gate.sh".to_string()),
                timeout_ms: None,
                depends_on: Vec::new(),
                output_adapter: None,
                on_transition: vec!["review".to_string()],
                verifies: vec!["FR-102".to_string()],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
            GateConfig {
                id: "sec-audit".to_string(),
                command: Some("cargo audit".to_string()),
                timeout_ms: None,
                depends_on: Vec::new(),
                output_adapter: None,
                on_transition: Vec::new(),
                verifies: vec!["NFR-99".to_string()],
                kind: None,
                metric: None,
                direction: None,
                skip: None,
            },
        ],
        hygiene: HygieneConfig::default(),
        ..Default::default()
    };

    (config, store)
}

fn author() -> Author {
    Author {
        author_type: "human".to_string(),
        id: "simon".to_string(),
    }
}

#[test]
fn test_impact_by_story_id_with_relations_and_gates() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    // Target story E12S4 targeting bridge
    store
        .upsert_entity(&EntityRecord {
            id: "E12S4".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Layout".to_string()),
            status: Some("ready".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S4.md".to_string(),
            content_hash: "hash1".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(4),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    // Active concurrent story E12S5 in bridge (in-progress)
    store
        .upsert_entity(&EntityRecord {
            id: "E12S5".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Async Handling".to_string()),
            status: Some("in-progress".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S5.md".to_string(),
            content_hash: "hash2".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(5),
            appetite: Some("medium".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    // Non-active story in bridge (ready) -> should NOT be in active stories
    store
        .upsert_entity(&EntityRecord {
            id: "E12S6".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Logging".to_string()),
            status: Some("ready".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S6.md".to_string(),
            content_hash: "hash3".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(6),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    // Active story in foundation (in-progress) -> should NOT be reported since foundation is unaffected
    store
        .upsert_entity(&EntityRecord {
            id: "E12S7".to_string(),
            kind: EntityKind::Story,
            title: Some("Foundation Types".to_string()),
            status: Some("in-progress".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S7.md".to_string(),
            content_hash: "hash4".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(7),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["foundation"]).unwrap()),
        })
        .unwrap();

    // Dependent stories: E12S8 depends on E12S4; E12S9 extends E12S8
    store
        .upsert_entity(&EntityRecord {
            id: "E12S8".to_string(),
            kind: EntityKind::Story,
            title: Some("Swift Bridge Binding".to_string()),
            status: Some("draft".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S8.md".to_string(),
            content_hash: "hash5".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(8),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    store
        .upsert_entity(&EntityRecord {
            id: "E12S9".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Extender".to_string()),
            status: Some("draft".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S9.md".to_string(),
            content_hash: "hash6".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(9),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    // Relations
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "traces_to".to_string(),
            target_id: "FR-102".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "mitigates".to_string(),
            target_id: "HAZ-14".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "governed_by".to_string(),
            target_id: "AD-43".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S8".to_string(),
            relation: "depends_on".to_string(),
            target_id: "E12S4".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S9".to_string(),
            relation: "extends".to_string(),
            target_id: "E12S8".to_string(),
        })
        .unwrap();

    let options = ImpactOptions {
        story: Some("E12S4".to_string()),
        paths: Vec::new(),
    };

    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();

    assert_eq!(outcome.target_story, Some("E12S4".to_string()));
    assert_eq!(outcome.modules, vec!["bridge"]);
    assert_eq!(outcome.requirements, vec!["FR-102"]);
    assert_eq!(outcome.hazards, vec!["HAZ-14"]);
    assert_eq!(outcome.adrs, vec!["AD-43"]);
    assert_eq!(outcome.gates, vec!["c-abi-round-trip"]); // overlaps on FR-102

    // Active stories: E12S5 (in-progress) is touching bridge
    assert_eq!(outcome.stories.len(), 1);
    assert_eq!(outcome.stories[0].id, "E12S5");
    assert_eq!(outcome.stories[0].status, "in-progress");

    // Dependents: E12S8 at depth 1, E12S9 at depth 2
    assert_eq!(outcome.dependents.len(), 2);
    assert_eq!(outcome.dependents[0].id, "E12S8");
    assert_eq!(outcome.dependents[0].depth, 1);
    assert_eq!(
        outcome.dependents[0].relation,
        Some("depends_on".to_string())
    );

    assert_eq!(outcome.dependents[1].id, "E12S9");
    assert_eq!(outcome.dependents[1].depth, 2);
    assert_eq!(outcome.dependents[1].relation, Some("extends".to_string()));

    // Verify text format
    let text = format_impact_text(&outcome);
    assert!(text.contains("Impact Analysis for Story: E12S4"));
    assert!(text.contains("Modules: bridge"));
    assert!(text.contains("Active Stories in Modules (1):"));
    assert!(text.contains("E12S5 [in-progress]: Bridge Async Handling"));
    assert!(text.contains("Transitive Dependents (2):"));
    assert!(text.contains("E12S8 (depth 1, depends_on)"));
    assert!(text.contains("E12S9 (depth 2, extends)"));
    assert!(text.contains("Linked Requirements: FR-102"));
    assert!(text.contains("Linked Hazards: HAZ-14"));
    assert!(text.contains("Linked ADRs: AD-43"));
    assert!(text.contains("Overlapping Gates: c-abi-round-trip"));
}

#[test]
fn test_impact_by_paths_and_citation_extraction() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    // Active story E12S4 in review touching bridge
    store
        .upsert_entity(&EntityRecord {
            id: "E12S4".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Core".to_string()),
            status: Some("review".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S4.md".to_string(),
            content_hash: "hash1".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(4),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "traces_to".to_string(),
            target_id: "FR-102".to_string(),
        })
        .unwrap();

    // Write file with citations in crates/bridge/src/lib.rs
    let code = r#"
    // Implementation per [FR-102]
    // Architecture decision: [AD-43]
    // Follows deferred item [DW-7f3a] and hazard [HAZ-14]
    pub fn bridge_call() {}
    "#;
    fs::write(root.join("crates/bridge/src/lib.rs"), code).unwrap();

    let options = ImpactOptions {
        story: None,
        paths: vec!["crates/bridge/src/lib.rs".to_string()],
    };

    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();

    assert_eq!(outcome.target_story, None);
    assert_eq!(outcome.target_paths, vec!["crates/bridge/src/lib.rs"]);
    assert_eq!(outcome.modules, vec!["bridge"]);
    assert_eq!(outcome.stories.len(), 1);
    assert_eq!(outcome.stories[0].id, "E12S4");
    assert_eq!(outcome.requirements, vec!["FR-102"]);
    assert_eq!(outcome.gates, vec!["c-abi-round-trip"]);

    // Check citations
    let mut expected_citations = vec![
        "AD-43".to_string(),
        "DW-7f3a".to_string(),
        "FR-102".to_string(),
        "HAZ-14".to_string(),
    ];
    expected_citations.sort();
    assert_eq!(outcome.cited_entities, expected_citations);
}

#[test]
fn test_impact_lease_fallback() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    store
        .upsert_entity(&EntityRecord {
            id: "E12S4".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Core".to_string()),
            status: Some("ready".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S4.md".to_string(),
            content_hash: "hash1".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(4),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    // Claim lease for E12S4
    claim_story(
        root,
        "E12S4",
        &author(),
        Some(&config.storage),
        Some(&store),
    )
    .unwrap();

    let options = ImpactOptions {
        story: None,
        paths: Vec::new(),
    };

    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();
    assert_eq!(outcome.target_story, Some("E12S4".to_string()));
    assert_eq!(outcome.modules, vec!["bridge"]);
}

#[test]
fn test_impact_missing_target_no_lease_fails_with_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    let options = ImpactOptions {
        story: None,
        paths: Vec::new(),
    };

    let err = run_impact(root, &config, &options, Some(&store)).unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("No target story or paths specified"));
}

#[test]
fn test_impact_nonexistent_story_fails_with_entity_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    let options = ImpactOptions {
        story: Some("NONEXISTENT".to_string()),
        paths: Vec::new(),
    };

    let err = run_impact(root, &config, &options, Some(&store)).unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::LogicalFailure);
    assert_eq!(err.code(), "entity_not_found");
    assert!(err.message().contains("Story 'NONEXISTENT' not found"));
}

#[test]
fn test_impact_circular_dependencies_gracefully_handled() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    // Story A and Story B depend on each other:
    // A depends on B, B depends on A
    for id in ["StoryA", "StoryB"] {
        store
            .upsert_entity(&EntityRecord {
                id: id.to_string(),
                kind: EntityKind::Story,
                title: Some(format!("Title {}", id)),
                status: Some("ready".to_string()),
                owners: None,
                source_path: format!("docs/specs/stories/{}.md", id),
                content_hash: "hash".to_string(),
                version: 1,
                created_by: Some(author()),
                updated_by: Some(author()),
                updated_at: "2026-09-26T00:00:00Z".to_string(),
                stale: false,
                epic_id: Some("E12".to_string()),
                seq: Some(1),
                appetite: Some("small".to_string()),
                safety_class: Some("ClassB".to_string()),
                target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
            })
            .unwrap();
    }

    // StoryA depends on StoryB
    store
        .upsert_relation(&RelationRecord {
            source_id: "StoryA".to_string(),
            relation: "depends_on".to_string(),
            target_id: "StoryB".to_string(),
        })
        .unwrap();

    // StoryB depends on StoryA
    store
        .upsert_relation(&RelationRecord {
            source_id: "StoryB".to_string(),
            relation: "depends_on".to_string(),
            target_id: "StoryA".to_string(),
        })
        .unwrap();

    let options = ImpactOptions {
        story: Some("StoryA".to_string()),
        paths: Vec::new(),
    };

    // Should complete cleanly and not hang in an infinite loop
    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();
    assert_eq!(outcome.dependents.len(), 1);
    assert_eq!(outcome.dependents[0].id, "StoryB");
    assert_eq!(outcome.dependents[0].depth, 1);
}

#[test]
fn test_impact_diamond_dependencies_deduplication() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    // Graph:
    // A is seed.
    // B depends on A (depth 1)
    // C depends on A (depth 1)
    // D depends on B and C (depth 2)
    for id in ["StoryA", "StoryB", "StoryC", "StoryD"] {
        store
            .upsert_entity(&EntityRecord {
                id: id.to_string(),
                kind: EntityKind::Story,
                title: Some(format!("Title {}", id)),
                status: Some("ready".to_string()),
                owners: None,
                source_path: format!("docs/specs/stories/{}.md", id),
                content_hash: "hash".to_string(),
                version: 1,
                created_by: Some(author()),
                updated_by: Some(author()),
                updated_at: "2026-09-26T00:00:00Z".to_string(),
                stale: false,
                epic_id: Some("E12".to_string()),
                seq: Some(1),
                appetite: Some("small".to_string()),
                safety_class: Some("ClassB".to_string()),
                target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
            })
            .unwrap();
    }

    store
        .upsert_relation(&RelationRecord {
            source_id: "StoryB".to_string(),
            relation: "depends_on".to_string(),
            target_id: "StoryA".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "StoryC".to_string(),
            relation: "depends_on".to_string(),
            target_id: "StoryA".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "StoryD".to_string(),
            relation: "depends_on".to_string(),
            target_id: "StoryB".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "StoryD".to_string(),
            relation: "depends_on".to_string(),
            target_id: "StoryC".to_string(),
        })
        .unwrap();

    let options = ImpactOptions {
        story: Some("StoryA".to_string()),
        paths: Vec::new(),
    };

    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();
    // B and C at depth 1, D at depth 2 (only once!)
    assert_eq!(outcome.dependents.len(), 3);
    let ids: Vec<&str> = outcome.dependents.iter().map(|d| d.id.as_str()).collect();
    assert_eq!(ids, vec!["StoryB", "StoryC", "StoryD"]);
    assert_eq!(outcome.dependents[0].depth, 1);
    assert_eq!(outcome.dependents[1].depth, 1);
    assert_eq!(outcome.dependents[2].depth, 2);
}

#[test]
fn test_impact_story_declared_gates_and_binary_graceful() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (mut config, store) = setup_repo(root);

    // Custom gate configured in config
    config.gates.push(GateConfig {
        id: "custom-gate".to_string(),
        command: Some("./check.sh".to_string()),
        timeout_ms: None,
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: Vec::new(),
        verifies: vec!["REQ-88".to_string()],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    });

    // Story file with declared gate in frontmatter
    let story_yaml = r#"---
id: E12S4
title: Gate Story
status: in-progress
gates:
  - custom-gate
target_modules:
  - bridge
---
# Body
"#;
    fs::write(root.join("docs/specs/stories/E12S4.md"), story_yaml).unwrap();

    // Create a binary/unreadable file in bridge
    fs::write(
        root.join("crates/bridge/src/binary.dat"),
        [0xFF, 0xFE, 0x00, 0xFD],
    )
    .unwrap();

    let options = ImpactOptions {
        story: Some("E12S4".to_string()),
        paths: vec!["crates/bridge/src/binary.dat".to_string()],
    };

    // Analysis succeeds without crashing on binary file
    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();

    // custom-gate is directly declared on story, so it overlaps, and its verifies (REQ-88) is reached
    assert!(outcome.gates.contains(&"custom-gate".to_string()));
    assert!(outcome.requirements.contains(&"REQ-88".to_string()));
}

#[test]
fn test_impact_git_worktree_diff_citations() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let (config, store) = setup_repo(root);

    let story_yaml = r#"---
id: E12S4
title: Bridge Story
status: in-progress
target_modules:
  - bridge
---
# Body
"#;
    fs::write(root.join("docs/specs/stories/E12S4.md"), story_yaml).unwrap();
    store
        .upsert_entity(&EntityRecord {
            id: "E12S4".to_string(),
            kind: EntityKind::Story,
            title: Some("Bridge Story".to_string()),
            status: Some("in-progress".to_string()),
            owners: None,
            source_path: "docs/specs/stories/E12S4.md".to_string(),
            content_hash: "hash1".to_string(),
            version: 1,
            created_by: Some(author()),
            updated_by: Some(author()),
            updated_at: "2026-09-26T00:00:00Z".to_string(),
            stale: false,
            epic_id: Some("E12".to_string()),
            seq: Some(4),
            appetite: Some("small".to_string()),
            safety_class: Some("ClassB".to_string()),
            target_modules: Some(serde_json::to_string(&vec!["bridge"]).unwrap()),
        })
        .unwrap();

    let tracked_file = root.join("crates/bridge/src/lib.rs");
    fs::write(&tracked_file, "// initial content\n").unwrap();

    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial commit"]);

    // Modify tracked file in working tree with inline citations
    fs::write(
        &tracked_file,
        "// Implements [FR-102] and mitigates [HAZ-3]\npub fn bridge_fn() {}\n",
    )
    .unwrap();

    // Run impact with story ID and empty paths
    let options = ImpactOptions {
        story: Some("E12S4".to_string()),
        paths: Vec::new(),
    };

    let outcome = run_impact(root, &config, &options, Some(&store)).unwrap();

    // Verify cited_entities extracted via git diff
    assert!(outcome.cited_entities.contains(&"FR-102".to_string()));
    assert!(outcome.cited_entities.contains(&"HAZ-3".to_string()));
}
