//! `qdev next` selection tests (Story 2.11): scope from active sprints, blocked skip with
//! dependency ids, leased-vs-own-worktree, `--owner` filter, phase ranking, and the
//! shuffled-order property test (same fixture permuted → identical output).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use qdev_core::{
    Author, Config, EntityFilter, EntityKind, EntityRecord, IdentityConfig, NextOptions,
    NextOwnerFilter, SprintAssignmentRecord, SprintRecord, SqliteStore, Store, StoryLease,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture plumbing
// ---------------------------------------------------------------------------

#[derive(Clone)]
// `deps`, `sprint`, and `lease` describe the fixture (documentation of intent); the
// actual rows are written by the separate `Job::Rel`/`Job::Assign`/`Job::Lease` entries.
#[allow(dead_code)]
struct StoryFix {
    id: &'static str,
    epic: &'static str,
    seq: u32,
    status: &'static str,
    owners: Vec<&'static str>,
    /// `depends_on` edges (source = this story).
    deps: Vec<&'static str>,
    /// Sprint the story is assigned to, if any.
    sprint: Option<i64>,
    /// `(holder, worktree)`; worktree `"HERE"` means this worktree (the temp root).
    lease: Option<(&'static str, &'static str)>,
}

#[derive(Clone)]
struct EpicFix {
    id: &'static str,
    /// Raw YAML scalar for the `phase:` key; `None` = epic carries no phase.
    phase: Option<&'static str>,
}

#[derive(Clone)]
struct SprintFix {
    id: i64,
    status: &'static str,
}

