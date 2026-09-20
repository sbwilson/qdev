//! Workspace pulse tests (Story 2.12): the four-counter exclusivity and sum, stale-row
//! exclusion, workspace-wide DW open/unacceptable counts, gates null-vs-present, the
//! no-active-sprints case, non-git degradation, the own-worktree lease filter, and a
//! shuffled-fixture determinism test — plus the "no readable cache" cases, where an
//! initialized workspace with a missing or unreadable cache is *reported* (degraded cache,
//! empty sprints, no recommendation) instead of being rebuilt or swept. Everything runs
//! against seeded caches — the pulse must stay pure over store/config/git/lease reads and
//! write nothing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qdev_core::{
    Author, CacheHealth, CacheSchemaStatus, Config, DeferredWorkRecord, EntityKind, EntityRecord,
    EnvironmentPulse, GateRunRecord, PulseOptions, PulsePayload, SprintAssignmentRecord,
    SprintRecord, SqliteStore, Store, StoryLease,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture plumbing
// ---------------------------------------------------------------------------

fn test_author() -> Author {
    Author::new("human", "simon")
}

fn test_config() -> Config {
    Config::default()
}

/// A fixed clock — the same `now` on both sides of every determinism comparison.
fn fixed_now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_760_000_000)
}

fn open_store(root: &Path) -> SqliteStore {
    let cache_db = open_store_dir(root);
    {
        let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
        qdev_core::create_schema(&conn).unwrap();
        // Stamping is the separate, explicit step: `create_schema` deliberately
        // leaves `PRAGMA user_version` at 0, and an unstamped cache reads as
        // `mismatch` — this fixture wants a healthy cache.
        qdev_core::stamp_cache_version(&conn).unwrap();
    }
    SqliteStore::open(&cache_db).unwrap()
}

/// Creates the fixture directories and returns the cache path, without touching the
/// cache itself.
fn open_store_dir(root: &Path) -> PathBuf {
    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/state/sprints")).unwrap();
    fs::create_dir_all(root.join("docs/state/dw")).unwrap();
    root.join(".qdev/cache/cache.sqlite")
}

/// Builds a cache that this binary cannot read: the tables exist, but
/// `PRAGMA user_version` is left at 0 (or set to `stamp` when given), so
/// `inspect_cache_schema` reports `Mismatch`. Used to pin that the pulse reports such a
/// cache instead of rebuilding or sweeping it.
fn unreadable_store(root: &Path, stamp: Option<u32>) -> SqliteStore {
    let cache_db = open_store_dir(root);
    {
        let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
        qdev_core::create_schema(&conn).unwrap();
        match stamp {
            // Never stamped — a cache created by `create_schema` and left alone.
            None => {}
            Some(stamp) => conn
                .execute_batch(&format!("PRAGMA user_version = {stamp};"))
                .unwrap(),
        }
    }
    SqliteStore::open(&cache_db).unwrap()
}

/// Adds a table that is not in `ALL_TABLE_NAMES`, another route to `Mismatch`.
fn add_legacy_table(root: &Path) {
    let conn =
        qdev_core::rusqlite::Connection::open(root.join(".qdev/cache/cache.sqlite")).unwrap();
    conn.execute_batch("CREATE TABLE IF NOT EXISTS old_legacy_table (val TEXT);")
        .unwrap();
    drop(conn);
}

