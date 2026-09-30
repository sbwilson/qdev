//! Story 4.1: `qdev context` projection tests — reference-fixture budgets, per-phase section
//! sets, the budget walk (truncation order, first-section exemption), `--stats` numbers,
//! byte-identical repeated builds, and the I/O-matrix error cases.

use std::fs;
use std::path::Path;

use qdev_core::config::{Config, GateConfig};
use qdev_core::context::{
    build_context, render_context_markdown, render_context_text, ContextOptions, ContextPhase,
    HYGIENE_DIRECTIVE,
};
use qdev_core::schema::EntityKind;
use qdev_core::scratch::estimate_tokens;
use qdev_core::store::{
    ConstraintRecord, EntityRecord, GateRunRecord, RelationRecord, ScratchpadRecord, SqliteStore,
    Store,
};
use qdev_core::write::Author;
use qdev_core::ExitCode;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixture: the reference E12S4 workspace (mirrors query_tests' setup_e12s4),
// plus the entity files the projection re-reads.
// ---------------------------------------------------------------------------

fn author() -> Author {
    Author::new("human", "simon")
}

fn base_entity(id: &str, kind: EntityKind, source_path: &str) -> EntityRecord {
    EntityRecord {
        id: id.to_string(),
        kind,
        title: Some(format!("{} title", id)),
        status: Some("draft".to_string()),
        owners: None,
        source_path: source_path.to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(author()),
        updated_by: Some(author()),
        updated_at: "2026-09-08T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    }
}

/// The reference fixture: story E12S4 under epic E12 with the cli-reference `get` envelope's
/// shape — own no_go + inherited rabbit_hole constraints, depends_on/traces_to/governed_by
/// relations, two target modules — plus the files and a scratchpad entry it re-reads.
fn setup_reference(root: &Path, store: &SqliteStore) {
    store
        .upsert_entity(&EntityRecord {
            status: Some("active".to_string()),
            title: Some("Core Engine Architecture".to_string()),
            ..base_entity("E12", EntityKind::Epic, "docs/specs/epics/E12.md")
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            title: Some("CoreResponse Buffer Layout".to_string()),
            status: Some("ready".to_string()),
            epic_id: Some("E12".to_string()),
            seq: Some(4),
            owners: Some(r#"["simon"]"#.to_string()),
            target_modules: Some(r#"["bridge","foundation"]"#.to_string()),
            ..base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md")
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            status: Some("done".to_string()),
            epic_id: Some("E12".to_string()),
            seq: Some(3),
            ..base_entity("E12S3", EntityKind::Story, "docs/specs/stories/E12S3.md")
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            title: Some("Response Buffering".to_string()),
            ..base_entity("FR-102", EntityKind::Requirement, "docs/specs/requirements/FR-102.md")
        })
        .unwrap();
    store
        .upsert_entity(&EntityRecord {
            status: Some("accepted".to_string()),
            title: Some("Bounded Response Buffers".to_string()),
            ..base_entity("AD-43", EntityKind::Adr, "docs/specs/adrs/AD-43.md")
        })
        .unwrap();

    store
        .upsert_constraint(&ConstraintRecord {
            id: "E12S4/NG-1".to_string(),
            owner_id: "E12S4".to_string(),
            kind: "no_go".to_string(),
            text: "Do not implement Swift decoding".to_string(),
        })
        .unwrap();
    store
        .upsert_constraint(&ConstraintRecord {
            id: "E12/RH-2".to_string(),
            owner_id: "E12".to_string(),
            kind: "rabbit_hole".to_string(),
            text: "len == 0 does not mean empty result".to_string(),
        })
        .unwrap();

    for (relation, target) in [
        ("depends_on", "E12S3"),
        ("traces_to", "FR-102"),
        ("governed_by", "AD-43"),
    ] {
        store
            .upsert_relation(&RelationRecord {
                source_id: "E12S4".to_string(),
                relation: relation.to_string(),
                target_id: target.to_string(),
            })
            .unwrap();
    }

    store
        .upsert_scratchpad_entry(&ScratchpadRecord {
            story_id: "E12S4".to_string(),
            seq: 1,
            at: "2026-09-08T00:00:00Z".to_string(),
            author_type: Some("human".to_string()),
            author_id: Some("simon".to_string()),
            kind: Some("decision".to_string()),
            text: Some("Keep the buffer ring at 16 entries".to_string()),
        })
        .unwrap();

    // The files the projection re-reads (the cache rows above name them).
    let specs = root.join("docs/specs");
    fs::create_dir_all(specs.join("epics")).unwrap();
    fs::create_dir_all(specs.join("stories")).unwrap();
    fs::create_dir_all(specs.join("adrs")).unwrap();
    fs::create_dir_all(specs.join("requirements")).unwrap();

    fs::write(
        specs.join("epics/E12.md"),
        r#"---
id: E12
title: "Core Engine Architecture"
status: active
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# Core Engine Architecture

## Goal
Deliver a zero-copy response path.
"#,
    )
    .unwrap();
    fs::write(
        specs.join("stories/E12S4.md"),
        r#"---
id: E12S4
title: "CoreResponse Buffer Layout"
status: ready
version: 1
epic_id: E12
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Buffers round-trip without copying.
- Layout is stable across the C ABI.
"#,
    )
    .unwrap();
    fs::write(
        specs.join("adrs/AD-43.md"),
        r#"---
id: AD-43
title: "Bounded Response Buffers"
status: accepted
version: 1
decision: "Bound the ring at 16 entries"
prevents:
  - "unbounded queue growth"
  - "O(n) drain scans"
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# Bounded Response Buffers
"#,
    )
    .unwrap();
    fs::write(
        specs.join("requirements/FR-102.md"),
        r#"---
id: FR-102
title: "Response Buffering"
status: accepted
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#,
    )
    .unwrap();
}

/// A config with the fixture's modules and one review-bound command gate.
fn test_config() -> Config {
    let mut config = Config::default();
    config.modules.push(qdev_core::config::ModuleConfig {
        id: "bridge".to_string(),
        paths: vec!["crates/bridge/**".to_string()],
        layer: None,
        may_depend_on: vec!["foundation".to_string()],
    });
    config.modules.push(qdev_core::config::ModuleConfig {
        id: "foundation".to_string(),
        paths: vec!["crates/foundation/**".to_string()],
        layer: Some(0),
        may_depend_on: vec![],
    });
    config.gates.push(GateConfig {
        id: "c-abi-round-trip".to_string(),
        command: Some("cargo test".to_string()),
        timeout_ms: None,
        depends_on: vec![],
        output_adapter: None,
        on_transition: vec!["review".to_string()],
        verifies: vec![],
        kind: None,
        metric: None,
        direction: None,
        skip: None,
    });
    config
}

fn options(phase: ContextPhase, budget: Option<u32>, stats: bool) -> ContextOptions {
    ContextOptions {
        phase,
        budget,
        stats,
    }
}

// ---------------------------------------------------------------------------
// Develop: the reference happy path
// ---------------------------------------------------------------------------

#[test]
fn test_develop_reference_fixture_section_set_and_priority() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let config = test_config();
    let payload = build_context(
        temp.path(),
        &store,
        &config,
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    // The develop section set, in fixed priority order (architecture.md §12).
    let names: Vec<&str> = payload
        .sections
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "story_spec",
            "constraints",
            "modules",
            "adr_excerpts",
            "requirements",
            "scratchpad",
            "gates",
            "hygiene"
        ]
    );
    let priorities: Vec<u32> = payload.sections.iter().map(|s| s.priority).collect();
    assert_eq!(priorities, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    // Default budget is the develop typical budget; the small fixture fits it.
    assert_eq!(payload.budget, 1200);
    assert!(
        payload.total_tokens <= 1200,
        "reference develop fixture must project under 1,200 tokens (got {})",
        payload.total_tokens
    );
    assert!(payload.truncated.is_empty());
    assert!(payload.stats.is_none(), "no --stats: no stats object");

    // Every section is non-empty for this fixture.
    for section in &payload.sections {
        assert_ne!(section.content, "(none)", "section {} unexpectedly empty", section.name);
    }
}

#[test]
fn test_develop_sections_carry_the_right_content() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let config = test_config();
    let payload = build_context(
        temp.path(),
        &store,
        &config,
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    let section = |name: &str| {
        payload
            .sections
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .content
            .clone()
    };

    // 1. Spec body: the file's body with the frontmatter stripped.
    let spec = section("story_spec");
    assert!(spec.contains("## Acceptance Criteria"));
    assert!(spec.contains("round-trip"));
    assert!(!spec.contains("id: E12S4"), "frontmatter must be stripped");

    // 2. Constraints: own first, then inherited, full ids, tagged.
    let constraints = section("constraints");
    let ng_pos = constraints.find("E12S4/NG-1").unwrap();
    let rh_pos = constraints.find("E12/RH-2").unwrap();
    assert!(
        ng_pos < rh_pos,
        "own constraints must precede inherited ones"
    );
    assert!(constraints.contains("[inherited from E12]"));
    assert!(constraints.contains("Do not implement Swift decoding"));

    // 3. Modules: id -> declared paths, id-sorted.
    let modules = section("modules");
    let bridge_pos = modules.find("bridge ->").unwrap();
    let foundation_pos = modules.find("foundation ->").unwrap();
    assert!(
        bridge_pos < foundation_pos,
        "module ids must be sorted"
    );
    assert!(modules.contains("crates/bridge/**"));

    // 4. ADR excerpt: Rule from frontmatter `decision`, Prevents from `prevents`.
    let adrs = section("adr_excerpts");
    assert!(adrs.contains("AD-43"));
    assert!(adrs.contains("Rule: Bound the ring at 16 entries"));
    assert!(adrs.contains("Prevents: unbounded queue growth; O(n) drain scans"));

    // 5. Linked requirement titles.
    let requirements = section("requirements");
    assert!(requirements.contains("FR-102"));
    assert!(requirements.contains("Response Buffering"));

    // 6. Scratchpad summary: the key decision entry, seq-ordered, no timestamps.
    let scratchpad = section("scratchpad");
    assert!(scratchpad.contains("#1 (decision): Keep the buffer ring at 16 entries"));
    assert!(!scratchpad.contains("2026-09-08"), "no timestamps in output");

    // 7. Bound gates: built-ins in execution order first, then configured, id-sorted.
    let gates = section("gates");
    let scope_pos = gates.find("qdev-scope").unwrap();
    let deps_pos = gates.find("qdev-deps").unwrap();
    let hygiene_pos = gates.find("qdev-hygiene").unwrap();
    let custom_pos = gates.find("c-abi-round-trip").unwrap();
    assert!(scope_pos < deps_pos && deps_pos < hygiene_pos && hygiene_pos < custom_pos);
    assert!(gates.contains("c-abi-round-trip (command [review])"));

    // 8. Hygiene directive & citation templates: verbatim from compliance-and-safety.md §1 plus derived templates.
    let hygiene = section("hygiene");
    assert!(hygiene.contains(HYGIENE_DIRECTIVE));
    assert!(hygiene.contains("rust: // [{id}] {summary}"));
    assert!(hygiene.contains("swift: // [{id}] {summary}"));
    assert!(hygiene.contains("python: # [{id}] {summary}"));
    assert!(HYGIENE_DIRECTIVE.contains("`qdev scratch append`"));
}

#[test]
fn test_develop_dangling_relation_targets_are_marked_unresolved() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        "---\nid: E12S4\ntitle: T\nstatus: ready\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\nBody.\n",
    )
    .unwrap();
    store
        .upsert_entity(&base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md"))
        .unwrap();
    // No rows for AD-99 / FR-999: both dangling.
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "governed_by".to_string(),
            target_id: "AD-99".to_string(),
        })
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "traces_to".to_string(),
            target_id: "FR-999".to_string(),
        })
        .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    assert!(
        payload
            .sections
            .iter()
            .find(|s| s.name == "adr_excerpts")
            .unwrap()
            .content
            .contains("AD-99 (unresolved)")
    );
    assert!(
        payload
            .sections
            .iter()
            .find(|s| s.name == "requirements")
            .unwrap()
            .content
            .contains("FR-999 (unresolved)")
    );
}

