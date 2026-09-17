#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::Path;

use qdev_core::dw::{
    add_deferred_work, close_deferred_work, list_deferred_work_records, AddDeferredWorkInput,
    CloseDeferredWorkInput, ListDeferredWorkFilter,
};
use qdev_core::store::{ensure_cache, SqliteStore, Store};
use qdev_core::{
    load_config, Author, EntityKind, ExitCode, StorageConfig, TransitionEngine, TransitionOptions,
};
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
cache_dir = ".qdev/cache"

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]

[[modules]]
id = "foundation"
paths = ["crates/foundation/**"]
"#,
    )
    .unwrap();

    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/state/dw")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    ensure_cache(root, &StorageConfig::default()).unwrap();
}

fn setup_test_workspace_with_regulatory(root: &Path, require_rationale: &[&str]) {
    let rationale_list = require_rationale
        .iter()
        .map(|r| format!("\"{}\"", r))
        .collect::<Vec<_>>()
        .join(", ");
    let qdev_toml = root.join("qdev.toml");
    fs::write(
        qdev_toml,
        format!(
            r#"
[project]
name = "TestProject"

[identity]
developer_id = "simon"
teams = ["core-platform"]

[storage]
specs_dir = "docs/specs"
cache_dir = ".qdev/cache"

[regulatory]
require_rationale_for = [{}]

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]
"#,
            rationale_list
        ),
    )
    .unwrap();

    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/state/dw")).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    ensure_cache(root, &StorageConfig::default()).unwrap();
}

fn write_story_file(root: &Path, id: &str, content: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{}.md", id)), content).unwrap();
}

fn open_store(root: &Path) -> SqliteStore {
    let path = root.join(".qdev/cache/cache.sqlite");
    SqliteStore::open(&path).unwrap()
}

#[test]
fn test_add_deferred_work_negligible_risk_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let author = Author::new("human", "simon");
    let input = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Address memory footprint".to_string(),
        safety_risk: "negligible".to_string(),
        rationale: None,
        origin_story_id: None,
        gate: None,
        owners: Vec::new(),
        author: author.clone(),
    };

    let result = add_deferred_work(root, &cfg, &input).unwrap();
    assert!(result.id.starts_with("DW-"), "ID must start with DW-");
    assert_eq!(result.target_module, "bridge");
    assert_eq!(result.safety_risk, "negligible");
    assert_eq!(result.status, "open");

    // Check file exists at docs/state/dw/<id>.md
    let file_path = root.join(format!("docs/state/dw/{}.md", result.id));
    assert!(file_path.exists());
    let content = fs::read_to_string(&file_path).unwrap();
    assert!(content.contains(&format!("id: {}", result.id)));
    assert!(content.contains("target_module: bridge"));
    assert!(content.contains("safety_risk: negligible"));
    assert!(content.contains("status: open"));

    // Check SQLite cache synchronization
    let store = open_store(root);
    let entity = store
        .get_entity(&result.id)
        .unwrap()
        .expect("Entity must exist in store");
    assert_eq!(entity.kind, EntityKind::DeferredWork);
    assert_eq!(entity.status.as_deref(), Some("open"));
    assert!(entity
        .target_modules
        .as_deref()
        .unwrap_or("")
        .contains("bridge"));

    let records = store.list_deferred_work().unwrap();
    let record = records
        .iter()
        .find(|r| r.id == result.id)
        .expect("Record in deferred_work table");
    assert_eq!(record.target_module, "bridge");
    assert_eq!(record.safety_risk.as_deref(), Some("negligible"));
    assert_eq!(record.status.as_deref(), Some("open"));
    assert!(record.origin_story_id.is_none());
}

#[test]
fn test_add_deferred_work_unregistered_module_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let input = AddDeferredWorkInput {
        target_module: "unknown_mod".to_string(),
        title: "Test".to_string(),
        safety_risk: "negligible".to_string(),
        rationale: None,
        origin_story_id: None,
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let err = add_deferred_work(root, &cfg, &input).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("unknown_mod"));
}