/// Runs git in `root`, failing loudly: these fixtures depend on real git behaviour.
fn git(root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
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

/// Builds the pulse with **no** store, the way the CLI does for an initialized workspace
/// whose cache is missing or unreadable.
fn build_without_store(root: &Path) -> PulsePayload {
    let config = test_config();
    let options = PulseOptions {
        workspace: true,
        workspace_root: root,
        store: None,
        config: &config,
        author: test_author(),
        now: fixed_now(),
    };
    qdev_core::build_pulse(&options)
        .expect("a workspace with no readable cache is reported, not an error")
}

/// The report a workspace with no usable cache must produce.
fn degraded_report() -> PulsePayload {
    PulsePayload {
        workspace: true,
        environment: Some(EnvironmentPulse {
            working_tree: None,
            integration: None,
            cache: CacheHealth {
                schema_status: "mismatch".to_string(),
                entity_count: None,
                finding_count: None,
                synced_ms_ago: None,
            },
            lease: None,
        }),
        sprints: Some(vec![]),
        gates: None,
        next: None,
    }
}

fn story_record(id: &str, status: &str) -> EntityRecord {
    EntityRecord {
        id: id.to_string(),
        kind: EntityKind::Story,
        title: Some(format!("{id} story")),
        status: Some(status.to_string()),
        owners: Some(r#"["simon"]"#.to_string()),
        source_path: format!("docs/specs/stories/{id}.md"),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(test_author()),
        updated_by: Some(test_author()),
        updated_at: "2026-09-15T00:00:00Z".to_string(),
        stale: false,
        epic_id: Some("E12".to_string()),
        seq: id.strip_prefix("E12S").and_then(|s| s.parse::<u32>().ok()),
        appetite: Some("small".to_string()),
        safety_class: Some("ClassB".to_string()),
        target_modules: Some(r#"["bridge"]"#.to_string()),
    }
}

fn seed_story(store: &SqliteStore, id: &str, status: &str) {
    store.upsert_entity(&story_record(id, status)).unwrap();
}

fn seed_stale_story(store: &SqliteStore, id: &str, status: &str) {
    let mut record = story_record(id, status);
    record.stale = true;
    store.upsert_entity(&record).unwrap();
}

fn seed_sprint(store: &SqliteStore, num: i64, status: &str, assignments: &[&str]) {
    store
        .upsert_sprint(&SprintRecord {
            id: num,
            title: Some(format!("Sprint {num}")),
            release_version: Some("0.1.0".to_string()),
            status: Some(status.to_string()),
            owners: Some(r#"["team:core-platform"]"#.to_string()),
            started_at: Some("2026-09-01T00:00:00Z".to_string()),
            completed_at: None,
        })
        .unwrap();
    let entity_id = qdev_core::sprint_entity_id(num);
    let record = EntityRecord {
        id: entity_id.clone(),
        kind: EntityKind::Sprint,
        title: Some(format!("Sprint {num}")),
        status: Some(status.to_string()),
        owners: None,
        source_path: format!("docs/state/sprints/sprint-{num}.md"),
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
    for story in assignments {
        store
            .upsert_sprint_assignment(&SprintAssignmentRecord {
                sprint_id: num,
                story_id: story.to_string(),
                assigned_at: "2026-09-01".to_string(),
                carried_from: None,
            })
            .unwrap();
    }
}

fn seed_dep(store: &SqliteStore, from: &str, to: &str) {
    store
        .upsert_relation(&qdev_core::RelationRecord {
            source_id: from.to_string(),
            relation: "depends_on".to_string(),
            target_id: to.to_string(),
        })
        .unwrap();
}

fn seed_dw(store: &SqliteStore, id: &str, status: &str, risk: &str) {
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: id.to_string(),
            origin_story_id: Some("E12S4".to_string()),
            target_module: "bridge".to_string(),
            status: Some(status.to_string()),
            safety_risk: Some(risk.to_string()),
            rationale: Some("recorded".to_string()),
            gate: None,
            resolution: None,
        })
        .unwrap();
}

fn seed_gate_run(store: &SqliteStore, id: &str, gate: &str, status: &str, ran_at: &str) {
    store
        .upsert_gate_run(&GateRunRecord {
            id: id.to_string(),
            story_id: Some("E12S4".to_string()),
            gate_id: gate.to_string(),
            commit_sha: "8f1b2c4".to_string(),
            status: Some(status.to_string()),
            exit_code: Some(if status == "pass" { 0 } else { 1 }),
            duration_ms: Some(500),
            metric_value: None,
            summary: Some(format!("{gate} {status}")),
            evidence_path: format!("docs/state/evidence/{id}.json"),
            output_hash: None,
            run_by_type: Some("human".to_string()),
            run_by_id: Some("simon".to_string()),
            ran_at: Some(ran_at.to_string()),
        })
        .unwrap();
}

fn seed_lease(root: &Path, story: &str, holder: &str, worktree: &str) {
    let dir = root.join(".qdev/leases");
    fs::create_dir_all(&dir).unwrap();
    let worktree_path = if worktree == "HERE" {
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
        started_at: "2026-09-17T09:41:00Z".to_string(),
        session_token: "qs_test_0001".to_string(),
    };
    fs::write(
        dir.join(format!("{story}.json")),
        serde_json::to_string_pretty(&lease).unwrap(),
    )
    .unwrap();
}

fn build(root: &Path, store: &SqliteStore) -> PulsePayload {
    build_at(root, store, fixed_now())
}

fn build_at(root: &Path, store: &dyn Store, now: SystemTime) -> PulsePayload {
    let config = test_config();
    let options = PulseOptions {
        workspace: true,
        workspace_root: root,
        store: Some(store),
        config: &config,
        author: test_author(),
        now,
    };
    qdev_core::build_pulse(&options).unwrap()
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
// Story counters: the four fixed buckets
// ---------------------------------------------------------------------------

/// D-2: `done` first; then `in-progress` iff status `in-progress` and not
/// computed-blocked; then `blocked` iff not done and computed-blocked; everything else
/// assigned is `backlog`. The buckets sum to the assigned-story count.
#[test]
fn test_four_counter_buckets_are_exclusive_and_sum_to_assigned() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "done");
    seed_story(&store, "E12S2", "in-progress"); // dep E12S1 is done — not blocked
    seed_story(&store, "E12S3", "in-progress"); // dep E12S4 is not done — computed-blocked
    seed_story(&store, "E12S4", "ready"); // unblocked, not in-progress — backlog
    seed_story(&store, "E12S5", "ready"); // backlog
    seed_story(&store, "E12S6", "review"); // neither done, in-progress, nor blocked — backlog
    seed_dep(&store, "E12S2", "E12S1");
    seed_dep(&store, "E12S3", "E12S4");
    seed_sprint(
        &store,
        5,
        "active",
        &["E12S1", "E12S2", "E12S3", "E12S4", "E12S5", "E12S6"],
    );

    let pulse = build(root, &store);
    let sprints = pulse
        .sprints
        .expect("sprints section present in a workspace");
    assert_eq!(sprints.len(), 1);
    let block = &sprints[0];
    assert_eq!(block.id, 5);
    assert_eq!(block.status, "active");
    assert_eq!(block.title.as_deref(), Some("Sprint 5"));
    assert_eq!(block.release.as_deref(), Some("0.1.0"));
    assert_eq!(block.stories.done, 1);
    assert_eq!(block.stories.in_progress, 1);
    assert_eq!(block.stories.blocked, 1);
    assert_eq!(block.stories.backlog, 3);
    let assigned = 6;
    let sum = block.stories.done
        + block.stories.in_progress
        + block.stories.blocked
        + block.stories.backlog;
    assert_eq!(
        sum, assigned,
        "buckets must sum to the assigned-story count"
    );
}

#[test]
fn test_stale_assigned_story_is_excluded_everywhere() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "done");
    seed_story(&store, "E12S2", "in-progress");
    seed_dep(&store, "E12S2", "E12S1");
    // Retained stale row — live-only reads exclude it from every bucket.
    seed_stale_story(&store, "E12S3", "ready");
    seed_story(&store, "E12S5", "review");
    seed_sprint(&store, 5, "active", &["E12S1", "E12S2", "E12S3", "E12S5"]);

    let pulse = build(root, &store);
    let block = &pulse.sprints.unwrap()[0];
    // 3 of the 4 assigned stories are counted; the stale row contributes to none.
    let sum = block.stories.done
        + block.stories.in_progress
        + block.stories.blocked
        + block.stories.backlog;
    assert_eq!(
        sum, 3,
        "stale row must never be counted: {:?}",
        block.stories
    );
    assert_eq!(block.stories.done, 1);
    assert_eq!(block.stories.in_progress, 1);
    assert_eq!(block.stories.blocked, 0);
    assert_eq!(block.stories.backlog, 1);
}