#[derive(Clone)]
enum Job {
    Story(StoryFix),
    Epic(EpicFix),
    Sprint(SprintFix),
    Assign(i64, &'static str),
    Rel(&'static str, &'static str),
    Lease(&'static str, &'static str, &'static str),
}

fn test_author() -> Author {
    Author::new("human", "simon")
}

fn test_config() -> Config {
    let mut teams = BTreeMap::new();
    teams.insert(
        "core-platform".to_string(),
        vec!["simon".to_string(), "amelia".to_string()],
    );
    teams.insert("frontend".to_string(), vec!["sally".to_string()]);
    Config {
        identity: IdentityConfig {
            developer_id: "simon".to_string(),
            teams: vec!["core-platform".to_string()],
        },
        teams: qdev_core::TeamsConfig::new(teams),
        ..Config::default()
    }
}

fn open_store(root: &Path) -> SqliteStore {
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/specs/epics")).unwrap();
    fs::create_dir_all(root.join("docs/state/sprints")).unwrap();
    let cache_db = root.join(".qdev/cache/cache.sqlite");
    {
        let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
        qdev_core::create_schema(&conn).unwrap();
    }
    SqliteStore::open(&cache_db).unwrap()
}

fn story_record(f: &StoryFix) -> EntityRecord {
    EntityRecord {
        id: f.id.to_string(),
        kind: EntityKind::Story,
        title: Some(format!("{} story", f.id)),
        status: Some(f.status.to_string()),
        owners: Some(serde_json::to_string(&f.owners).unwrap()),
        source_path: format!("docs/specs/stories/{}.md", f.id),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(test_author()),
        updated_by: Some(test_author()),
        updated_at: "2026-09-15T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some(f.epic.to_string()),
        seq: Some(f.seq),
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: Some(r#"["bridge"]"#.to_string()),
    }
}

fn run_job(store: &SqliteStore, root: &Path, job: &Job) {
    match job {
        Job::Story(f) => store.upsert_entity(&story_record(f)).unwrap(),
        Job::Epic(f) => {
            let phase_line = f.phase.map(|p| format!("phase: {p}\n")).unwrap_or_default();
            let content = format!(
                "---\nid: {id}\ntitle: \"{id} epic\"\nstatus: active\n{phase_line}---\n\n# Goal\n",
                id = f.id,
            );
            fs::write(root.join(format!("docs/specs/epics/{}.md", f.id)), content).unwrap();
            let record = EntityRecord {
                id: f.id.to_string(),
                kind: EntityKind::Epic,
                title: Some(format!("{} epic", f.id)),
                status: Some("active".to_string()),
                owners: Some(r#"["team:core-platform"]"#.to_string()),
                source_path: format!("docs/specs/epics/{}.md", f.id),
                content_hash: "hash".to_string(),
                version: 1,
                created_by: Some(test_author()),
                updated_by: Some(test_author()),
                updated_at: "2026-09-15T00:00:00Z".to_string(),
                stale: false,
                epic_id: None,
                seq: None,
                appetite: None,
                safety_class: None,
                target_modules: None,
            };
            store.upsert_entity(&record).unwrap();
        }
        Job::Sprint(f) => {
            store
                .upsert_sprint(&SprintRecord {
                    id: f.id,
                    title: Some(format!("sprint {}", f.id)),
                    release_version: Some("0.1.0".to_string()),
                    status: Some(f.status.to_string()),
                    owners: Some(r#"["team:core-platform"]"#.to_string()),
                    started_at: Some("2026-09-01T00:00:00Z".to_string()),
                    completed_at: None,
                })
                .unwrap();
            let record = EntityRecord {
                id: format!("sprint-{}", f.id),
                kind: EntityKind::Sprint,
                title: Some(format!("sprint {}", f.id)),
                status: Some(f.status.to_string()),
                owners: None,
                source_path: format!("docs/state/sprints/sprint-{}.md", f.id),
                content_hash: "hash".to_string(),
                version: 1,
                created_by: Some(test_author()),
                updated_by: Some(test_author()),
                updated_at: "2026-09-15T00:00:00Z".to_string(),
                stale: false,
                epic_id: None,
                seq: None,
                appetite: None,
                safety_class: None,
                target_modules: None,
            };
            store.upsert_entity(&record).unwrap();
        }
        Job::Assign(sprint, story) => {
            store
                .upsert_sprint_assignment(&SprintAssignmentRecord {
                    sprint_id: *sprint,
                    story_id: story.to_string(),
                    assigned_at: "2026-09-01".to_string(),
                    carried_from: None,
                })
                .unwrap();
        }
        Job::Rel(source, target) => {
            store
                .upsert_relation(&qdev_core::RelationRecord {
                    source_id: source.to_string(),
                    relation: "depends_on".to_string(),
                    target_id: target.to_string(),
                })
                .unwrap();
        }
        Job::Lease(story, holder, worktree) => {
            let lease_dir = root.join(".qdev/leases");
            fs::create_dir_all(&lease_dir).unwrap();
            let worktree_path = if *worktree == "HERE" {
                root.canonicalize().unwrap_or_else(|_| root.to_path_buf())
            } else {
                PathBuf::from(worktree)
            };
            let lease = StoryLease {
                story_id: story.to_string(),
                holder: holder.to_string(),
                author_type: "agent".to_string(),
                worktree_path: worktree_path.to_string_lossy().to_string(),
                branch: "feature/test".to_string(),
                started_at: "2026-09-17T00:00:00Z".to_string(),
                session_token: "qs_test_0001".to_string(),
            };
            fs::write(
                lease_dir.join(format!("{story}.json")),
                serde_json::to_string_pretty(&lease).unwrap(),
            )
            .unwrap();
        }
    }
}

fn select(
    root: &Path,
    store: &dyn Store,
    config: &Config,
    sprint: Option<i64>,
    owner: Option<NextOwnerFilter>,
) -> Result<qdev_core::NextSelection, qdev_core::QdevError> {
    let options = NextOptions {
        workspace_root: root,
        store,
        config,
        sprint,
        owner,
        author: test_author(),
    };
    qdev_core::select_next(&options)
}

/// The spec's happy-path fixture: sprint 5 active; `E12S2` blocked (dep `E12S1` not
/// done), `E12S3` leased by bot-9 in another worktree, `E12S4` ready/unblocked/unleased.
fn spec_matrix_jobs() -> Vec<Job> {
    vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S1",
            epic: "E12",
            seq: 1,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: None, // in no sprint assignment — never a candidate
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec!["E12S1"],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S3",
            epic: "E12",
            seq: 3,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: Some(("bot-9", "/wt/x")),
        }),
        Job::Story(StoryFix {
            id: "E12S4",
            epic: "E12",
            seq: 4,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Assign(5, "E12S2"),
        Job::Assign(5, "E12S3"),
        Job::Assign(5, "E12S4"),
        Job::Rel("E12S2", "E12S1"),
        Job::Lease("E12S3", "bot-9", "/wt/x"),
    ]
}

fn build_store(root: &Path, jobs: &[Job]) -> SqliteStore {
    let store = open_store(root);
    for job in jobs {
        run_job(&store, root, job);
    }
    store
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, map: &mut BTreeMap<String, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, map);
            } else if path.is_file() {
                map.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut map = BTreeMap::new();
    walk(root, root, &mut map);
    map
}

// ---------------------------------------------------------------------------
// Happy path and scope
// ---------------------------------------------------------------------------

#[test]
fn test_happy_path_spec_matrix() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = build_store(root, &spec_matrix_jobs());
    let config = test_config();

    let selection = select(root, &store, &config, None, None).unwrap();
    let next = selection.next.as_ref().expect("E12S4 selected");
    assert_eq!(next.id, "E12S4");
    assert_eq!(next.kind, "story");
    assert_eq!(next.status, "ready");
    assert!(!next.blocked);
    assert_eq!(next.sprint, 5);
    assert_eq!(next.sprint_status, "active");
    assert_eq!(next.seq, Some(4));
    assert_eq!(selection.reason.code, "selected");

    // E12S2 and E12S3 appear under blockers with dep ids / holder; E12S1 (in no
    // sprint assignment) is never a candidate and names nothing.
    assert_eq!(selection.blockers.len(), 2);
    let blocked = &selection.blockers[0];
    assert_eq!(blocked.story_id.as_deref(), Some("E12S2"));
    assert_eq!(blocked.kind, "blocked");
    assert_eq!(blocked.detail, "waiting on E12S1");
    assert_eq!(
        blocked.blocking_ids,
        Some(vec!["E12S1".to_string()]),
        "blocked entry carries the unmet dependency id"
    );
    let leased = &selection.blockers[1];
    assert_eq!(leased.story_id.as_deref(), Some("E12S3"));
    assert_eq!(leased.kind, "leased");
    assert_eq!(leased.detail, "held by bot-9 in /wt/x");
    assert_eq!(leased.holder.as_deref(), Some("bot-9"));
    assert_eq!(leased.worktree_path.as_deref(), Some("/wt/x"));
}

#[test]
fn test_nothing_eligible_all_candidates_blocked_or_leased() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut jobs = spec_matrix_jobs();
    jobs.retain(|j| {
        !matches!(j, Job::Story(s) if s.id == "E12S4") && !matches!(j, Job::Assign(_, "E12S4"))
    });
    let store = build_store(root, &jobs);
    let config = test_config();

    let selection = select(root, &store, &config, None, None).unwrap();
    assert!(
        selection.next.is_none(),
        "only blocked/leased candidates remain"
    );
    assert_eq!(selection.reason.code, "all_filtered");
    assert!(!selection.blockers.is_empty());
    let kinds: Vec<&str> = selection.blockers.iter().map(|b| b.kind.as_str()).collect();
    assert_eq!(kinds, vec!["blocked", "leased"]);
}

#[test]
fn test_dangling_dependency_blocks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // E12S2 depends on E99S9, which exists nowhere — dangling blocks the story.
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec!["E99S9"],
            sprint: Some(5),
            lease: None,
        }),
        Job::Assign(5, "E12S2"),
        Job::Rel("E12S2", "E99S9"),
    ];
    let store = build_store(root, &jobs);
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.blockers[0].kind, "blocked");
    assert_eq!(
        selection.blockers[0].blocking_ids,
        Some(vec!["E99S9".to_string()])
    );
}

