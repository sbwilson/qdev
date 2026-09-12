#![allow(clippy::disallowed_methods)]

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use qdev_core::store::{SqliteStore, Store};
use qdev_core::{
    acquire_write_lock, classify_transition, Author, EntityKind, EntityRecord, ExitCode, QdevError,
    StoryState, TransitionContext, TransitionEngine, TransitionKind, TransitionOptions,
};
use tempfile::TempDir;

fn setup_story_workspace(root: &Path) {
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

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    let cache_dir = root.join(".qdev/cache");
    fs::create_dir_all(&cache_dir).unwrap();
}

fn write_story_file(root: &Path, id: &str, content: &str) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::write(stories_dir.join(format!("{}.md", id)), content).unwrap();
}

fn write_dw_file(root: &Path, id: &str, content: &str) {
    let dw_dir = root.join("docs/state/dw");
    fs::create_dir_all(&dw_dir).unwrap();
    fs::write(dw_dir.join(format!("{}.md", id)), content).unwrap();
}

fn populate_cache_for_story(root: &Path, story_id: &str, status: &str) {
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db).unwrap();
    store
        .with_conn(|conn| {
            qdev_core::create_schema(conn).unwrap();
            Ok(())
        })
        .unwrap();
    let record = EntityRecord {
        id: story_id.to_string(),
        kind: EntityKind::Story,
        title: Some(format!("Story {}", story_id)),
        status: Some(status.to_string()),
        owners: None,
        source_path: format!("docs/specs/stories/{}.md", story_id),
        content_hash: "dummyhash".to_string(),
        version: 1,
        created_by: None,
        updated_by: None,
        updated_at: "2026-09-12T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: Some("small".to_string()),
        safety_class: None,
        target_modules: Some("[\"bridge\"]".to_string()),
    };
    store.upsert_entity(&record).unwrap();
}

#[test]
fn test_story_state_parse_and_properties() {
    assert_eq!(StoryState::parse("draft").unwrap(), StoryState::Draft);
    assert_eq!(StoryState::parse("Draft").unwrap(), StoryState::Draft);
    assert_eq!(StoryState::parse("ready").unwrap(), StoryState::Ready);
    assert_eq!(
        StoryState::parse("in-progress").unwrap(),
        StoryState::InProgress
    );
    assert_eq!(
        StoryState::parse("in_progress").unwrap(),
        StoryState::InProgress
    );
    assert_eq!(
        StoryState::parse("IN_PROGRESS").unwrap(),
        StoryState::InProgress
    );
    assert_eq!(StoryState::parse("review").unwrap(), StoryState::Review);
    assert_eq!(StoryState::parse("done").unwrap(), StoryState::Done);
    assert_eq!(
        StoryState::parse("superseded").unwrap(),
        StoryState::Superseded
    );
    assert_eq!(
        StoryState::parse("abandoned").unwrap(),
        StoryState::Abandoned
    );

    let err = StoryState::parse("unknown").unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);

    assert!(!StoryState::Draft.is_terminal());
    assert!(!StoryState::Ready.is_terminal());
    assert!(!StoryState::InProgress.is_terminal());
    assert!(!StoryState::Review.is_terminal());
    assert!(StoryState::Done.is_terminal());
    assert!(StoryState::Superseded.is_terminal());
    assert!(StoryState::Abandoned.is_terminal());

    assert_eq!(StoryState::Draft.as_str(), "draft");
    assert_eq!(StoryState::InProgress.as_str(), "in-progress");
}