#[test]
fn test_a_story_assigned_but_missing_from_the_cache_is_excluded() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "done");
    // Assignment to an id with no cache row at all.
    seed_sprint(&store, 5, "active", &["E12S1", "E12S9"]);

    let pulse = build(root, &store);
    let block = &pulse.sprints.unwrap()[0];
    assert_eq!(block.stories.done, 1);
    assert_eq!(
        block.stories.done
            + block.stories.in_progress
            + block.stories.blocked
            + block.stories.backlog,
        1,
        "a story with no live row is in no bucket"
    );
}

// ---------------------------------------------------------------------------
// Deferred work: workspace-wide debt in every active sprint block
// ---------------------------------------------------------------------------

#[test]
fn test_dw_counts_are_workspace_wide_and_render_in_every_block() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "ready");
    seed_story(&store, "E12S2", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);
    seed_sprint(&store, 6, "active", &["E12S2"]);
    seed_dw(&store, "DW-1111", "open", "negligible");
    seed_dw(&store, "DW-2222", "open", "unacceptable");
    seed_dw(&store, "DW-3333", "done", "unacceptable");

    let pulse = build(root, &store);
    let sprints = pulse.sprints.unwrap();
    assert_eq!(sprints.len(), 2);
    // Sorted by sprint id — not by insertion order.
    assert_eq!(sprints[0].id, 5);
    assert_eq!(sprints[1].id, 6);
    for block in &sprints {
        // Two open rows (the done one does not count); one carries `unacceptable`.
        assert_eq!(
            (block.deferred_work.open, block.deferred_work.unacceptable),
            (2, 1),
            "workspace-wide debt must render in every active sprint block"
        );
    }
}