#[test]
fn test_develop_empty_sections_stay_present_with_none_marker() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        "---\nid: E12S4\ntitle: T\nstatus: ready\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\nBody.\n",
    )
    .unwrap();
    store
        .upsert_entity(&base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md"))
        .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    // The section set depends only on the phase, never on the data shape.
    assert_eq!(payload.sections.len(), 8);
    for name in ["constraints", "modules", "adr_excerpts", "requirements", "scratchpad"] {
        let section = payload.sections.iter().find(|s| s.name == name).unwrap();
        assert_eq!(
            section.content,
            "(none)",
            "empty {} must be (none)-marked, not dropped",
            name
        );
        assert!(!section.truncated);
    }
}

// ---------------------------------------------------------------------------
// Specify / review: the §12 section sets
// ---------------------------------------------------------------------------

#[test]
fn test_specify_section_set_and_content() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let payload = build_context(
        temp.path(),
        &store,
        &test_config(),
        &options(ContextPhase::Specify, None, false),
        "E12S4",
    )
    .unwrap();

    let names: Vec<&str> = payload.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "epic_goal",
            "epic_constraints",
            "sibling_stories",
            "adr_summaries",
            "requirements"
        ]
    );
    assert_eq!(payload.budget, 800, "specify default budget");

    let section = |name: &str| {
        payload
            .sections
            .iter()
            .find(|s| s.name == name)
            .unwrap()
            .content
            .clone()
    };

    // Epic goal: the body's `## Goal` section, under the epic title.
    let goal = section("epic_goal");
    assert!(goal.contains("E12"));
    assert!(goal.contains("Deliver a zero-copy response path."));

    // Epic constraints, full ids.
    assert!(section("epic_constraints").contains("E12/RH-2 (rabbit_hole):"));

    // Sibling stories: seq-ordered, the target marked current.
    let siblings = section("sibling_stories");
    let s3 = siblings.find("E12S3").unwrap();
    let s4 = siblings.find("E12S4").unwrap();
    assert!(s3 < s4, "siblings must be seq-ordered");
    assert!(siblings.contains("E12S4 \"CoreResponse Buffer Layout\" (ready) (current)"));

    // ADR summaries: one line each, title + decision.
    let adrs = section("adr_summaries");
    assert!(adrs.contains("AD-43 \"Bounded Response Buffers\": Bound the ring at 16 entries"));

    // Linked requirements.
    assert!(section("requirements").contains("FR-102 \"Response Buffering\""));
}