#[test]
fn test_no_active_sprint_gives_null_and_reason() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 4,
            status: "completed",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(4),
            lease: None,
        }),
        Job::Assign(4, "E12S2"),
    ];
    let store = build_store(root, &jobs);
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.reason.code, "no_active_sprints");
    assert!(
        selection.reason.summary.contains("No active sprints"),
        "reason says \"no active sprints\": {}",
        selection.reason.summary
    );
    assert_eq!(selection.blockers.len(), 1);
    assert_eq!(selection.blockers[0].kind, "no_active_sprints");
    assert_eq!(selection.blockers[0].story_id, None);
}

#[test]
fn test_explicit_closed_sprint_selects_anyway_and_unknown_is_not_found() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 4,
            status: "completed",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(4),
            lease: None,
        }),
        Job::Assign(4, "E12S2"),
    ];
    let store = build_store(root, &jobs);
    let config = test_config();

    // `--sprint 4`, sprint 4 `status: closed`: selection runs over sprint 4's
    // assignments anyway (explicit scope).
    let selection = select(root, &store, &config, Some(4), None).unwrap();
    let next = selection.next.as_ref().expect("explicit scope selects");
    assert_eq!(next.id, "E12S2");
    assert_eq!(next.sprint, 4);
    assert_eq!(next.sprint_status, "completed");

    // Unknown id → error code `sprint_not_found`, exit 2.
    let err =
        select(root, &store, &config, Some(99), None).expect_err("unknown sprint must be refused");
    assert_eq!(err.code(), "sprint_not_found");
    assert_eq!(err.exit_code(), qdev_core::ExitCode::UsageError);

    // Explicit active sprint still selects; several active sprints tie-break by
    // sprint id first.
    let two_active = vec![
        Job::Sprint(SprintFix {
            id: 4,
            status: "active",
        }),
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S1",
            epic: "E12",
            seq: 1,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(4),
            lease: None,
        }),
        Job::Assign(4, "E12S1"),
        Job::Assign(5, "E12S2"),
    ];
    let root2 = TempDir::new().unwrap();
    let store2 = build_store(root2.path(), &two_active);
    let selection = select(root2.path(), &store2, &config, None, None).unwrap();
    assert_eq!(
        selection.next.as_ref().unwrap().id,
        "E12S1",
        "sprint id breaks the tie first"
    );
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