// ---------------------------------------------------------------------------
// Gates: only from existing evidence
// ---------------------------------------------------------------------------

#[test]
fn test_gates_null_without_gate_run_rows() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);

    let pulse = build(root, &store);
    assert!(pulse.gates.is_none(), "no gate_runs rows → gates: null");
}

#[test]
fn test_gates_use_the_most_recent_run_per_gate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);
    // `fmt`: an early pass and a later fail — the most recent run decides.
    seed_gate_run(&store, "gr-early", "fmt", "pass", "2026-09-17T00:00:00Z");
    seed_gate_run(&store, "gr-late", "fmt", "fail", "2026-09-18T00:00:00Z");
    seed_gate_run(&store, "gr-lint", "lint", "pass", "2026-09-18T12:00:00Z");

    let pulse = build(root, &store);
    let gates = pulse.gates.expect("gate_runs rows exist");
    assert_eq!(gates.total, 2, "two distinct gates carry evidence");
    assert_eq!(gates.passing, 1, "fmt's latest run failed; lint's passed");
}

// ---------------------------------------------------------------------------
// No active sprints
// ---------------------------------------------------------------------------

#[test]
fn test_no_active_sprints_gives_empty_sprints_and_no_active_sprints_next() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S2", "ready");
    seed_sprint(&store, 4, "completed", &["E12S2"]);

    let pulse = build(root, &store);
    let sprints = pulse.sprints.expect("empty array inside a workspace");
    assert!(sprints.is_empty(), "only active sprints get a block");
    let next = pulse
        .next
        .expect("next renders even with no active sprints");
    assert!(next.next.is_none());
    assert_eq!(next.reason.code, "no_active_sprints");
    assert!(next
        .reason
        .summary
        .to_lowercase()
        .contains("no active sprints"));
    assert_eq!(next.blockers[0].kind, "no_active_sprints");
}

#[test]
fn test_sprint_with_stale_entity_row_is_not_a_block() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);
    // The sprints row says active, but the sprint's own entity row is stale — the
    // live-only rule excludes the block.
    let record = {
        let mut r = story_record("sprint-5", "active");
        r.kind = EntityKind::Sprint;
        r.source_path = "docs/state/sprints/sprint-5.md".to_string();
        r.stale = true;
        r
    };
    store.upsert_entity(&record).unwrap();

    let pulse = build(root, &store);
    assert!(pulse.sprints.unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// Non-git degradation
// ---------------------------------------------------------------------------

#[test]
fn test_non_git_workspace_degrades_git_fields_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // Deliberately no `git init` — every git probe fails.
    let store = open_store(root);

    seed_story(&store, "E12S1", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);

    let pulse = build(root, &store);
    let env = pulse
        .environment
        .expect("environment renders in a workspace");
    assert!(
        env.working_tree.is_none(),
        "non-git directory: working_tree is the null marker"
    );
    assert!(
        env.integration.is_none(),
        "non-git directory: integration is the null marker"
    );
    // Cache, sprints, and next render normally from cache-native reads.
    assert_eq!(env.cache.schema_status, "ok");
    assert!(env.cache.entity_count.is_some());
    assert!(env.cache.synced_ms_ago.is_none(), "never synced here");
    assert_eq!(pulse.sprints.unwrap().len(), 1);
    let next = pulse.next.expect("next renders normally");
    assert_eq!(next.reason.code, "selected");
    assert_eq!(next.next.as_ref().unwrap().id, "E12S1");
}