#[test]
fn test_review_section_set_extends_develop() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let payload = build_context(
        temp.path(),
        &store,
        &test_config(),
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    let names: Vec<&str> = payload.sections.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "story_spec",
            "constraints",
            "modules",
            "adr_excerpts",
            "requirements",
            "scratchpad",
            "gates",
            "hygiene",
            "diff",
            "hygiene_findings",
            "gate_receipts",
            "evidence"
        ]
    );
    assert_eq!(payload.budget, 2500, "review default budget");

    // Outside a git repository the diff summary is present but empty with a reason note.
    let diff = payload
        .sections
        .iter()
        .find(|s| s.name == "diff")
        .unwrap()
        .content
        .clone();
    assert!(
        diff.starts_with("(empty: "),
        "diff must be empty with a reason note, got: {diff}"
    );

    let hygiene_findings = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene_findings")
        .unwrap()
        .content
        .clone();
    assert!(
        hygiene_findings.starts_with("(empty: "),
        "hygiene_findings must be empty with a reason note outside git worktree, got: {hygiene_findings}"
    );

    // No runs yet: both review extras are (none)-marked but present.
    for name in ["gate_receipts", "evidence"] {
        assert_eq!(
            payload
                .sections
                .iter()
                .find(|s| s.name == name)
                .unwrap()
                .content,
            "(none)"
        );
    }
}