#[test]
fn test_owner_me_filters_team_owned_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["team:frontend"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Assign(5, "E12S2"),
    ];
    let store = build_store(root, &jobs);
    let config = test_config();

    // Identity `simon` in `core-platform`; only story owned by `team:frontend` —
    // filtered out; it was the only candidate → `next: null`, blocker names its owner.
    let selection = select(root, &store, &config, None, Some(NextOwnerFilter::Current)).unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.blockers.len(), 1);
    assert_eq!(selection.blockers[0].kind, "owner");
    assert!(selection.blockers[0].detail.contains("team:frontend"));

    // A literal `--owner team:frontend` matches the `team:frontend` owner.
    let selection = select(
        root,
        &store,
        &config,
        None,
        Some(NextOwnerFilter::Literal("team:frontend".to_string())),
    )
    .unwrap();
    assert_eq!(selection.next.as_ref().unwrap().id, "E12S2");

    // A literal that matches nothing → null with the owner blocker.
    let selection = select(
        root,
        &store,
        &config,
        None,
        Some(NextOwnerFilter::Literal("team:core-platform".to_string())),
    )
    .unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.blockers[0].kind, "owner");
}

#[test]
fn test_without_owner_current_user_match_sorts_ahead() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S1",
            epic: "E12",
            seq: 1,
            status: "ready",
            owners: vec!["team:frontend"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Assign(5, "E12S1"),
        Job::Assign(5, "E12S2"),
    ];
    let store = build_store(root, &jobs);
    // Any ownership is eligible, but the current-user match (E12S2) sorts ahead of
    // the other-owned story even though its sequence number is later.
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert_eq!(selection.next.as_ref().unwrap().id, "E12S2");
}

// ---------------------------------------------------------------------------
// Leases
// ---------------------------------------------------------------------------