// ---------------------------------------------------------------------------
// Own-worktree lease filter
// ---------------------------------------------------------------------------

#[test]
fn test_environment_lease_lists_this_worktree_records_only() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S2", "ready");
    seed_story(&store, "E12S4", "ready");
    seed_story(&store, "E12S7", "ready");
    seed_sprint(&store, 5, "active", &["E12S2", "E12S4", "E12S7"]);
    seed_lease(root, "E12S4", "simon", "HERE");
    seed_lease(root, "E12S7", "bot-9", "/wt/x");

    let pulse = build(root, &store);
    let env = pulse.environment.expect("environment renders");
    let leases = env.lease.expect("one lease held by this worktree");
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].story_id, "E12S4");
    assert_eq!(leases[0].holder, "simon");
    assert_eq!(leases[0].branch, "feature/test");
}

#[test]
fn test_multiple_this_worktree_leases_sort_by_story_id() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S2", "ready");
    seed_story(&store, "E12S4", "ready");
    seed_sprint(&store, 5, "active", &["E12S2", "E12S4"]);
    // Insert the later story first — ordering must not follow insertion.
    seed_lease(root, "E12S4", "simon", "HERE");
    seed_lease(root, "E12S2", "simon", "HERE");

    let pulse = build(root, &store);
    let leases = pulse.environment.unwrap().lease.expect("two leases");
    let ids: Vec<&str> = leases.iter().map(|l| l.story_id.as_str()).collect();
    assert_eq!(ids, vec!["E12S2", "E12S4"]);
}

#[test]
fn test_no_leases_gives_null_lease_field() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);
    seed_lease(root, "E12S1", "bot-9", "/wt/x");

    let pulse = build(root, &store);
    assert!(
        pulse.environment.unwrap().lease.is_none(),
        "a lease held in another worktree must not surface here"
    );
}

// ---------------------------------------------------------------------------
// Determinism and purity
// ---------------------------------------------------------------------------

/// The same fixture, seeded in shuffled order into two separate caches, must produce
/// identical payloads — output cannot depend on cache-row or fixture order.
#[test]
fn test_shuffled_fixture_order_gives_identical_output() {
    let temp_a = TempDir::new().unwrap();
    let temp_b = TempDir::new().unwrap();

    // Forward-order seed jobs; the second store runs them rotated.
    let jobs: Vec<(&str, Job)> = vec![
        ("sprint5", Job::Sprint(5, "active")),
        ("assign-1", Job::Assign(5, "E12S1")),
        ("assign-2", Job::Assign(5, "E12S2")),
        ("assign-3", Job::Assign(5, "E12S3")),
        ("assign-5", Job::Assign(5, "E12S5")),
        ("story-1", Job::Story("E12S1", "done")),
        ("story-2", Job::Story("E12S2", "in-progress")),
        ("story-3", Job::Story("E12S3", "ready")),
        ("story-5", Job::Story("E12S5", "ready")),
        ("dep-2-1", Job::Rel("E12S2", "E12S1")),
        ("dep-3-2", Job::Rel("E12S3", "E12S2")),
        ("dw-1", Job::Dw("DW-1111", "open", "negligible")),
        ("dw-2", Job::Dw("DW-2222", "open", "unacceptable")),
        (
            "run-fmt-early",
            Job::GateRun("gr-a", "fmt", "pass", "2026-09-17T00:00:00Z"),
        ),
        (
            "run-fmt-late",
            Job::GateRun("gr-b", "fmt", "fail", "2026-09-18T00:00:00Z"),
        ),
        (
            "run-lint",
            Job::GateRun("gr-c", "lint", "pass", "2026-09-18T12:00:00Z"),
        ),
    ];
    // The same jobs, reversed — with one exception the foreign keys enforce:
    // `sprint_assignments.sprint_id` references `sprints(id)`, so the sprint row
    // must be seeded before any of its assignments in either order. Everything
    // else swaps order between the two runs.
    let mut rotated: Vec<(&str, Job)> = jobs.iter().rev().cloned().collect();
    let sprint_at = rotated
        .iter()
        .position(|(_, job)| matches!(job, Job::Sprint(..)))
        .expect("sprint seed in the fixture");
    let sprint_job = rotated.remove(sprint_at);
    rotated.insert(0, sprint_job);

    let run = |root: &Path, jobs: &[(&str, Job)], label: &str| {
        let store = open_store(root);
        for (_, job) in jobs {
            match job {
                Job::Sprint(n, s) => {
                    // Only the sprint row (plus its entity row); the assignments
                    // are seeded by their own `assign-*` jobs.
                    if store.get_sprint(*n).unwrap().is_some() {
                        panic!("duplicate sprint seed in fixture {label}");
                    }
                    seed_sprint(&store, *n, s, &[]);
                }
                Job::Assign(n, story) => store
                    .upsert_sprint_assignment(&SprintAssignmentRecord {
                        sprint_id: *n,
                        story_id: story.to_string(),
                        assigned_at: "2026-09-01".to_string(),
                        carried_from: None,
                    })
                    .unwrap(),
                Job::Story(id, status) => seed_story(&store, id, status),
                Job::Rel(from, to) => seed_dep(&store, from, to),
                Job::Dw(id, status, risk) => seed_dw(&store, id, status, risk),
                Job::GateRun(id, gate, status, at) => seed_gate_run(&store, id, gate, status, at),
            }
        }
        build_at(root, &store, fixed_now())
    };

    let pulse_a = run(temp_a.path(), &jobs, "a");
    let pulse_b = run(temp_b.path(), &rotated, "b");
    let json_a = serde_json::to_string(&pulse_a).unwrap();
    let json_b = serde_json::to_string(&pulse_b).unwrap();
    assert_eq!(
        json_a, json_b,
        "shuffled fixture order must not change output"
    );
}