#[test]
fn test_review_gate_receipts_and_evidence_use_latest_runs_per_gate() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        "---\nid: E12S4\ntitle: T\nstatus: ready\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\nBody.\n",
    )
    .unwrap();
    store
        .upsert_entity(&base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md"))
        .unwrap();

    store
        .upsert_gate_run(&GateRunRecord {
            id: "run-old-scope".to_string(),
            story_id: Some("E12S4".to_string()),
            gate_id: "qdev-scope".to_string(),
            commit_sha: "aaa".to_string(),
            status: Some("pass".to_string()),
            exit_code: Some(0),
            duration_ms: Some(10),
            metric_value: None,
            summary: Some("old scope pass".to_string()),
            evidence_path: "docs/state/evidence/E12S4/old-scope.json".to_string(),
            output_hash: None,
            run_by_type: Some("agent".to_string()),
            run_by_id: Some("claude".to_string()),
            ran_at: Some("2026-09-01T00:00:00Z".to_string()),
        })
        .unwrap();
    store
        .upsert_gate_run(&GateRunRecord {
            id: "run-new-scope".to_string(),
            story_id: Some("E12S4".to_string()),
            gate_id: "qdev-scope".to_string(),
            commit_sha: "bbb".to_string(),
            status: Some("fail".to_string()),
            exit_code: Some(1),
            duration_ms: Some(12),
            metric_value: None,
            summary: Some("new scope fail".to_string()),
            evidence_path: "docs/state/evidence/E12S4/new-scope.json".to_string(),
            output_hash: None,
            run_by_type: Some("agent".to_string()),
            run_by_id: Some("claude".to_string()),
            ran_at: Some("2026-09-02T00:00:00Z".to_string()),
        })
        .unwrap();
    store
        .upsert_gate_run(&GateRunRecord {
            id: "run-custom".to_string(),
            story_id: Some("E12S4".to_string()),
            gate_id: "c-abi".to_string(),
            commit_sha: "ccc".to_string(),
            status: Some("pass".to_string()),
            exit_code: Some(0),
            duration_ms: Some(5),
            metric_value: None,
            summary: None,
            evidence_path: "docs/state/evidence/E12S4/c-abi.json".to_string(),
            output_hash: None,
            run_by_type: Some("agent".to_string()),
            run_by_id: Some("claude".to_string()),
            ran_at: Some("2026-09-02T01:00:00Z".to_string()),
        })
        .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    let receipts = payload
        .sections
        .iter()
        .find(|s| s.name == "gate_receipts")
        .unwrap()
        .content
        .clone();
    // Only the latest run per gate — the older scope run is gone.
    assert!(receipts.contains("c-abi [pass]"));
    assert!(receipts.contains("qdev-scope [fail] new scope fail"));
    assert!(!receipts.contains("old scope pass"));
    let c_pos = receipts.find("c-abi").unwrap();
    let s_pos = receipts.find("qdev-scope").unwrap();
    assert!(c_pos < s_pos, "receipts must be gate-id-sorted");

    let evidence = payload
        .sections
        .iter()
        .find(|s| s.name == "evidence")
        .unwrap()
        .content
        .clone();
    assert!(evidence.contains("docs/state/evidence/E12S4/c-abi.json"));
    assert!(evidence.contains("docs/state/evidence/E12S4/new-scope.json"));
    assert!(!evidence.contains("old-scope.json"));
    assert!(
        evidence.find("c-abi.json").unwrap() < evidence.find("new-scope.json").unwrap(),
        "evidence paths must be path-sorted"
    );
}

