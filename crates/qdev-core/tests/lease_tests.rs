use std::fs;
use std::path::Path;

use qdev_core::config::LeasesConfig;
use qdev_core::lease::{
    claim_story, discover_git_common_dir, find_active_lease, get_lease, release_story,
};
use qdev_core::store::sqlite::SqliteStore;
use qdev_core::transition::{TransitionEngine, TransitionOptions};
use qdev_core::write::Author;
use qdev_core::{DoctorSection, ExitCode};
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();

    // Create qdev.toml
    fs::write(
        root.join("qdev.toml"),
        r#"[project]
name = "test"

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"
"#,
    )
    .unwrap();

    // Initialize cache database
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
    qdev_core::create_schema(&conn).unwrap();
    qdev_core::stamp_cache_version(&conn).unwrap();
}

fn write_story(root: &Path, story_id: &str, title: &str, status: &str) {
    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    let content = format!(
        r#"---
id: {story_id}
title: "{title}"
status: {status}
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC1.
"#
    );
    fs::write(stories_dir.join(format!("{}.md", story_id)), content).unwrap();
}

#[test]
fn test_claim_unleased_story_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    fs::create_dir_all(root.join(".git")).unwrap();
    let author = Author::new("human", "simon");
    let lease = claim_story(root, "E12S4", &author, None, None).unwrap();

    assert_eq!(lease.story_id, "E12S4");
    assert_eq!(lease.holder, "simon");
    assert_eq!(lease.author_type, "human");
    assert!(lease.session_token.starts_with("qs_E12S4_"));

    // Verify local lease file
    let local_file = root.join(".qdev/leases/E12S4.json");
    assert!(local_file.is_file());
    let local_content = fs::read_to_string(&local_file).unwrap();
    assert!(local_content.contains("\"story_id\": \"E12S4\""));
    assert!(local_content.contains("\"holder\": \"simon\""));

    // Verify shared mirror file
    let git_common = discover_git_common_dir(root);
    let shared_file = git_common.join("qdev/leases/E12S4.json");
    assert!(shared_file.is_file());
}

#[test]
fn test_claim_nonexistent_story_fails_entity_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let author = Author::new("human", "simon");
    let err = claim_story(root, "E99S99", &author, None, None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "entity_not_found");
}

#[test]
fn test_claim_already_leased_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    let author = Author::new("human", "simon");
    let _ = claim_story(root, "E12S4", &author, None, None).unwrap();

    // Second claim should fail with conflict (code already_leased, exit 5)
    let author2 = Author::new("agent", "worker-1");
    let err = claim_story(root, "E12S4", &author2, None, None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "already_leased");
    assert!(err.message().contains("simon"));

    let details = err.details().expect("details must be present");
    assert_eq!(details["story_id"], "E12S4");
    assert_eq!(details["holder"], "simon");
}

#[test]
fn test_release_own_lease_success() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    fs::create_dir_all(root.join(".git")).unwrap();
    let author = Author::new("human", "simon");
    let _ = claim_story(root, "E12S4", &author, None, None).unwrap();

    let res = release_story(root, "E12S4", &author, false, None, None, None).unwrap();
    assert_eq!(res.story_id, "E12S4");
    assert!(res.released);
    assert!(res.decision_id.is_none());

    // Both local and shared files should be deleted
    assert!(!root.join(".qdev/leases/E12S4.json").exists());
    let git_common = discover_git_common_dir(root);
    assert!(!git_common.join("qdev/leases/E12S4.json").exists());
}

#[test]
fn test_release_unleased_story_fails_lease_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let author = Author::new("human", "simon");
    let err = release_story(root, "E12S4", &author, false, None, None, None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "lease_not_found");
}

#[test]
fn test_find_active_lease_single_and_multiple() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S1", "Story 1", "in-progress");
    write_story(root, "E12S2", "Story 2", "in-progress");

    // Empty workspace
    let err = find_active_lease(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "no_active_lease");

    // 1 lease
    let author = Author::new("human", "simon");
    claim_story(root, "E12S1", &author, None, None).unwrap();
    let active = find_active_lease(root).unwrap();
    assert_eq!(active.story_id, "E12S1");

    // 2 leases
    claim_story(root, "E12S2", &author, None, None).unwrap();
    let err = find_active_lease(root).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_release_other_holder_refused_without_force() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    let author_simon = Author::new("human", "simon");
    claim_story(root, "E12S4", &author_simon, None, None).unwrap();

    // Release by alice without force
    let author_alice = Author::new("human", "alice");
    let err = release_story(root, "E12S4", &author_alice, false, None, None, None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "policy_refusal");
    assert!(err.message().contains("--force --justification"));
}

#[test]
fn test_release_with_force_missing_justification() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    let author_simon = Author::new("human", "simon");
    claim_story(root, "E12S4", &author_simon, None, None).unwrap();

    let author_alice = Author::new("human", "alice");
    let err = release_story(root, "E12S4", &author_alice, true, None, None, None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err.code(), "needs_justification");

    let err2 = release_story(root, "E12S4", &author_alice, true, Some("   "), None, None).unwrap_err();
    assert_eq!(err2.exit_code(), ExitCode::PolicyRefusal);
    assert_eq!(err2.code(), "needs_justification");
}