#[test]
fn test_build_pulse_writes_nothing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    seed_story(&store, "E12S1", "done");
    seed_story(&store, "E12S2", "in-progress");
    seed_dep(&store, "E12S2", "E12S1");
    seed_sprint(&store, 5, "active", &["E12S1", "E12S2"]);
    seed_dw(&store, "DW-1111", "open", "unacceptable");

    // Warm any probe bootstrap first, then compare: a run must not create or change
    // any file — including the cache database itself.
    build(root, &store);
    let before = snapshot_tree(root);
    build(root, &store);
    let after = snapshot_tree(root);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "file list changed — nothing may be created"
    );
    for ((path, content), (after_path, after_content)) in before.iter().zip(&after) {
        assert_eq!(path, after_path);
        assert_eq!(
            content, after_content,
            "file {path} changed — the pulse must not mutate anything"
        );
    }
}

// ---------------------------------------------------------------------------
// Performance AC: no network git args, checked from the source of truth
// ---------------------------------------------------------------------------

#[test]
fn test_no_network_git_commands_ever_constructed() {
    // The checked form of the "100 ms + Git status time" AC: the probes are local and
    // bounded — the remote is never contacted. These argument vectors must never
    // appear in the module, and nothing else constructs a `git` command.
    let src = include_str!("../src/pulse.rs");
    for forbidden in ["\"fetch\"", "\"pull\"", "\"ls-remote\""] {
        assert!(
            !src.contains(forbidden),
            "pulse.rs must never construct the network git arg {forbidden}"
        );
    }
    // And every git arg list the module *does* build is from the pinned local set.
    for allowed in [
        "\"status\"",
        "--porcelain",
        "\"rev-parse\"",
        "--abbrev-ref",
        "--short",
        "--verify",
        "--quiet",
        "\"rev-list\"",
        "--left-right",
        "--count",
    ] {
        assert!(
            src.contains(allowed),
            "expected the local probe fragment {allowed} in pulse.rs"
        );
    }
}

// ---------------------------------------------------------------------------
// No readable cache: reported, never repaired
// ---------------------------------------------------------------------------

/// An initialized workspace whose cache was never built: `qdev` reports the cache as
/// degraded with every count `null`, reports no sprints and no recommendation, and
/// creates neither the cache file nor its directory.
#[test]
fn test_workspace_with_no_cache_is_reported_and_nothing_is_created() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("qdev.toml"), "[project]\nname = \"NoCache\"\n").unwrap();

    let pulse = build_without_store(root);

    // The whole report is the degraded one — `sprints: Some(vec![])` (not `null`, this
    // is a workspace), no gate evidence, and `next: null` because `select_next` cannot
    // run without a store.
    assert_eq!(
        pulse,
        degraded_report(),
        "a missing cache is reported, never rebuilt"
    );

    // Nothing was created to fill the gap: not the cache, not even its directory.
    assert!(
        !root.join(".qdev/cache/cache.sqlite").exists(),
        "the pulse must not create the cache it reports on"
    );
    assert!(
        !root.join(".qdev").exists(),
        "the pulse must not create the cache directory either"
    );
    assert_eq!(
        snapshot_tree(root),
        BTreeMap::from([(
            "qdev.toml".to_string(),
            "[project]\nname = \"NoCache\"\n".as_bytes().to_vec()
        )]),
        "the workspace is untouched"
    );
}