#[test]
fn test_review_diff_summary_counts_changes_since_integration_branch() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    let git = |args: &[&str]| {
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
    };

    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "simon@example.com"]);
    git(&["config", "user.name", "Simon"]);
    git(&["config", "commit.gpgsign", "false"]);
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(root.join("a.txt"), "one\n").unwrap();
    fs::write(root.join("b.txt"), "one\n").unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        "---\nid: E12S4\ntitle: T\nstatus: ready\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\nBody.\n",
    )
    .unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-m", "base"]);
    git(&["branch", "develop"]); // the default [git] integration_branch

    // One committed change since the baseline, one staged, one untracked.
    fs::write(root.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&["commit", "-am", "add two lines"]);
    fs::write(root.join("b.txt"), "one\nstaged\n").unwrap();
    git(&["add", "b.txt"]);
    fs::write(root.join("c.txt"), "u1\nu2\nu3\n").unwrap();

    store
        .upsert_entity(&base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md"))
        .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    let diff = payload
        .sections
        .iter()
        .find(|s| s.name == "diff")
        .unwrap()
        .content
        .clone();
    assert!(
        diff.contains("a.txt +2 -0"),
        "committed leg: a.txt gained two lines: {diff}"
    );
    assert!(
        diff.contains("b.txt +1 -0"),
        "staged leg: b.txt gained one line: {diff}"
    );
    assert!(
        diff.contains("c.txt +3 -0"),
        "untracked leg: c.txt counts its three lines: {diff}"
    );
    // Path-sorted.
    let a = diff.find("a.txt").unwrap();
    let b = diff.find("b.txt").unwrap();
    let c = diff.find("c.txt").unwrap();
    assert!(a < b && b < c);
}

// ---------------------------------------------------------------------------
// The budget walk
// ---------------------------------------------------------------------------

#[test]
fn test_first_section_alone_over_budget_is_kept_whole() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    // A ~100-token spec body: 400 characters.
    let body = "w".repeat(400);
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        format!(
            "---\nid: E12S4\ntitle: T\nstatus: ready\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\n{}\n",
            body
        ),
    )
    .unwrap();
    store
        .upsert_entity(&base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md"))
        .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, Some(30), true),
        "E12S4",
    )
    .unwrap();

    // The first section is exempt: kept whole even though it alone exceeds the budget.
    assert_eq!(payload.sections.len(), 1);
    assert_eq!(payload.sections[0].name, "story_spec");
    assert!(!payload.sections[0].truncated);
    assert_eq!(payload.sections[0].content, body);
    assert!(
        payload.total_tokens > payload.budget,
        "the exemption may overrun; total is {} against budget {}",
        payload.total_tokens,
        payload.budget
    );

    // Every other section is dropped, recorded in drop order.
    let truncated_names: Vec<&str> = payload
        .truncated
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(
        truncated_names,
        vec![
            "constraints", "modules", "adr_excerpts", "requirements", "scratchpad", "gates",
            "hygiene"
        ]
    );

    // Truncation rendering in text and markdown modes
    let text = render_context_text(&payload);
    assert!(
        text.contains("Truncated (drop order)"),
        "text rendering must contain truncation header: {text}"
    );
    let md = render_context_markdown(&payload);
    assert!(
        md.contains("## truncated"),
        "markdown rendering must contain '## truncated': {md}"
    );
    assert!(
        md.contains("Trimmed or dropped by the budget walk, in drop order:"),
        "markdown rendering must contain truncation header: {md}"
    );

    // --stats surfaces the overrun.
    let stats = payload.stats.as_ref().unwrap();
    assert_eq!(stats.budget, 30);
    assert!(stats.over_budget);
    assert_eq!(stats.total_tokens, payload.total_tokens);
}