#[test]
fn test_own_lease_continuation_and_other_holder_skips() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: Some(("simon", "HERE")),
        }),
        Job::Assign(5, "E12S2"),
        Job::Lease("E12S2", "simon", "HERE"),
    ];
    let store = build_store(root, &jobs);

    // `E12S2` leased by this worktree → returned (continuation), reason says
    // "leased by you here".
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    let next = selection.next.as_ref().expect("own-lease story returned");
    assert_eq!(next.id, "E12S2");
    let lease = next.lease.as_ref().expect("lease carried");
    assert_eq!(lease.holder, "simon");
    assert!(selection.reason.summary.contains("leased by you here"));

    // Same fixture, but the lease is held by another holder: never returned.
    let temp2 = TempDir::new().unwrap();
    let root2 = temp2.path();
    let jobs2 = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: Some(("bot-9", "HERE")),
        }),
        Job::Assign(5, "E12S2"),
        Job::Lease("E12S2", "bot-9", "HERE"),
    ];
    let store2 = build_store(root2, &jobs2);
    let selection = select(root2, &store2, &test_config(), None, None).unwrap();
    assert!(
        selection.next.is_none(),
        "a story leased by another holder is never returned"
    );
    assert_eq!(selection.blockers[0].kind, "leased");
}

// ---------------------------------------------------------------------------
// Status eligibility and epic phase
// ---------------------------------------------------------------------------

#[test]
fn test_terminal_and_review_stories_never_picked() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
        Job::Story(StoryFix {
            id: "E12S1",
            epic: "E12",
            seq: 1,
            status: "done",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S2",
            epic: "E12",
            seq: 2,
            status: "review",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S3",
            epic: "E12",
            seq: 3,
            status: "superseded",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E12S4",
            epic: "E12",
            seq: 4,
            status: "abandoned",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Assign(5, "E12S1"),
        Job::Assign(5, "E12S2"),
        Job::Assign(5, "E12S3"),
        Job::Assign(5, "E12S4"),
    ];
    let store = build_store(root, &jobs);
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.blockers.len(), 4);
    assert!(selection.blockers.iter().all(|b| b.kind == "status"));
}