#[test]
fn test_add_deferred_work_unresolvable_story_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let input = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Test".to_string(),
        safety_risk: "negligible".to_string(),
        rationale: None,
        origin_story_id: Some("E99S99".to_string()),
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let err = add_deferred_work(root, &cfg, &input).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "entity_not_found");
    assert!(err.message().contains("E99S99"));
}

#[test]
fn test_add_deferred_work_with_resolvable_story_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    write_story_file(
        root,
        "E1S1",
        r#"---
id: E1S1
title: "First story"
status: in-progress
version: 1
owners: [simon]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#,
    );

    let input = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Story linked work".to_string(),
        safety_risk: "negligible".to_string(),
        rationale: None,
        origin_story_id: Some("E1S1".to_string()),
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let result = add_deferred_work(root, &cfg, &input).unwrap();
    assert_eq!(result.origin_story_id, Some("E1S1".to_string()));

    let store = open_store(root);
    let records = store.list_deferred_work().unwrap();
    let record = records.iter().find(|r| r.id == result.id).unwrap();
    assert_eq!(record.origin_story_id.as_deref(), Some("E1S1"));
}

#[test]
fn test_add_deferred_work_invalid_risk_level_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let input = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Test".to_string(),
        safety_risk: "catastrophic".to_string(),
        rationale: None,
        origin_story_id: None,
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let err = add_deferred_work(root, &cfg, &input).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("valid risk levels are"));
}

#[test]
fn test_add_deferred_work_non_negligible_without_rationale_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    for risk in &["acceptable_with_mitigation", "unacceptable"] {
        let input = AddDeferredWorkInput {
            target_module: "bridge".to_string(),
            title: "Safety relevant work".to_string(),
            safety_risk: risk.to_string(),
            rationale: None,
            origin_story_id: None,
            gate: None,
            owners: Vec::new(),
            author: Author::new("human", "simon"),
        };

        let err = add_deferred_work(root, &cfg, &input).unwrap_err();
        assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
        assert!(
            err.message().contains("regulatory.require_rationale_for"),
            "Error must cite regulatory.require_rationale_for, got: {}",
            err.message()
        );
    }
}

#[test]
fn test_add_deferred_work_non_negligible_with_rationale_succeeds() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let input = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Mitigated issue".to_string(),
        safety_risk: "acceptable_with_mitigation".to_string(),
        rationale: Some("Covered by redundant hardware watchdog.".to_string()),
        origin_story_id: None,
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let result = add_deferred_work(root, &cfg, &input).unwrap();
    assert_eq!(result.safety_risk, "acceptable_with_mitigation");
    assert_eq!(
        result.rationale.as_deref(),
        Some("Covered by redundant hardware watchdog.")
    );

    let store = open_store(root);
    let records = store.list_deferred_work().unwrap();
    let record = records.iter().find(|r| r.id == result.id).unwrap();
    assert_eq!(
        record.safety_risk.as_deref(),
        Some("acceptable_with_mitigation")
    );
    assert_eq!(
        record.rationale.as_deref(),
        Some("Covered by redundant hardware watchdog.")
    );
}

#[test]
fn test_regulatory_config_forces_rationale_for_negligible() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace_with_regulatory(root, &["negligible"]);
    let cfg = load_config(root).unwrap().config;

    let input_no_rat = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Strict regulatory test".to_string(),
        safety_risk: "negligible".to_string(),
        rationale: None,
        origin_story_id: None,
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let err = add_deferred_work(root, &cfg, &input_no_rat).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert!(err.message().contains("regulatory.require_rationale_for"));

    let input_with_rat = AddDeferredWorkInput {
        target_module: "bridge".to_string(),
        title: "Strict regulatory test with rationale".to_string(),
        safety_risk: "negligible".to_string(),
        rationale: Some("Explained clearly.".to_string()),
        origin_story_id: None,
        gate: None,
        owners: Vec::new(),
        author: Author::new("human", "simon"),
    };

    let result = add_deferred_work(root, &cfg, &input_with_rat).unwrap();
    assert_eq!(result.safety_risk, "negligible");
    assert_eq!(result.rationale.as_deref(), Some("Explained clearly."));
}