#[test]
fn test_over_budget_trims_and_drops_lowest_priority_first_in_order() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    // story_spec: 18 chars -> 5 tokens. Fits a budget of 50, leaving 45.
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S4.md"),
        "---\nid: E12S4\ntitle: T\nstatus: ready\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\nline one\nline two\n",
    )
    .unwrap();
    store
        .upsert_entity(&base_entity("E12S4", EntityKind::Story, "docs/specs/stories/E12S4.md"))
        .unwrap();

    // constraints: ten 20-char lines; the rendered section (with the `id (kind): ` prefix)
    // is 228 chars -> 57 tokens, and the 45 tokens remaining cannot hold it whole.
    let constraint_line = "a".repeat(20);
    let full_constraint_text = std::iter::once(format!("E12S4/NG-1 (no_go): {constraint_line}"))
        .chain(std::iter::repeat(constraint_line.clone()).take(9))
        .collect::<Vec<_>>()
        .join("\n");
    store
        .upsert_constraint(&ConstraintRecord {
            id: "E12S4/NG-1".to_string(),
            owner_id: "E12S4".to_string(),
            kind: "no_go".to_string(),
            text: vec![constraint_line.as_str(); 10]
                .join("\n"),
        })
        .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, Some(50), false),
        "E12S4",
    )
    .unwrap();

    let names: Vec<&str> = payload
        .sections
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    // story_spec (18 chars = 5 tokens) + trimmed constraints (7 lines = 165 chars = 42
    // tokens) + modules "(none)" (2) = 49 <= 50; everything lower priority drops.
    assert_eq!(names, vec!["story_spec", "constraints", "modules"]);

    let constraints = payload
        .sections
        .iter()
        .find(|s| s.name == "constraints")
        .unwrap();
    assert!(
        constraints.truncated,
        "a partially fitting section is flagged truncated"
    );
    let lines: Vec<&str> = constraints.content.lines().collect();
    assert_eq!(lines.len(), 7, "deterministic line trim keeps 7 of 10 lines");
    assert!(
        lines[0].starts_with("E12S4/NG-1 (no_go): "),
        "trimming keeps the head of the section"
    );
    assert_eq!(estimate_tokens(&constraints.content) as u32, constraints.tokens);

    // Drop order: the trimmed section first (walk order), then the dropped ones.
    let truncated_names: Vec<&str> = payload
        .truncated
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(
        truncated_names,
        vec![
            "constraints", "adr_excerpts", "requirements", "scratchpad", "gates", "hygiene"
        ]
    );
    // The record carries each section's full pre-trim token count.
    let constraints_rec = payload
        .truncated
        .iter()
        .find(|t| t.name == "constraints")
        .unwrap();
    assert_eq!(
        constraints_rec.tokens,
        estimate_tokens(&full_constraint_text) as u32
    );

    assert!(payload.total_tokens <= 50);

    let text = render_context_text(&payload);
    assert!(
        text.contains("Truncated (drop order)"),
        "text rendering must contain truncation header: {text}"
    );
    let md = render_context_markdown(&payload);
    assert!(
        md.contains("## truncated"),
        "markdown rendering must contain '## truncated': {md}"
    );
    assert!(
        md.contains("Trimmed or dropped by the budget walk, in drop order:"),
        "markdown rendering must contain truncation header: {md}"
    );
}

// ---------------------------------------------------------------------------
// --stats
// ---------------------------------------------------------------------------

#[test]
fn test_stats_numbers_match_the_payload() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let payload = build_context(
        temp.path(),
        &store,
        &test_config(),
        &options(ContextPhase::Develop, None, true),
        "E12S4",
    )
    .unwrap();

    let stats = payload.stats.as_ref().expect("--stats must set the stats object");
    assert_eq!(stats.budget, payload.budget);
    assert_eq!(stats.total_tokens, payload.total_tokens);
    assert!(!stats.over_budget);
    assert_eq!(stats.sections.len(), payload.sections.len());
    for section in &payload.sections {
        assert_eq!(
            stats.sections.get(section.name.as_str()),
            Some(&section.tokens),
            "per-section stats must match the section"
        );
        assert_eq!(section.tokens, estimate_tokens(&section.content) as u32);
    }
    assert!(
        stats.total_tokens
            == stats.sections.values().sum::<u32>(),
        "stats total must be the per-section sum"
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn test_repeat_builds_are_byte_identical() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);
    let config = test_config();

    let payload = build_context(
        temp.path(),
        &store,
        &config,
        &options(ContextPhase::Review, None, true),
        "E12S4",
    )
    .unwrap();

    for _ in 0..2 {
        let repeat = build_context(
            temp.path(),
            &store,
            &config,
            &options(ContextPhase::Review, None, true),
            "E12S4",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&payload).unwrap(),
            serde_json::to_string(&repeat).unwrap(),
            "JSON builds must be byte-identical"
        );
        assert_eq!(
            render_context_text(&payload),
            render_context_text(&repeat),
            "text renders must be byte-identical"
        );
        assert_eq!(
            render_context_markdown(&payload),
            render_context_markdown(&repeat),
            "markdown renders must be byte-identical"
        );
    }
}

