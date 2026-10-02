#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::Path;

use qdev_core::store::{SqliteStore, Store};
use qdev_core::{
    classify_mutation, close_sprint, get_refusal_entry, load_config, run_pre_push, Author,
    DeferredWorkRecord, EntityKind, EntityRecord, ExitCode, GateConfig, Interactivity,
    JsonErrorEnvelope, QdevError, RejectionAttribution, SprintCloseOptions, TransitionEngine,
    TransitionGateHook, TransitionOptions, REFUSAL_CATALOG,
};
use tempfile::TempDir;

fn setup_git_workspace(root: &Path) {
    for args in [
        vec!["init", "-q", "-b", "develop"],
        vec!["config", "user.name", "Test User"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        let _ = std::process::Command::new("git")
            .current_dir(root)
            .args(&args)
            .output();
    }

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
state_dir = "docs/state"

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]

[[modules]]
id = "foundation"
paths = ["crates/foundation/**"]
"#,
    )
    .unwrap();

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();

    let _ = std::process::Command::new("git")
        .current_dir(root)
        .args(["add", "."])
        .output();
    let _ = std::process::Command::new("git")
        .current_dir(root)
        .args(["commit", "-q", "-m", "initial"])
        .output();
}

fn write_story_file(root: &Path, id: &str, content: &str) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join(format!("{}.md", id)), content).unwrap();
}

#[test]
fn test_conformance_refusal_catalog_iteration() {
    // Assert every refusal error code in the binary is in the catalog and adheres to attribution schema
    assert!(
        !REFUSAL_CATALOG.is_empty(),
        "REFUSAL_CATALOG must not be empty"
    );

    let expected_codes = [
        "gate_failed",
        "gate_infra_failure",
        "already_leased",
        "lease_not_found",
        "needs_justification",
        "needs_confirmation",
        "story_blocked",
        "unacceptable_deferred_work",
        "active_story_lease",
        "sprint_already_closed",
        "sprint_not_active",
        "carry_over_not_allowed",
        "tty_required",
        "human_required",
        "preflight_refusal",
        "scope_violation",
        "policy_refusal",
    ];

    for code in expected_codes {
        let entry = get_refusal_entry(code);
        assert!(
            entry.is_some(),
            "Refusal error code '{}' must be present in REFUSAL_CATALOG",
            code
        );
        let entry = entry.unwrap();
        assert!(
            !entry.default_rule.is_empty(),
            "Rule text must not be empty"
        );
        assert!(
            entry.required_fields.contains(&"rule"),
            "Entry '{}' must require 'rule' field",
            code
        );
        assert!(
            entry.required_fields.iter().any(|f| matches!(
                *f,
                "constraint_id" | "gate_id" | "policy" | "blocking_ids" | "holder"
            )),
            "Entry '{}' must require at least one structured attribution key",
            code
        );

        // Construct sample attribution satisfying entry requirements
        let mut attr = RejectionAttribution::new(entry.default_rule);
        for field in entry.required_fields {
            match *field {
                "constraint_id" => attr = attr.with_constraint_id("E12S4/NG-1"),
                "gate_id" => attr = attr.with_gate_id("qdev-scope"),
                "policy" => attr = attr.with_policy("target_modules"),
                "blocking_ids" => attr = attr.with_blocking_ids(vec!["E12S1".to_string()]),
                "holder" => attr = attr.with_holder("amelia"),
                _ => {}
            }
        }

        assert!(attr.has_attribution());

        let error = QdevError::new(entry.exit_code, entry.code, "Sample refusal error")
            .with_attribution(attr.clone());

        assert_eq!(error.exit_code(), entry.exit_code);
        assert_eq!(error.code(), entry.code);

        let details = error.details().expect("details must be present");
        let details_obj = details.as_object().expect("details must be JSON object");

        // Rule must be present and non-empty
        assert!(
            details_obj
                .get("rule")
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty()),
            "rule must be present and non-empty for code '{}'",
            code
        );

        // At least one attribution key must be present
        let has_key = details_obj.contains_key("constraint_id")
            || details_obj.contains_key("gate_id")
            || details_obj.contains_key("policy")
            || details_obj.contains_key("blocking_ids")
            || details_obj.contains_key("holder");
        assert!(
            has_key,
            "Error details for code '{}' must contain at least one attribution key",
            code
        );

        // All required fields must be present
        for req in entry.required_fields {
            assert!(
                details_obj.contains_key(*req),
                "Error details for code '{}' must contain required field '{}'",
                code,
                req
            );
        }

        // Must serialize to AD-13 JSON envelope on stdout
        let envelope = JsonErrorEnvelope::from(&error);
        assert_eq!(envelope.schema_version, "1");
        assert_eq!(envelope.error.code, entry.code);
        let envelope_json = serde_json::to_string(&envelope).unwrap();
        assert!(envelope_json.contains("\"schema_version\":\"1\""));
        assert!(envelope_json.contains("\"rule\":"));
    }
}