#[test]
fn test_close_deferred_work_done_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let author = Author::new("human", "simon");
    let added = add_deferred_work(
        root,
        &cfg,
        &AddDeferredWorkInput {
            target_module: "bridge".to_string(),
            title: "Work to complete".to_string(),
            safety_risk: "negligible".to_string(),
            rationale: None,
            origin_story_id: None,
            gate: None,
            owners: Vec::new(),
            author: author.clone(),
        },
    )
    .unwrap();

    let close_input = CloseDeferredWorkInput {
        id: added.id.clone(),
        status: Some("done".to_string()),
        resolution: Some("Implemented in follow-up.".to_string()),
        justification: None,
        author: author.clone(),
    };

    let closed = close_deferred_work(root, Some(&cfg.storage), &close_input).unwrap();
    assert_eq!(closed.status, "done");
    assert_eq!(
        closed.resolution.as_deref(),
        Some("Implemented in follow-up.")
    );

    // Verify file updated
    let content = fs::read_to_string(root.join(format!("docs/state/dw/{}.md", added.id))).unwrap();
    assert!(content.contains("status: done"));
    assert!(content.contains("resolution: Implemented in follow-up."));

    // Verify store updated
    let store = open_store(root);
    let records = store.list_deferred_work().unwrap();
    let record = records.iter().find(|r| r.id == added.id).unwrap();
    assert_eq!(record.status.as_deref(), Some("done"));
    assert_eq!(
        record.resolution.as_deref(),
        Some("Implemented in follow-up.")
    );
}

#[test]
fn test_close_deferred_work_wont_fix_requires_justification_or_resolution() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let author = Author::new("human", "simon");
    let added = add_deferred_work(
        root,
        &cfg,
        &AddDeferredWorkInput {
            target_module: "bridge".to_string(),
            title: "Work to drop".to_string(),
            safety_risk: "negligible".to_string(),
            rationale: None,
            origin_story_id: None,
            gate: None,
            owners: Vec::new(),
            author: author.clone(),
        },
    )
    .unwrap();

    // Closing as wont_fix without justification/resolution must fail with UsageError
    let close_no_just = CloseDeferredWorkInput {
        id: added.id.clone(),
        status: Some("wont_fix".to_string()),
        resolution: None,
        justification: None,
        author: author.clone(),
    };

    let err = close_deferred_work(root, Some(&cfg.storage), &close_no_just).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);

    // Closing with justification succeeds
    let close_with_just = CloseDeferredWorkInput {
        id: added.id.clone(),
        status: Some("wont_fix".to_string()),
        resolution: None,
        justification: Some("Feature obsolete after architecture rethink.".to_string()),
        author: author.clone(),
    };

    let closed = close_deferred_work(root, Some(&cfg.storage), &close_with_just).unwrap();
    assert_eq!(closed.status, "wont_fix");
    assert_eq!(
        closed.resolution.as_deref(),
        Some("Feature obsolete after architecture rethink.")
    );

    let store = open_store(root);
    let records = store.list_deferred_work().unwrap();
    let record = records.iter().find(|r| r.id == added.id).unwrap();
    assert_eq!(record.status.as_deref(), Some("wont_fix"));
    assert_eq!(
        record.resolution.as_deref(),
        Some("Feature obsolete after architecture rethink.")
    );
}