#[test]
fn test_no_timestamps_in_any_rendering() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let payload = build_context(
        temp.path(),
        &store,
        &test_config(),
        &options(ContextPhase::Review, None, true),
        "E12S4",
    )
    .unwrap();

    let text = render_context_text(&payload);
    let markdown = render_context_markdown(&payload);
    let json = serde_json::to_string(&payload).unwrap();
    for output in [&text, &markdown, &json] {
        assert!(
            !output.contains("2026-09-08"),
            "no timestamps may reach the output: {output}"
        );
    }
}

// ---------------------------------------------------------------------------
// I/O matrix error cases
// ---------------------------------------------------------------------------

#[test]
fn test_non_story_target_is_usage_error_exit_2() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let err = build_context(
        temp.path(),
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, None, false),
        "E12",
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert_eq!(err.code(), "usage_error");
    assert!(err.message().contains("epic"), "{}", err.message());
    assert!(
        err.message().contains("story"),
        "the error must say a story is required: {}",
        err.message()
    );
}

#[test]
fn test_unknown_id_is_logical_error_exit_1() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let err = build_context(
        temp.path(),
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, None, false),
        "E99S9",
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::LogicalFailure);
    assert_eq!(err.code(), "entity_not_found");
}

#[test]
fn test_zero_budget_is_usage_error_exit_2() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let err = build_context(
        temp.path(),
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, Some(0), false),
        "E12S4",
    )
    .unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
}

#[test]
fn test_unknown_phase_is_usage_error_exit_2() {
    let err = ContextPhase::from_str_loose("implement").unwrap_err();
    assert_eq!(err.exit_code(), ExitCode::UsageError);
    assert!(err.message().contains("specify, develop, review"));
}

// ---------------------------------------------------------------------------
// ADR excerpt rules and phase defaults
// ---------------------------------------------------------------------------

#[test]
fn test_adr_body_sections_win_over_frontmatter() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(root, &store);

    // A second ADR whose body carries `## Rule` / `## Prevents` headings: the body wins.
    store
        .upsert_entity(&base_entity("AD-44", EntityKind::Adr, "docs/specs/adrs/AD-44.md"))
        .unwrap();
    store
        .upsert_relation(&RelationRecord {
            source_id: "E12S4".to_string(),
            relation: "governed_by".to_string(),
            target_id: "AD-44".to_string(),
        })
        .unwrap();
    fs::write(
        root.join("docs/specs/adrs/AD-44.md"),
        r#"---
id: AD-44
title: "Body Wins"
status: accepted
version: 1
decision: "frontmatter rule"
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Rule
body rule wins

## Prevents
body prevents wins
"#,
    )
    .unwrap();

    let payload = build_context(
        root,
        &store,
        &Config::default(),
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    let adrs = payload
        .sections
        .iter()
        .find(|s| s.name == "adr_excerpts")
        .unwrap()
        .content
        .clone();
    // AD-43 (frontmatter only) keeps its frontmatter values.
    assert!(adrs.contains("Rule: Bound the ring at 16 entries"));
    // AD-44: body sections win.
    assert!(adrs.contains("Rule: body rule wins"));
    assert!(adrs.contains("Prevents: body prevents wins"));
    assert!(!adrs.contains("frontmatter rule"));
}

#[test]
fn test_default_budgets_per_phase() {
    assert_eq!(ContextPhase::Specify.default_budget(), 800);
    assert_eq!(ContextPhase::Develop.default_budget(), 1200);
    assert_eq!(ContextPhase::Review.default_budget(), 2500);
}

#[test]
fn test_hygiene_directive_matches_compliance_doc_verbatim() {
    let expected = "Write standard code comments. Cite entities with compact bracket tags such as \
`[E12S4]`, `[AD-43]`, `[DEC-2b91]`. Never write narrative history, story summaries, or review \
commentary in code; put reasoning in the scratchpad with `qdev scratch append`.";
    assert_eq!(HYGIENE_DIRECTIVE, expected);
}

/// The payload must serialize cleanly against its own schema — the 1.13 invariant that every
/// shipped payload is schematized.
#[test]
fn test_payload_validates_against_its_own_schema() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let payload = build_context(
        temp.path(),
        &store,
        &test_config(),
        &options(ContextPhase::Review, Some(100), true),
        "E12S4",
    )
    .unwrap();

    let instance = serde_json::to_value(&qdev_core::JsonEnvelope::new(payload)).unwrap();
    let schema = qdev_core::PayloadKind::Context.schema_json();
    let validator = jsonschema::validator_for(&schema).expect("context schema must compile");
    let errors: Vec<String> = validator.iter_errors(&instance).map(|e| e.to_string()).collect();
    assert!(
        errors.is_empty(),
        "payload failed its own schema: {errors:?}"
    );
}