#[test]
fn test_release_with_force_and_justification_creates_decision() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "in-progress");

    let author_simon = Author::new("human", "simon");
    claim_story(root, "E12S4", &author_simon, None, None).unwrap();

    let author_alice = Author::new("human", "alice");
    let res = release_story(
        root,
        "E12S4",
        &author_alice,
        true,
        Some("Agent crashed and left orphan lease"),
        None,
        None,
    )
    .unwrap();

    assert_eq!(res.story_id, "E12S4");
    assert!(res.released);
    let dec_id = res.decision_id.expect("decision_id must be recorded");
    assert!(dec_id.starts_with("DEC-"));

    // Verify DEC- file was created in docs/state/decisions
    let dec_dir = root.join("docs/state/decisions");
    let dec_file = dec_dir.join(format!("{}.md", dec_id));
    assert!(dec_file.is_file(), "Decision file must exist: {}", dec_file.display());
    let dec_content = fs::read_to_string(&dec_file).unwrap();
    assert!(dec_content.contains("decision_type: lease_override"));
    assert!(dec_content.contains("Agent crashed and left orphan lease"));

    // Verify lease is now released
    assert!(get_lease(root, "E12S4").is_none());
}

#[test]
fn test_transition_done_auto_releases_lease() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S4", "Story 4", "review");

    let author = Author::new("human", "simon");
    claim_story(root, "E12S4", &author, None, None).unwrap();
    assert!(get_lease(root, "E12S4").is_some());

    let engine = TransitionEngine::new();
    let res = engine
        .transition(&TransitionOptions {
            workspace_root: root.to_path_buf(),
            storage: None,
            entity_kind: "story".to_string(),
            story_id: "E12S4".to_string(),
            target_status: "done".to_string(),
            justification: None,
            author: author.clone(),
            if_version: None,
        })
        .unwrap();

    assert_eq!(res.to_status, "done");
    assert!(get_lease(root, "E12S4").is_none(), "Transition to done must auto-release lease");
}

#[test]
fn test_linked_worktree_lease_discovery() {
    let temp = TempDir::new().unwrap();
    let main_repo = temp.path().join("main_repo");
    fs::create_dir_all(&main_repo).unwrap();
    setup_workspace(&main_repo);
    write_story(&main_repo, "E12S4", "Story 4", "in-progress");

    // Mock a git repo directory structure
    let git_common = main_repo.join(".git");
    fs::create_dir_all(&git_common).unwrap();

    // Create linked worktree directory
    let worktree = temp.path().join("worktree_1");
    fs::create_dir_all(&worktree).unwrap();
    setup_workspace(&worktree);
    write_story(&worktree, "E12S4", "Story 4", "in-progress");

    // Point worktree's .git file to main_repo/.git/worktrees/wt1
    let wt_gitdir = git_common.join("worktrees/wt1");
    fs::create_dir_all(&wt_gitdir).unwrap();
    fs::write(wt_gitdir.join("commondir"), "../..\n").unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}\n", wt_gitdir.display()),
    )
    .unwrap();

    // Claim story E12S4 in main_repo by simon
    let author_simon = Author::new("human", "simon");
    claim_story(&main_repo, "E12S4", &author_simon, None, None).unwrap();

    // From worktree, lease must be discoverable immediately via shared Git dir!
    let discovered = get_lease(&worktree, "E12S4").expect("Worktree must discover lease from main repo");
    assert_eq!(discovered.story_id, "E12S4");
    assert_eq!(discovered.holder, "simon");

    // Attempting to claim in worktree must fail with exit 5 already_leased
    let author_bob = Author::new("human", "bob");
    let err = claim_story(&worktree, "E12S4", &author_bob, None, None).unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::Conflict);
    assert_eq!(err.code(), "already_leased");
}

#[test]
fn test_doctor_leases_section_reports_active_and_stale() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story(root, "E12S1", "Story 1", "in-progress");
    write_story(root, "E12S2", "Story 2", "in-progress");

    // Claim E12S1 fresh
    let author = Author::new("human", "simon");
    claim_story(root, "E12S1", &author, None, None).unwrap();

    // Create a stale lease for E12S2 (5 days old)
    let stale_started = "2026-09-01T00:00:00Z";
    let lease_dir = root.join(".qdev/leases");
    fs::create_dir_all(&lease_dir).unwrap();
    let stale_lease = qdev_core::lease::StoryLease {
        story_id: "E12S2".to_string(),
        holder: "bob".to_string(),
        author_type: "human".to_string(),
        worktree_path: root.display().to_string(),
        branch: "main".to_string(),
        started_at: stale_started.to_string(),
        session_token: "qs_E12S2_9999".to_string(),
    };
    fs::write(
        lease_dir.join("E12S2.json"),
        serde_json::to_string_pretty(&stale_lease).unwrap(),
    )
    .unwrap();

    let doctor_section = qdev_core::doctor::LeasesDoctorSection::new(
        root.to_path_buf(),
        LeasesConfig { stale_age_days: 3 },
    );

    let store = SqliteStore::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
    let report = doctor_section.run(&store).unwrap();

    assert_eq!(report.name, "leases");
    let fields: std::collections::HashMap<String, serde_json::Value> =
        report.fields.into_iter().collect();

    assert_eq!(fields["status"], "ok");
    assert_eq!(fields["active_count"], 2);
    assert_eq!(fields["stale_count"], 1);
    assert_eq!(fields["stale_age_days"], 3);

    let stale_list = fields["stale_leases"].as_array().unwrap();
    assert_eq!(stale_list.len(), 1);
    assert_eq!(stale_list[0]["story_id"], "E12S2");
    assert_eq!(stale_list[0]["holder"], "bob");
}