#[test]
fn test_list_deferred_work_records_with_filters() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    write_story_file(
        root,
        "E1S1",
        r#"---
id: E1S1
title: "Story 1"
status: ready
version: 1
owners: [simon]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#,
    );

    let author = Author::new("human", "simon");

    let dw1 = add_deferred_work(
        root,
        &cfg,
        &AddDeferredWorkInput {
            target_module: "bridge".to_string(),
            title: "Bridge dw 1".to_string(),
            safety_risk: "negligible".to_string(),
            rationale: None,
            origin_story_id: Some("E1S1".to_string()),
            gate: None,
            owners: Vec::new(),
            author: author.clone(),
        },
    )
    .unwrap();

    let dw2 = add_deferred_work(
        root,
        &cfg,
        &AddDeferredWorkInput {
            target_module: "foundation".to_string(),
            title: "Foundation dw 2".to_string(),
            safety_risk: "acceptable_with_mitigation".to_string(),
            rationale: Some("Safety plan in place.".to_string()),
            origin_story_id: None,
            gate: None,
            owners: Vec::new(),
            author: author.clone(),
        },
    )
    .unwrap();

    let _dw3 = add_deferred_work(
        root,
        &cfg,
        &AddDeferredWorkInput {
            target_module: "bridge".to_string(),
            title: "Bridge dw 3".to_string(),
            safety_risk: "acceptable_with_mitigation".to_string(),
            rationale: Some("Double checked.".to_string()),
            origin_story_id: None,
            gate: None,
            owners: Vec::new(),
            author: author.clone(),
        },
    )
    .unwrap();

    // Close dw2
    close_deferred_work(
        root,
        Some(&cfg.storage),
        &CloseDeferredWorkInput {
            id: dw2.id.clone(),
            status: Some("done".to_string()),
            resolution: Some("Done".to_string()),
            justification: None,
            author: author.clone(),
        },
    )
    .unwrap();

    let store = open_store(root);

    // Filter by module
    let bridge_list = list_deferred_work_records(
        &store,
        &ListDeferredWorkFilter {
            module: Some("bridge".to_string()),
            risk: None,
            status: None,
            story: None,
        },
    )
    .unwrap();
    assert_eq!(bridge_list.len(), 2);

    // Filter by risk
    let risk_list = list_deferred_work_records(
        &store,
        &ListDeferredWorkFilter {
            module: None,
            risk: Some("acceptable_with_mitigation".to_string()),
            status: None,
            story: None,
        },
    )
    .unwrap();
    assert_eq!(risk_list.len(), 2);

    // Filter by status open
    let open_list = list_deferred_work_records(
        &store,
        &ListDeferredWorkFilter {
            module: None,
            risk: None,
            status: Some("open".to_string()),
            story: None,
        },
    )
    .unwrap();
    assert_eq!(open_list.len(), 2);

    // Filter by story
    let story_list = list_deferred_work_records(
        &store,
        &ListDeferredWorkFilter {
            module: None,
            risk: None,
            status: None,
            story: Some("E1S1".to_string()),
        },
    )
    .unwrap();
    assert_eq!(story_list.len(), 1);
    assert_eq!(story_list[0].id, dw1.id);
}

#[test]
fn test_story_transition_to_done_closes_referenced_dw() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_test_workspace(root);
    let cfg = load_config(root).unwrap().config;

    let author = Author::new("human", "simon");

    let dw = add_deferred_work(
        root,
        &cfg,
        &AddDeferredWorkInput {
            target_module: "bridge".to_string(),
            title: "Work to be resolved by E1S1".to_string(),
            safety_risk: "negligible".to_string(),
            rationale: None,
            origin_story_id: None,
            gate: None,
            owners: Vec::new(),
            author: author.clone(),
        },
    )
    .unwrap();

    // Create story E1S1 in `review` state that closes dw
    write_story_file(
        root,
        "E1S1",
        &format!(
            r#"---
id: E1S1
title: "Resolving story"
status: review
version: 1
owners: [simon]
target_modules: ["bridge"]
appetite: small
relations:
  closes_dw:
    - {}
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Verify resolution of deferred work.
"#,
            dw.id
        ),
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E1S1".to_string(),
        target_status: "done".to_string(),
        justification: None,
        author: author.clone(),
        if_version: None,
    };

    let result = engine.transition(&opts).unwrap();
    assert_eq!(result.to_status, "done");

    // Verify DW was closed in file
    let dw_file = root.join(format!("docs/state/dw/{}.md", dw.id));
    let content = fs::read_to_string(&dw_file).unwrap();
    assert!(
        content.contains("status: done"),
        "DW file status must be done: {}",
        content
    );
    assert!(
        content.contains("resolution: E1S1"),
        "DW file resolution must be E1S1: {}",
        content
    );

    // Verify store deferred_work table was updated
    let store = open_store(root);
    let records = store.list_deferred_work().unwrap();
    let record = records.iter().find(|r| r.id == dw.id).unwrap();
    assert_eq!(record.status.as_deref(), Some("done"));
    assert_eq!(record.resolution.as_deref(), Some("E1S1"));
}