#[test]
fn test_hygiene_section_with_custom_directive() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let mut cfg = test_config();
    cfg.hygiene.directive = Some("Custom engineering rules: be concise.".to_string());

    let payload = build_context(
        temp.path(),
        &store,
        &cfg,
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    let hygiene = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene")
        .unwrap()
        .content
        .clone();

    assert!(hygiene.starts_with("Custom engineering rules: be concise."));
    assert!(hygiene.contains("Citation templates:"));
    assert!(hygiene.contains("rust: // [{id}] {summary}"));
    assert!(hygiene.contains("swift: // [{id}] {summary}"));
    assert!(hygiene.contains("python: # [{id}] {summary}"));
}

#[test]
fn test_hygiene_section_with_custom_citation_template_and_alias() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let mut cfg = test_config();
    cfg.hygiene.citation_template = Some("[{id}]".to_string());

    let payload = build_context(
        temp.path(),
        &store,
        &cfg,
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    let hygiene = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene")
        .unwrap()
        .content
        .clone();

    assert!(hygiene.contains("rust: // [{id}]"));
    assert!(hygiene.contains("swift: // [{id}]"));
    assert!(hygiene.contains("python: # [{id}]"));
}

#[test]
fn test_hygiene_section_with_custom_per_language_templates() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    setup_reference(temp.path(), &store);

    let mut cfg = test_config();
    cfg.hygiene.citation_templates.insert("rust".to_string(), "// [{entity_id}]".to_string());

    let payload = build_context(
        temp.path(),
        &store,
        &cfg,
        &options(ContextPhase::Develop, None, false),
        "E12S4",
    )
    .unwrap();

    let hygiene = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene")
        .unwrap()
        .content
        .clone();

    assert!(hygiene.contains("rust: // [{entity_id}]"));
    assert!(hygiene.contains("swift: // [{id}] {summary}"));
    assert!(hygiene.contains("python: # [{id}] {summary}"));
}

#[test]
fn test_review_hygiene_findings_clean_diff_and_violations() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    let git = |args: &[&str]| {
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
    };

    git(&["init", "-b", "develop"]);
    git(&["config", "user.email", "simon@example.com"]);
    git(&["config", "user.name", "Simon"]);
    git(&["config", "commit.gpgsign", "false"]);

    setup_reference(root, &store);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/lib.rs"), "// standard clean code\npub fn foo() {}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "initial baseline"]);

    // Create feature branch
    git(&["checkout", "-b", "feature/E12S4"]);

    // Clean diff case: add clean code in a new commit
    fs::write(
        root.join("src/lib.rs"),
        "// [E12S4] standard clean citation\npub fn foo() {}\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-m", "clean update"]);

    let mut cfg = test_config();
    cfg.git.integration_branch = "develop".to_string();

    let payload = build_context(
        root,
        &store,
        &cfg,
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    // 1. Clean diff -> hygiene_findings is (none)
    let hygiene_findings = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene_findings")
        .unwrap();
    assert_eq!(hygiene_findings.content, "(none)");
    assert_eq!(hygiene_findings.priority, 10);

    // 2. Add a hygiene violation in diff (story banner)
    fs::write(
        root.join("src/lib.rs"),
        "// ⭐ STORY 1.2: Forensic development memoirs in source comments\npub fn foo() {}\n",
    )
    .unwrap();

    let payload = build_context(
        root,
        &store,
        &cfg,
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    let hygiene_findings = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene_findings")
        .unwrap();
    assert!(
        hygiene_findings.content.contains("src/lib.rs:1: [story_banner]"),
        "expected violation in hygiene_findings: {}",
        hygiene_findings.content
    );

    // 3. Hygiene disabled in config -> hygiene_findings is (none)
    let mut disabled_cfg = cfg.clone();
    disabled_cfg.hygiene.enabled = false;

    let payload_disabled = build_context(
        root,
        &store,
        &disabled_cfg,
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    let hygiene_findings_disabled = payload_disabled
        .sections
        .iter()
        .find(|s| s.name == "hygiene_findings")
        .unwrap();
    assert_eq!(hygiene_findings_disabled.content, "(none)");
}

#[test]
fn test_review_hygiene_findings_unresolvable_diff_baseline() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    let git = |args: &[&str]| {
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
    };

    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "simon@example.com"]);
    git(&["config", "user.name", "Simon"]);
    git(&["config", "commit.gpgsign", "false"]);

    setup_reference(root, &store);
    git(&["add", "."]);
    git(&["commit", "-m", "initial commit on main"]);

    let mut cfg = test_config();
    cfg.git.integration_branch = "develop".to_string();

    let payload = build_context(
        root,
        &store,
        &cfg,
        &options(ContextPhase::Review, None, false),
        "E12S4",
    )
    .unwrap();

    let hygiene_findings = payload
        .sections
        .iter()
        .find(|s| s.name == "hygiene_findings")
        .unwrap();

    assert_eq!(
        hygiene_findings.content,
        "(empty: cannot resolve a diff baseline against 'develop')"
    );
}