#[test]
fn test_classify_transition() {
    // Legal forward edges
    assert_eq!(
        classify_transition(StoryState::Draft, StoryState::Ready).unwrap(),
        TransitionKind::LegalForward
    );
    assert_eq!(
        classify_transition(StoryState::Ready, StoryState::InProgress).unwrap(),
        TransitionKind::LegalForward
    );
    assert_eq!(
        classify_transition(StoryState::InProgress, StoryState::Review).unwrap(),
        TransitionKind::LegalForward
    );
    assert_eq!(
        classify_transition(StoryState::Review, StoryState::Done).unwrap(),
        TransitionKind::LegalForward
    );

    // Terminal jumps
    assert_eq!(
        classify_transition(StoryState::Draft, StoryState::Superseded).unwrap(),
        TransitionKind::TerminalJump
    );
    assert_eq!(
        classify_transition(StoryState::Ready, StoryState::Abandoned).unwrap(),
        TransitionKind::TerminalJump
    );
    assert_eq!(
        classify_transition(StoryState::InProgress, StoryState::Abandoned).unwrap(),
        TransitionKind::TerminalJump
    );
    assert_eq!(
        classify_transition(StoryState::Review, StoryState::Superseded).unwrap(),
        TransitionKind::TerminalJump
    );

    // Backward transitions
    assert_eq!(
        classify_transition(StoryState::Review, StoryState::InProgress).unwrap(),
        TransitionKind::Backward
    );
    assert_eq!(
        classify_transition(StoryState::InProgress, StoryState::Ready).unwrap(),
        TransitionKind::Backward
    );
    assert_eq!(
        classify_transition(StoryState::Ready, StoryState::Draft).unwrap(),
        TransitionKind::Backward
    );
    assert_eq!(
        classify_transition(StoryState::Review, StoryState::Draft).unwrap(),
        TransitionKind::Backward
    );

    // Invalid transitions: same state
    let err = classify_transition(StoryState::Ready, StoryState::Ready).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "invalid_transition");

    // Invalid transitions: out of terminal
    let err = classify_transition(StoryState::Done, StoryState::Ready).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "invalid_transition");

    let err = classify_transition(StoryState::Abandoned, StoryState::Draft).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "invalid_transition");

    // Invalid forward skips
    let err = classify_transition(StoryState::Draft, StoryState::Review).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "invalid_transition");

    let err = classify_transition(StoryState::Draft, StoryState::Done).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);

    let err = classify_transition(StoryState::Ready, StoryState::Done).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
}

#[test]
fn test_transition_draft_to_ready_prerequisites_missing_ac() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Story lacks ## Acceptance Criteria heading
    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: draft
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

## Description
Some description without acceptance criteria.
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "readiness_criteria_unmet");
    assert!(err
        .message()
        .contains("missing '## Acceptance Criteria' section"));
}

#[test]
fn test_transition_draft_to_ready_prerequisites_missing_appetite() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Story lacks appetite
    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: draft
version: 1
target_modules: ["bridge"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Criteria 1
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "readiness_criteria_unmet");
    assert!(err.message().contains("appetite"));
}

#[test]
fn test_transition_draft_to_ready_prerequisites_missing_target_modules() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Story has empty target_modules
    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: draft
version: 1
appetite: small
target_modules: []
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Criteria 1
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "readiness_criteria_unmet");
    assert!(err.message().contains("target_modules"));
}

#[test]
fn test_transition_draft_to_ready_happy_path() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: draft
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
- Criteria 1
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let payload = engine.transition(&opts).unwrap();
    assert_eq!(payload.id, "E12S1");
    assert_eq!(payload.from_status, "draft");
    assert_eq!(payload.to_status, "ready");
    assert_eq!(payload.version, 2);

    // Verify file content was mutated
    let updated_content =
        fs::read_to_string(tmp.path().join("docs/specs/stories/E12S1.md")).unwrap();
    assert!(updated_content.contains("status: ready"));
    assert!(updated_content.contains("version: 2"));
}

#[test]
fn test_transition_ready_to_in_progress_blocked() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Dependency story E12S1 in ready (not done)
    populate_cache_for_story(tmp.path(), "E12S1", "ready");

    write_story_file(
        tmp.path(),
        "E12S2",
        r#"---
id: E12S2
title: Story 2
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
relations:
  depends_on:
    - E12S1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Criteria 2
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S2".to_string(),
        target_status: "in-progress".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "story_blocked");
    assert!(err.message().contains("E12S1"));

    let details = err.details().expect("details must be present");
    let blocking = details["blocking_ids"]
        .as_array()
        .expect("blocking_ids array");
    assert_eq!(blocking.len(), 1);
    assert_eq!(blocking[0], "E12S1");
}

#[test]
fn test_transition_ready_to_in_progress_unblocked() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Dependency story E12S1 is done
    populate_cache_for_story(tmp.path(), "E12S1", "done");

    write_story_file(
        tmp.path(),
        "E12S2",
        r#"---
id: E12S2
title: Story 2
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
relations:
  depends_on:
    - E12S1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Criteria 2
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S2".to_string(),
        target_status: "in-progress".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let payload = engine.transition(&opts).unwrap();
    assert_eq!(payload.from_status, "ready");
    assert_eq!(payload.to_status, "in-progress");
}