#[test]
fn test_draft_ready_in_progress_and_phase_ranking() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // E11 phase 1, E12 phase 2, E13 phase "alpha", E14 no phase — all owned the
    // same, all in sprint 5: numerics ascend first, strings lexicographic after,
    // epics without phase last.
    let jobs = vec![
        Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
        Job::Epic(EpicFix {
            id: "E11",
            phase: Some("1"),
        }),
        Job::Epic(EpicFix {
            id: "E12",
            phase: Some("2"),
        }),
        Job::Epic(EpicFix {
            id: "E13",
            phase: Some("alpha"),
        }),
        Job::Epic(EpicFix {
            id: "E14",
            phase: None,
        }),
        // E12S1 has the lowest sequence but its epic's phase is later — phase first.
        Job::Story(StoryFix {
            id: "E12S1",
            epic: "E12",
            seq: 1,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E11S9",
            epic: "E11",
            seq: 9,
            status: "in-progress",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E13S1",
            epic: "E13",
            seq: 1,
            status: "draft",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Story(StoryFix {
            id: "E14S1",
            epic: "E14",
            seq: 1,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: Some(5),
            lease: None,
        }),
        Job::Assign(5, "E11S9"),
        Job::Assign(5, "E12S1"),
        Job::Assign(5, "E13S1"),
        Job::Assign(5, "E14S1"),
    ];
    let store = build_store(root, &jobs);
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert_eq!(
        selection.next.as_ref().unwrap().id,
        "E11S9",
        "lowest numeric phase wins even with the worst sequence (and in-progress is eligible)"
    );

    // Without E11, phase 2 comes next; without E11/E12, the string phase; with
    // only the phase-less epic, that story is still selected (only candidate).
    let mut without_e11: Vec<Job> = jobs
        .iter()
        .filter(|j| !matches!(j, Job::Story(s) if s.epic == "E11" || s.epic == "E13" || s.epic == "E14"))
        .cloned()
        .collect();
    without_e11.retain(|j| {
        !matches!(
            j,
            Job::Assign(5, "E11S9") | Job::Assign(5, "E13S1") | Job::Assign(5, "E14S1")
        )
    });
    let temp2 = TempDir::new().unwrap();
    let store2 = build_store(temp2.path(), &without_e11);
    let selection = select(temp2.path(), &store2, &test_config(), None, None).unwrap();
    assert_eq!(selection.next.as_ref().unwrap().id, "E12S1");

    let mut strings_and_missing: Vec<Job> = jobs
        .iter()
        .filter(|j| !matches!(j, Job::Story(s) if s.epic == "E11" || s.epic == "E12"))
        .cloned()
        .collect();
    strings_and_missing.retain(|j| !matches!(j, Job::Assign(5, "E11S9") | Job::Assign(5, "E12S1")));
    let temp3 = TempDir::new().unwrap();
    let store3 = build_store(temp3.path(), &strings_and_missing);
    // "alpha" (E13) sorts ahead of the phase-less E14.
    let selection = select(temp3.path(), &store3, &test_config(), None, None).unwrap();
    assert_eq!(selection.next.as_ref().unwrap().id, "E13S1");

    let only_missing: Vec<Job> = jobs
        .iter()
        .filter(|j| !matches!(j, Job::Story(s) if s.epic != "E14"))
        .cloned()
        .collect();
    let temp4 = TempDir::new().unwrap();
    let store4 = build_store(temp4.path(), &only_missing);
    let selection = select(temp4.path(), &store4, &test_config(), None, None).unwrap();
    assert_eq!(selection.next.as_ref().unwrap().id, "E14S1");

    // Same epic, no phase data: order collapses to epic id then story seq — with a
    // single epic left, that is identical to dropping the key (seq decides).
}

// ---------------------------------------------------------------------------
// Determinism and purity
// ---------------------------------------------------------------------------

#[test]
fn test_shuffled_fixture_order_gives_identical_output() {
    let base = spec_matrix_jobs();
    let n = base.len();
    // Several bijective permutations of the same fixture.
    let orders: Vec<Vec<usize>> = vec![
        (0..n).collect(),
        (0..n).rev().collect(),
        (0..n).map(|i| (i + 3) % n).collect(),
        (0..n).map(|i| (i + 7) % n).collect(),
        {
            let mut v: Vec<usize> = (n / 2..n).chain(0..n / 2).collect();
            v.sort_unstable_by_key(|i| (i * 5) % n);
            v
        },
    ];

    let mut outputs: Vec<String> = Vec::new();
    for order in orders {
        assert_eq!(
            order.iter().collect::<std::collections::HashSet<_>>().len(),
            n,
            "test permutation must be a bijection"
        );
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let mut jobs: Vec<Job> = order.iter().map(|&i| base[i].clone()).collect();
        // Keep dependency-creating jobs (sprint records) before the assignments that
        // reference them — the cache could never legitimately hold the reverse —
        // while permuting everything else freely.
        jobs.sort_by_key(|job| match job {
            Job::Sprint(_) => 0,
            Job::Epic(_) | Job::Story(_) => 1,
            _ => 2,
        });
        let store = build_store(root, &jobs);
        let selection = select(root, &store, &test_config(), None, None).unwrap();
        outputs.push(serde_json::to_string(&selection).unwrap());
    }
    let first = &outputs[0];
    for other in &outputs {
        assert_eq!(first, other, "shuffled insertion order changed the output");
    }
    // And the shared answer is the expected one.
    let selection: qdev_core::NextSelection = serde_json::from_str(first).unwrap();
    assert_eq!(selection.next.as_ref().unwrap().id, "E12S4");
}

#[test]
fn test_selection_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = build_store(root, &spec_matrix_jobs());
    let config = test_config();

    // Snapshot every file except the cache (the open store connection legitimately
    // touches `cache.sqlite*` — its rows are checked separately below).
    let snapshot = |root: &Path| -> BTreeMap<String, Vec<u8>> {
        snapshot_tree(root)
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".qdev/cache/"))
            .collect::<BTreeMap<_, _>>()
    };
    let cache_rows = |store: &SqliteStore| {
        (
            // Raw reads are deliberate here: the point is to compare the FULL cached
            // rows before and after, including any retained stale row.
            #[allow(clippy::disallowed_methods)]
            {
                store.list_entities(&EntityFilter::default()).unwrap()
            },
            store.list_relations().unwrap(),
            store.list_sprints().unwrap(),
            store.list_sprint_assignments().unwrap(),
            store.list_findings().unwrap(),
            store.get_dirty_entities().unwrap(),
            store.get_last_synced_at().unwrap(),
        )
    };
    let before = snapshot(root);
    let before_cache = cache_rows(&store);

    let first = select(root, &store, &config, None, None).unwrap();
    let second = select(root, &store, &config, None, None).unwrap();
    assert_eq!(first, second, "same store must give the same selection");

    let after = snapshot(root);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "file list changed"
    );
    for (path, content) in &before {
        assert_eq!(
            content, &after[path],
            "file {path} changed during select_next"
        );
    }
    let after_cache = cache_rows(&store);
    assert_eq!(before_cache.0, after_cache.0, "entity rows changed");
    assert_eq!(before_cache.1, after_cache.1, "relation rows changed");
    assert_eq!(before_cache.2, after_cache.2, "sprint rows changed");
    assert_eq!(before_cache.3, after_cache.3, "assignment rows changed");
    assert_eq!(before_cache.4, after_cache.4, "finding rows changed");
    assert_eq!(before_cache.5, after_cache.5, "dirty rows changed");
    assert_eq!(before_cache.6, after_cache.6, "sync state changed");
}