/// A cache whose stamp this binary cannot read — never stamped, stamped at some older
/// version, or carrying a legacy table — is reported as degraded, and nothing opens it,
/// so no sweep and no restamp.
#[test]
fn test_unreadable_cache_is_reported_and_left_alone() {
    for stamp in [None, Some(1)] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::write(
            root.join("qdev.toml"),
            "[project]\nname = \"HalfMigrated\"\n",
        )
        .unwrap();
        let store = unreadable_store(root, stamp);
        seed_story(&store, "E12S1", "ready");
        seed_sprint(&store, 5, "active", &["E12S1"]);

        let cache_path = root.join(".qdev/cache/cache.sqlite");
        assert_eq!(
            qdev_core::inspect_cache_schema(&cache_path).unwrap(),
            CacheSchemaStatus::Mismatch,
            "fixture {stamp:?} must read as a mismatch"
        );
        let before = fs::read(&cache_path).unwrap();

        // The CLI hands no store in this state, and `build_pulse` handles that inside a
        // workspace instead of erroring with `cache_unavailable`.
        let pulse = build_without_store(root);
        assert_eq!(
            pulse,
            degraded_report(),
            "stamp {stamp:?}: the cache is reported, not repaired"
        );

        // Nothing opened the cache: same bytes, same stamp, same rows.
        let after = fs::read(&cache_path).unwrap();
        assert_eq!(after, before, "cache bytes changed for stamp {stamp:?}");
        let conn = qdev_core::rusqlite::Connection::open(&cache_path).unwrap();
        let entities: i64 = conn
            .query_row(
                "SELECT count(*) FROM entities WHERE id IN ('E12S1', 'sprint-5');",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            entities, 2,
            "the cached rows survive untouched for stamp {stamp:?}"
        );
    }
}

/// A cache with an extra table (missing-from-`ALL_TABLE_NAMES` is already covered by the
/// never-stamped fixtures) is also a `Mismatch` and also reported, not rebuilt.
#[test]
fn test_cache_with_a_legacy_table_is_reported_not_rebuilt() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("qdev.toml"), "[project]\nname = \"Legacy\"\n").unwrap();
    let store = open_store(root);
    seed_story(&store, "E12S1", "ready");
    seed_sprint(&store, 5, "active", &["E12S1"]);
    add_legacy_table(root);

    let cache_path = root.join(".qdev/cache/cache.sqlite");
    assert_eq!(
        qdev_core::inspect_cache_schema(&cache_path).unwrap(),
        CacheSchemaStatus::Mismatch,
        "an extra table is a mismatch"
    );
    let before = fs::read(&cache_path).unwrap();

    assert_eq!(
        build_without_store(root),
        degraded_report(),
        "the legacy-table cache is reported, not rebuilt"
    );
    assert_eq!(
        fs::read(&cache_path).unwrap(),
        before,
        "the cache must not be dropped or restamped"
    );
    // The rows are all still there — the pulse reported them instead of rebuilding them.
    let conn = qdev_core::rusqlite::Connection::open(&cache_path).unwrap();
    let rows: i64 = conn
        .query_row(
            "SELECT count(*) FROM entities WHERE id IN ('E12S1', 'sprint-5');",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(rows, 2, "the cached rows survive untouched");
}