#[test]
fn test_terminal_transition_requires_justification() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
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
- AC
"#,
    );

    let engine = TransitionEngine::new();

    // Abandoned without justification
    let opts_no_just = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "abandoned".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts_no_just).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_justification");

    // Abandoned with empty justification
    let opts_empty_just = TransitionOptions {
        justification: Some("   ".to_string()),
        ..opts_no_just.clone()
    };
    let err2 = engine.transition(&opts_empty_just).unwrap_err();
    assert_eq!(err2.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err2.code(), "needs_justification");

    // Abandoned with valid justification
    let opts_with_just = TransitionOptions {
        justification: Some("Superseded by bet E14".to_string()),
        ..opts_no_just
    };
    let payload = engine.transition(&opts_with_just).unwrap();
    assert_eq!(payload.to_status, "abandoned");
    assert_eq!(payload.version, 2);
}

#[test]
fn test_backward_transition_requires_justification() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: review
version: 3
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
- AC
"#,
    );

    let engine = TransitionEngine::new();

    // review -> in-progress without justification
    let opts_no_just = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "in-progress".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts_no_just).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_justification");

    // review -> in-progress with justification
    let opts_with_just = TransitionOptions {
        justification: Some("Review rejected due to failing edge case".to_string()),
        ..opts_no_just
    };
    let payload = engine.transition(&opts_with_just).unwrap();
    assert_eq!(payload.from_status, "review");
    assert_eq!(payload.to_status, "in-progress");
    assert_eq!(payload.version, 4);
}

#[test]
fn test_transition_out_of_terminal_state_rejected() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: done
version: 5
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
- AC
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: Some("Reopening".to_string()),
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "invalid_transition");
}

#[test]
fn test_transition_to_done_closes_deferred_work() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Create deferred work entity DW-7f3a
    write_dw_file(
        tmp.path(),
        "DW-7f3a",
        r#"---
id: DW-7f3a
title: "Cleanup buffer allocation"
status: open
version: 1
origin_story_id: E12S1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Description
Deferral description.
"#,
    );

    // Create story in review with relations.closes_dw
    write_story_file(
        tmp.path(),
        "E12S4",
        r#"---
id: E12S4
title: Story 4
status: review
version: 2
appetite: small
target_modules: ["bridge"]
relations:
  closes_dw:
    - DW-7f3a
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "done".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let payload = engine.transition(&opts).unwrap();
    assert_eq!(payload.id, "E12S4");
    assert_eq!(payload.to_status, "done");
    assert_eq!(payload.closed_dw, vec!["DW-7f3a".to_string()]);

    // Verify DW-7f3a was updated to status: done and resolution: E12S4
    let dw_content = fs::read_to_string(tmp.path().join("docs/state/dw/DW-7f3a.md")).unwrap();
    assert!(dw_content.contains("status: done"));
    assert!(dw_content.contains("resolution: E12S4"));
}

#[test]
fn test_transition_pre_and_post_hooks() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
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
- AC
"#,
    );

    let pre_called = Arc::new(Mutex::new(Vec::new()));
    let post_called = Arc::new(Mutex::new(Vec::new()));

    let pre_rec = pre_called.clone();
    let post_rec = post_called.clone();

    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(move |ctx: &TransitionContext| {
        pre_rec
            .lock()
            .unwrap()
            .push(format!("pre:{}:{}", ctx.story_id, ctx.to_state));
        Ok(())
    });
    engine.add_post_hook(
        move |ctx: &TransitionContext, res: &qdev_core::EntityUpdateResult| {
            post_rec
                .lock()
                .unwrap()
                .push(format!("post:{}:v{}", ctx.story_id, res.new_version));
            Ok(())
        },
    );

    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let payload = engine.transition(&opts).unwrap();
    assert_eq!(payload.to_status, "review");

    assert_eq!(*pre_called.lock().unwrap(), vec!["pre:E12S1:review"]);
    assert_eq!(*post_called.lock().unwrap(), vec!["post:E12S1:v2"]);
}