// ---------------------------------------------------------------------------
// Stale / missing rows and assignment-less scope (review coverage gaps)
// ---------------------------------------------------------------------------

#[test]
fn test_stale_row_and_missing_assignment_are_blockers_not_candidates() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);
    run_job(
        &store,
        root,
        &Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
    );
    run_job(
        &store,
        root,
        &Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
    );

    // E12S2's cache row is retained `stale`: its file failed the last parse.
    let mut stale = story_record(&StoryFix {
        id: "E12S2",
        epic: "E12",
        seq: 2,
        status: "ready",
        owners: vec!["simon"],
        deps: vec![],
        sprint: Some(5),
        lease: None,
    });
    stale.stale = true;
    store.upsert_entity(&stale).unwrap();

    run_job(&store, root, &Job::Assign(5, "E12S2"));
    // GHOST is assigned but has no cache row at all.
    run_job(&store, root, &Job::Assign(5, "GHOST"));

    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert!(
        selection.next.is_none(),
        "a stale row must never be offered from cached data"
    );
    assert_eq!(selection.reason.code, "all_filtered");
    let stale_blocker = selection
        .blockers
        .iter()
        .find(|b| b.story_id.as_deref() == Some("E12S2"))
        .expect("stale blocker for E12S2");
    assert_eq!(stale_blocker.kind, "stale");
    assert!(stale_blocker.detail.contains("failed the last parse"));
    let missing = selection
        .blockers
        .iter()
        .find(|b| b.story_id.as_deref() == Some("GHOST"))
        .expect("missing blocker for GHOST");
    assert_eq!(missing.kind, "missing");
    assert!(missing.detail.contains("no cache row"));
}

#[test]
fn test_sprint_without_assignments_gives_no_candidates() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);
    run_job(
        &store,
        root,
        &Job::Sprint(SprintFix {
            id: 5,
            status: "active",
        }),
    );
    run_job(
        &store,
        root,
        &Job::Epic(EpicFix {
            id: "E12",
            phase: None,
        }),
    );
    // E12S1 exists and is ready — but is assigned to NO sprint, so it is never a
    // candidate: a story in no sprint assignment must not be offered.
    run_job(
        &store,
        root,
        &Job::Story(StoryFix {
            id: "E12S1",
            epic: "E12",
            seq: 1,
            status: "ready",
            owners: vec!["simon"],
            deps: vec![],
            sprint: None,
            lease: None,
        }),
    );

    // Default scope: the active sprint assigns nothing — nothing to rank at all.
    let selection = select(root, &store, &test_config(), None, None).unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.reason.code, "no_candidates");
    assert!(
        selection.reason.summary.contains("[5]"),
        "summary names the empty active sprint: {}",
        selection.reason.summary
    );
    assert_eq!(selection.blockers.len(), 1);
    assert_eq!(selection.blockers[0].kind, "no_candidates");
    assert!(selection.blockers[0]
        .detail
        .contains("assign stories with `qdev sprint assign` first"));

    // Explicit --sprint on the assignment-less sprint: same branch, its own wording.
    let selection = select(root, &store, &test_config(), Some(5), None).unwrap();
    assert!(selection.next.is_none());
    assert_eq!(selection.reason.code, "no_candidates");
    assert!(selection
        .reason
        .summary
        .contains("Sprint 5 has no assigned stories"));
}