/// With no store, git probes, the workspace flag, and the lease read all still work.
#[test]
fn test_git_and_leases_still_report_without_a_store() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::write(root.join("qdev.toml"), "[project]\nname = \"GitOnly\"\n").unwrap();
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    // A lease held by this very worktree, with no cache anywhere.
    seed_lease(root, "E12S4", "simon", "HERE");

    // Commit everything (the lease record included), so `git status --porcelain` is
    // empty and the working tree reports clean.
    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "base"]);

    let pulse = build_without_store(root);
    let env = pulse.environment.expect("the environment renders");

    // The working-tree probes ran against the real repository.
    let working_tree = env.working_tree.expect("git reports a working tree");
    assert!(working_tree.git_repository);
    assert_eq!(working_tree.clean, Some(true));
    assert_eq!(working_tree.dirty_files, Some(0));
    assert_eq!(working_tree.branch.as_deref(), Some("main"));
    assert!(working_tree.head.is_some(), "short SHA still resolves");

    // The integration comparison still runs, and names the refs it could not find —
    // neither `develop` nor `origin/develop` exists in this fresh repository, and no
    // remote update is ever attempted to help.
    let integration = env.integration.expect("integration line renders");
    assert_eq!(integration.remote, "origin");
    assert_eq!(integration.branch, "develop");
    assert_eq!(integration.state, "refs_missing");
    assert_eq!(integration.ahead, None);
    assert_eq!(integration.behind, None);

    // The lease read still works with no cache.
    let leases = env.lease.expect("this worktree's lease still lists");
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].story_id, "E12S4");
    assert_eq!(leases[0].holder, "simon");

    // …while everything that needs cache rows stays empty.
    assert_eq!(pulse.sprints, Some(vec![]));
    assert!(pulse.gates.is_none());
    assert!(pulse.next.is_none());
}

// ---------------------------------------------------------------------------
// Outside a workspace (short-circuit)
// ---------------------------------------------------------------------------

#[test]
fn test_outside_workspace_payload_is_all_null() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // No store at all: outside a workspace the pulse must not open or create one.

    let config = test_config();
    let options = PulseOptions {
        workspace: false,
        workspace_root: root,
        store: None,
        config: &config,
        author: test_author(),
        now: fixed_now(),
    };
    let pulse = qdev_core::build_pulse(&options).unwrap();
    assert_eq!(
        pulse,
        PulsePayload {
            workspace: false,
            environment: None,
            sprints: None,
            gates: None,
            next: None,
        }
    );
}

// ---------------------------------------------------------------------------
// Sync age: computed from a real `sync_meta` row against a fixed `now`
// ---------------------------------------------------------------------------

/// The seconds→milliseconds conversion in the cache section must be pinned against a
/// seeded `sync_meta` row (Review row 26): with `fixed_now()` = 2025-10-09T08:53:20Z
/// and a row stamped 08:33:20Z, the pulse reports exactly 1_200_000 ms — not 1_200
/// (epoch seconds mistaken for milliseconds) and not some other scale.
#[test]
fn test_synced_ms_ago_computes_from_a_seeded_sync_row() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = open_store(root);

    let cache_db = root.join(".qdev/cache/cache.sqlite");
    {
        let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
        conn.execute(
            "INSERT INTO sync_meta (id, last_synced_at) VALUES (1, '2025-10-09T08:33:20Z')
             ON CONFLICT(id) DO UPDATE SET last_synced_at = excluded.last_synced_at;",
            [],
        )
        .unwrap();
    }

    let payload = build(root, &store);
    let env = payload
        .environment
        .as_ref()
        .expect("workspace renders environment");
    assert_eq!(
        env.cache,
        CacheHealth {
            schema_status: "ok".to_string(),
            entity_count: env.cache.entity_count,
            finding_count: Some(0),
            synced_ms_ago: Some(1_200_000),
        },
        "the seeded row must age exactly 1200 s against the fixed clock"
    );

    // A stamp in the future clamps to zero — never a negative or wrapped age.
    {
        let conn = qdev_core::rusqlite::Connection::open(&cache_db).unwrap();
        conn.execute(
            "UPDATE sync_meta SET last_synced_at = '2026-01-01T00:00:00Z' WHERE id = 1;",
            [],
        )
        .unwrap();
    }
    let payload = build(root, &store);
    let env = payload
        .environment
        .as_ref()
        .expect("workspace renders environment");
    assert_eq!(
        env.cache.synced_ms_ago,
        Some(0),
        "a future stamp must clamp to 0, not wrap"
    );
}

// ---------------------------------------------------------------------------
// Job plumbing for the shuffled determinism test
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Job {
    Sprint(i64, &'static str),
    Assign(i64, &'static str),
    Story(&'static str, &'static str),
    Rel(&'static str, &'static str),
    Dw(&'static str, &'static str, &'static str),
    GateRun(&'static str, &'static str, &'static str, &'static str),
}