#[test]
fn test_pre_transition_hook_abort_executes_zero_mutations() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    let original_content = r#"---
id: E12S1
title: Story 1
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
- AC
"#;
    write_story_file(tmp.path(), "E12S1", original_content);

    let post_called = Arc::new(AtomicBool::new(false));
    let post_flag = post_called.clone();

    let mut engine = TransitionEngine::new();
    engine.add_pre_hook(|_ctx: &TransitionContext| {
        Err(QdevError::logical_failure(
            "gate_verification_failed",
            "Gate review failed",
        ))
    });
    engine.add_post_hook(
        move |_ctx: &TransitionContext, _res: &qdev_core::EntityUpdateResult| {
            post_flag.store(true, Ordering::SeqCst);
            Ok(())
        },
    );

    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "gate_verification_failed");

    // Post hook must NOT have run
    assert!(!post_called.load(Ordering::SeqCst));

    // Story file must be untouched
    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S1.md")).unwrap();
    assert_eq!(content, original_content);
}

#[test]
fn test_lock_contention_fails_with_exit_5() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
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
- AC
"#,
    );

    let lock_path = tmp.path().join(".qdev/cache/write.lock");
    let lock_held = Arc::new(AtomicBool::new(false));
    let release_lock = Arc::new(AtomicBool::new(false));

    let lock_held_clone = lock_held.clone();
    let release_lock_clone = release_lock.clone();
    let lock_path_clone = lock_path.clone();

    let bg_thread = thread::spawn(move || {
        let _guard = acquire_write_lock(&lock_path_clone, Duration::from_millis(1000)).unwrap();
        lock_held_clone.store(true, Ordering::SeqCst);
        while !release_lock_clone.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(50));
        }
    });

    while !lock_held.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(10));
    }

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "review".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    // apply_entity_update will wait up to 5000ms and time out
    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "lock_timeout");

    release_lock.store(true, Ordering::SeqCst);
    bg_thread.join().unwrap();
}

#[test]
fn test_transition_ready_to_in_progress_blocked_when_cache_absent() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Do NOT create or populate cache.sqlite
    let cache_db = tmp.path().join(".qdev/cache/cache.sqlite");
    if cache_db.exists() {
        fs::remove_file(cache_db).unwrap();
    }

    write_story_file(
        tmp.path(),
        "E12S2",
        r#"---
id: E12S2
title: Story 2
status: ready
version: 1
appetite: small
target_modules: ["bridge"]
relations:
  depends_on:
    - E12S1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Criteria 2
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S2".to_string(),
        target_status: "in-progress".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "story_blocked");
    assert!(err.message().contains("E12S1"));
}

#[test]
fn test_transition_draft_to_ready_frontmatter_comment_does_not_satisfy_ac() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    // Frontmatter contains a comment '# Acceptance Criteria', but the body does not have heading
    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: draft
version: 1
appetite: small
target_modules: ["bridge"]
# Acceptance Criteria
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Description
Only description here, no AC section in markdown body.
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "readiness_criteria_unmet");
    assert!(err
        .message()
        .contains("missing '## Acceptance Criteria' section"));
}

#[test]
fn test_transition_to_done_missing_dw_target_pre_validation_fails_without_story_mutation() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    let original_story_content = r#"---
id: E12S4
title: Story 4
status: review
version: 2
appetite: small
target_modules: ["bridge"]
relations:
  closes_dw:
    - DW-nonexistent
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC
"#;
    write_story_file(tmp.path(), "E12S4", original_story_content);

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S4".to_string(),
        target_status: "done".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("Entity file not found for 'DW-nonexistent'"));

    // Story file must remain untouched in review status at version 2
    let content = fs::read_to_string(tmp.path().join("docs/specs/stories/E12S4.md")).unwrap();
    assert_eq!(content, original_story_content);
}

#[test]
fn test_transition_invalid_status_on_disk_returns_logical_failure_exit_1() {
    let tmp = TempDir::new().unwrap();
    setup_story_workspace(tmp.path());

    write_story_file(
        tmp.path(),
        "E12S1",
        r#"---
id: E12S1
title: Story 1
status: corrupted_status_value
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
- AC
"#,
    );

    let engine = TransitionEngine::new();
    let opts = TransitionOptions {
        workspace_root: tmp.path().to_path_buf(),
        storage: None,
        entity_kind: "story".to_string(),
        story_id: "E12S1".to_string(),
        target_status: "ready".to_string(),
        justification: None,
        author: Author::new("human", "simon"),
        if_version: None,
    };

    let err = engine.transition(&opts).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "invalid_status");
}