#[test]
fn test_conformance_path_gate_fail_and_infra() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    let mut annotated = load_config(root).unwrap();
    // Add external failing gate
    annotated.config.gates.push(GateConfig {
        id: "mock-fail-gate".to_string(),
        command: Some("false".to_string()),
        timeout_ms: Some(5000),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: vec!["review".to_string()],
        verifies: Vec::new(),
        kind: Some("test".to_string()),
        metric: None,
        direction: None,
        skip: None,
    });

    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(TransitionGateHook::new(annotated.config));

    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: false,
        interactivity: Interactivity::Interactive,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "gate_failed");

    let details = err.details().expect("details must be present");
    assert_eq!(details["gate_id"], "mock-fail-gate");
    assert!(!details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_gate_infra_failure() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
appetite: half-day
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    let mut annotated = load_config(root).unwrap();
    annotated.config.gates.push(GateConfig {
        id: "mock-infra-gate".to_string(),
        command: Some("sleep 2".to_string()),
        timeout_ms: Some(5),
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: vec!["review".to_string()],
        verifies: Vec::new(),
        kind: Some("test".to_string()),
        metric: None,
        direction: None,
        skip: None,
    });

    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(TransitionGateHook::new(annotated.config));

    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: false,
        interactivity: Interactivity::Interactive,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::InfrastructureFailure);
    assert_eq!(err.code(), "gate_infra_failure");

    let details = err.details().expect("details must be present");
    assert_eq!(details["gate_id"], "mock-infra-gate");
    assert_eq!(details["policy"], "gate_infrastructure");
    assert!(!details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_scope_violation_with_no_go() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
appetite: small
target_modules: ["bridge"]
constraints:
  - id: NG-2
    kind: no_go
    text: Do not touch frame buffers
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    // Touch frame buffers outside target_modules
    let video_dir = root.join("crates/video");
    fs::create_dir_all(&video_dir).unwrap();
    fs::write(video_dir.join("frame.rs"), "// video frame buffer\n").unwrap();

    let config = load_config(root).unwrap().config;
    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(TransitionGateHook::new(config));

    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: false,
        interactivity: Interactivity::Interactive,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "gate_failed");

    let details = err.details().expect("details must be present");
    assert_eq!(details["gate_id"], "qdev-scope");
    assert_eq!(details["constraint_id"], "E12S4/NG-2");
    assert!(!details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_scope_violation_without_no_go() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    // Touch path outside target_modules with no matching no-go
    let other_dir = root.join("crates/other");
    fs::create_dir_all(&other_dir).unwrap();
    fs::write(other_dir.join("lib.rs"), "// out of scope\n").unwrap();

    let config = load_config(root).unwrap().config;
    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(TransitionGateHook::new(config));

    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: false,
        interactivity: Interactivity::Interactive,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "gate_failed");

    let details = err.details().expect("details must be present");
    assert_eq!(details["gate_id"], "qdev-scope");
    assert_eq!(details["policy"], "target_modules");
    assert!(!details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_lease_conflict_claim_and_release() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
created_by:
  type: human
  id: amelia
updated_by:
  type: human
  id: amelia
---
## Acceptance Criteria
- AC1
"#,
    );

    // 1. Claim by amelia succeeds
    let author_amelia = Author::new("human", "amelia");
    qdev_core::claim_story(root, "E12S4", &author_amelia, None, None).unwrap();

    // 2. Second claim by bob fails with exit 5 conflict (already_leased)
    let author_bob = Author::new("human", "bob");
    let claim_err = qdev_core::claim_story(root, "E12S4", &author_bob, None, None).unwrap_err();
    assert_eq!(claim_err.exit_code(), ExitCode::Conflict);
    assert_eq!(claim_err.code(), "already_leased");

    let details = claim_err.details().expect("details present");
    assert_eq!(details["holder"], "amelia");
    assert_eq!(details["policy"], "single_lease_holder");
    assert!(!details["rule"].as_str().unwrap().is_empty());

    // 3. Release by bob without force fails with exit 3 (policy_refusal)
    let release_err =
        qdev_core::release_story(root, "E12S4", &author_bob, false, None, None, None).unwrap_err();
    assert_eq!(release_err.exit_code(), ExitCode::PolicyRefusal);

    let r_details = release_err.details().expect("details present");
    assert_eq!(r_details["holder"], "amelia");
    assert_eq!(r_details["policy"], "lease_ownership");
    assert!(!r_details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_missing_justification() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: review
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    let config = load_config(root).unwrap().config;
    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(TransitionGateHook::new(config.clone()));

    // Backward transition from review to in-progress without justification
    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "in-progress".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: false,
        interactivity: Interactivity::Interactive,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_justification");

    let details = err.details().expect("details present");
    assert_eq!(details["policy"], "justification_required");
    assert!(!details["rule"].as_str().unwrap().is_empty());

    // Skip gates without justification
    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    let skip_opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: true,
        interactivity: Interactivity::Interactive,
    };

    let skip_err = engine.transition(&skip_opts).unwrap_err();
    assert_eq!(skip_err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(skip_err.code(), "needs_justification");

    let s_details = skip_err.details().expect("details present");
    assert_eq!(s_details["policy"], "justification_required");
    assert!(!s_details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_blocked_dependency() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    // [E12S1] Dependency in ready (not done)
    write_story_file(
        root,
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    // [E12S4] Target depends on E12S1
    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
relations:
  depends_on: ["E12S1"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    // Populate sqlite cache with status ready for E12S1
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();

    let rec1 = EntityRecord {
        id: "E12S1".to_string(),
        kind: EntityKind::Story,
        title: Some("Story 1".to_string()),
        status: Some("ready".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S1.md".to_string(),
        content_hash: "dummyhash1".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-12T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: Some("small".to_string()),
        safety_class: None,
        target_modules: None,
    };
    let rec4 = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Story 4".to_string()),
        status: Some("ready".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "dummyhash4".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-12T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: Some("small".to_string()),
        safety_class: None,
        target_modules: None,
    };
    store.upsert_entity(&rec1).unwrap();
    store.upsert_entity(&rec4).unwrap();

    let _config = load_config(root).unwrap().config;
    let engine = TransitionEngine::new();

    let opts = TransitionOptions {
        workspace_root: root.to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "in-progress".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
        skip_gates: false,
        interactivity: Interactivity::Interactive,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "story_blocked");

    let details = err.details().expect("details present");
    assert_eq!(details["policy"], "dependency_order");
    assert_eq!(details["blocking_ids"], serde_json::json!(["E12S1"]));
    assert!(!details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_sprint_close_refusals() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    let author = Author::new("human", "simon");
    let mut config = load_config(root).unwrap().config;
    config.storage.state_dir = "docs/state".to_string();

    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();

    // 1. Open sprint 5
    let sprint_opts = qdev_core::sprint::SprintOpenOptions {
        workspace_root: root,
        storage: &config.storage,
        store: &store,
        sprint: 5,
        title: "Sprint 5",
        release: None,
        author: &author,
    };
    let _open_res = qdev_core::open_sprint(&sprint_opts).unwrap();

    // Add unacceptable DW without rationale
    let dw_rec = DeferredWorkRecord {
        id: "DW-9999".to_string(),
        safety_risk: Some("unacceptable".to_string()),
        status: Some("open".to_string()),
        ..Default::default()
    };
    store.upsert_deferred_work(&dw_rec).unwrap();

    let close_opts = SprintCloseOptions {
        workspace_root: root,
        storage: &config.storage,
        store: &store,
        sprint: 5,
        status: "completed",
        carry_over_target: None,
        reason: None,
        author: &author,
        gates: None,
        integration_branch: None,
    };

    let dw_err = close_sprint(&close_opts).unwrap_err();
    assert_eq!(dw_err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(dw_err.code(), "unacceptable_deferred_work");

    let dw_details = dw_err.details().expect("details present");
    assert_eq!(dw_details["policy"], "deferred_work_rationale");
    assert_eq!(dw_details["blocking_ids"], serde_json::json!(["DW-9999"]));
    assert!(!dw_details["rule"].as_str().unwrap().is_empty());

    // Resolve DW
    let mut resolved_dw = dw_rec.clone();
    resolved_dw.rationale = Some("Approved safety mitigations in place".to_string());
    store.upsert_deferred_work(&resolved_dw).unwrap();

    // Add active story lease assigned to sprint
    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );
    let s_rec = EntityRecord {
        id: "E12S4".to_string(),
        kind: EntityKind::Story,
        title: Some("Story 4".to_string()),
        status: Some("in-progress".to_string()),
        owners: None,
        source_path: "docs/specs/stories/E12S4.md".to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-12T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: Some("small".to_string()),
        safety_class: None,
        target_modules: None,
    };
    store.upsert_entity(&s_rec).unwrap();
    qdev_core::assign_to_sprint(&qdev_core::SprintAssignOptions {
        workspace_root: root,
        storage: &config.storage,
        store: &store,
        sprint: 5,
        stories: &["E12S4".to_string()],
        author: &author,
    })
    .unwrap();

    // Claim lease
    qdev_core::claim_story(root, "E12S4", &author, Some(&config.storage), None).unwrap();

    let lease_err = close_sprint(&close_opts).unwrap_err();
    assert_eq!(lease_err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(lease_err.code(), "active_story_lease");

    let l_details = lease_err.details().expect("details present");
    assert_eq!(l_details["policy"], "lease_lifecycle");
    assert_eq!(l_details["holder"], "simon");
    assert!(!l_details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_preflight_push_refusal() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    // [E12S4] Bridge module leased
    write_story_file(
        root,
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: in-progress
version: 1
appetite: small
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
## Acceptance Criteria
- AC1
"#,
    );

    let author = Author::new("human", "simon");
    let config = load_config(root).unwrap().config;
    qdev_core::claim_story(root, "E12S4", &author, None, None).unwrap();

    // Modify a file outside target_modules
    let out_dir = root.join("crates/foundation");
    fs::create_dir_all(&out_dir).unwrap();
    fs::write(out_dir.join("lib.rs"), "// out of scope\n").unwrap();

    let err = run_pre_push(root, &config, &[], None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "preflight_refusal");

    let details = err.details().expect("details present");
    assert_eq!(details["policy"], "preflight_guard");
    assert!(!details["rule"].as_str().unwrap().is_empty());
}

#[test]
fn test_conformance_path_cross_team_and_out_of_lease_governance() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    setup_git_workspace(root);

    // Story E12 owned by infra
    write_story_file(
        root,
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: in-progress
version: 1
owners: ["team:infra"]
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
---
## Acceptance Criteria
- AC1
"#,
    );

    let config = load_config(root).unwrap().config;
    let author_frontend = Author::new("human", "bob"); // bob is not in team infra

    let classification = classify_mutation(
        root,
        "E12S1",
        Some(EntityKind::Story),
        &author_frontend,
        &config,
        None,
    )
    .unwrap();

    assert!(classification.is_cross_team);

    // Emulate CLI governance check emission for non-interactive cross-team mutation
    let (policy, rule) = if classification.is_cross_team {
        (
            "cross_team_governance",
            "Non-interactive mutations crossing team ownership boundaries require override and justification",
        )
    } else {
        ("lease_scope", "Out-of-lease mutation")
    };

    let attr = RejectionAttribution::new(rule).with_policy(policy);
    let err = QdevError::policy_refusal(
        "needs_confirmation",
        "Cross-team mutation on entity 'E12S1' requires confirmation",
    )
    .with_details(serde_json::json!({
        "target_id": "E12S1",
        "is_cross_team": true,
    }))
    .with_attribution(attr);

    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_confirmation");
    let details = err.details().expect("details present");
    assert_eq!(details["policy"], "cross_team_governance");
    assert_eq!(
        details["rule"],
        "Non-interactive mutations crossing team ownership boundaries require override and justification"
    );
}
