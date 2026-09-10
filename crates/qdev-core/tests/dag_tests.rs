//! Relation kind-pair validation, `depends_on` cycle detection, and hydration wiring
//! (spec-1-10).

use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use qdev_core::dag::{
    allowed_kind_pairs, find_dependency_cycle, is_known_relation, is_valid_kind_pair,
    relation_names, would_create_cycle,
};
use qdev_core::schema::EntityKind;
use qdev_core::store::{ensure_cache, FindingRecord, SqliteStore, Store};
use qdev_core::StorageConfig;

// ---------------------------------------------------------------------------
// dag.rs unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_allowed_kind_pairs_covers_exactly_the_8_relations_from_architecture_md() {
    // depends_on, extends, supersedes, traces_to, verifies, mitigates, closes_dw, governed_by
    assert_eq!(
        allowed_kind_pairs("depends_on"),
        &[(EntityKind::Story, EntityKind::Story)]
    );
    assert_eq!(
        allowed_kind_pairs("extends"),
        &[(EntityKind::Story, EntityKind::Story)]
    );
    assert_eq!(
        allowed_kind_pairs("supersedes"),
        &[
            (EntityKind::Story, EntityKind::Story),
            (EntityKind::Story, EntityKind::Epic),
            (EntityKind::Epic, EntityKind::Story),
            (EntityKind::Epic, EntityKind::Epic),
        ]
    );
    assert_eq!(
        allowed_kind_pairs("traces_to"),
        &[(EntityKind::Story, EntityKind::Requirement)]
    );
    // verifies is Gate -> Requirement; no Gate EntityKind variant exists yet.
    assert!(allowed_kind_pairs("verifies").is_empty());
    assert_eq!(
        allowed_kind_pairs("mitigates"),
        &[(EntityKind::Story, EntityKind::Hazard)]
    );
    assert_eq!(
        allowed_kind_pairs("closes_dw"),
        &[(EntityKind::Story, EntityKind::DeferredWork)]
    );
    assert_eq!(
        allowed_kind_pairs("governed_by"),
        &[
            (EntityKind::Story, EntityKind::Adr),
            (EntityKind::Epic, EntityKind::Adr),
        ]
    );
    assert!(allowed_kind_pairs("not_a_relation").is_empty());
}

/// The relation names and their kind pairs come from one table, so this asserts the two views
/// of it agree: every name `relation_names` yields is `is_known_relation`, exactly the eight
/// architecture.md §8 defines are yielded, and nothing else is known. A ninth relation added to
/// the table gets both views at once — the assertion that would fail here is a name added to
/// this test's expectation and not to the table, or removed from the table and not from here.
#[test]
fn test_relation_names_and_kind_pairs_come_from_one_list() {
    let names: Vec<&str> = relation_names().collect();
    assert_eq!(
        names,
        vec![
            "depends_on",
            "extends",
            "supersedes",
            "traces_to",
            "verifies",
            "mitigates",
            "closes_dw",
            "governed_by",
        ],
        "relation_names must be architecture.md §8's table, in its order"
    );
    for name in &names {
        assert!(is_known_relation(name), "'{name}' must be known");
    }
    for unknown in [
        "dependson",
        "depends-on",
        "",
        "DEPENDS_ON",
        "not_a_relation",
    ] {
        assert!(!is_known_relation(unknown), "'{unknown}' must not be known");
    }
}

/// `verifies` is the case that makes the name list necessary rather than redundant: it is a
/// known relation whose allowed-pair list is empty, exactly like an unknown name's, so the two
/// questions cannot be answered by one function.
#[test]
fn test_verifies_is_known_but_allows_no_pairs_unlike_an_unknown_name() {
    assert!(is_known_relation("verifies"));
    assert!(allowed_kind_pairs("verifies").is_empty());
    assert!(!is_known_relation("verifiez"));
    assert!(allowed_kind_pairs("verifiez").is_empty());
}

#[test]
fn test_is_valid_kind_pair() {
    assert!(is_valid_kind_pair(
        "depends_on",
        EntityKind::Story,
        EntityKind::Story
    ));
    assert!(!is_valid_kind_pair(
        "depends_on",
        EntityKind::Story,
        EntityKind::Epic
    ));
    assert!(!is_valid_kind_pair(
        "verifies",
        EntityKind::Story,
        EntityKind::Requirement
    ));
    assert!(is_valid_kind_pair(
        "governed_by",
        EntityKind::Epic,
        EntityKind::Adr
    ));
}

#[test]
fn test_find_dependency_cycle_none_on_acyclic_graph() {
    let edges = vec![
        ("E1S2".to_string(), "E1S1".to_string()),
        ("E1S3".to_string(), "E1S2".to_string()),
    ];
    assert_eq!(find_dependency_cycle(&edges), None);
}

#[test]
fn test_find_dependency_cycle_detects_simple_cycle() {
    let edges = vec![
        ("E1S1".to_string(), "E1S2".to_string()),
        ("E1S2".to_string(), "E1S1".to_string()),
    ];
    let cycle = find_dependency_cycle(&edges).expect("expected a cycle");
    assert_eq!(cycle.first(), cycle.last());
    assert!(cycle.contains(&"E1S1".to_string()));
    assert!(cycle.contains(&"E1S2".to_string()));
}

#[test]
fn test_find_dependency_cycle_detects_self_loop() {
    let edges = vec![("E1S1".to_string(), "E1S1".to_string())];
    let cycle = find_dependency_cycle(&edges).expect("expected a self-loop cycle");
    assert_eq!(cycle, vec!["E1S1".to_string(), "E1S1".to_string()]);
}

#[test]
fn test_would_create_cycle_true_when_target_can_already_reach_source() {
    // E1S1 depends_on E1S2 already exists. Relating E1S2 depends_on E1S1 would close a cycle.
    let existing = vec![("E1S1".to_string(), "E1S2".to_string())];
    let cycle = would_create_cycle(&existing, "E1S2", "E1S1").expect("expected a would-be cycle");
    assert_eq!(
        cycle,
        vec!["E1S2".to_string(), "E1S1".to_string(), "E1S2".to_string()]
    );
}

#[test]
fn test_would_create_cycle_false_when_unrelated() {
    let existing = vec![("E1S1".to_string(), "E1S2".to_string())];
    assert_eq!(would_create_cycle(&existing, "E1S3", "E1S1"), None);
}

#[test]
fn test_would_create_cycle_self_relation() {
    assert!(would_create_cycle(&[], "E1S1", "E1S1").is_some());
}

// ---------------------------------------------------------------------------
// Hydration integration: kind-pair / dangling / cycle findings
// ---------------------------------------------------------------------------

fn storage() -> StorageConfig {
    StorageConfig::default()
}

fn write_qdev_toml(root: &Path) {
    fs::write(root.join("qdev.toml"), "[project]\nname = \"DagTest\"\n").unwrap();
}

fn story_path(root: &Path, id: &str) -> PathBuf {
    root.join("docs/specs/stories").join(format!("{id}.md"))
}

fn write_story(root: &Path, id: &str, relations_yaml: &str) {
    let path = story_path(root, id);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let relations_block = if relations_yaml.is_empty() {
        String::new()
    } else {
        format!("relations:\n{relations_yaml}")
    };
    fs::write(
        &path,
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: draft
version: 1
created_by:
  type: human
  id: alice
updated_by:
  type: human
  id: alice
{relations_block}---
Body for {id}
"#
        ),
    )
    .unwrap();
}

fn findings_for(store: &SqliteStore, path: &str) -> Vec<FindingRecord> {
    store.get_findings_for_path(path).unwrap()
}

#[test]
fn test_hydration_records_dangling_relation_finding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root);
    write_story(root, "E1S1", "  depends_on: [\"E1S9\"]\n");

    let store = ensure_cache(root, &storage()).unwrap();

    let findings = findings_for(&store, "docs/specs/stories/E1S1.md");
    assert!(
        findings.iter().any(|f| f.code == "dangling_relation"),
        "expected a dangling_relation finding, got {:?}",
        findings
    );

    // The relation is still stored despite the dangling target.
    let rels = store.get_relations_for_source("E1S1").unwrap();
    assert!(rels
        .iter()
        .any(|r| r.relation == "depends_on" && r.target_id == "E1S9"));
}

#[test]
fn test_hydration_records_invalid_relation_kind_finding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root);
    // traces_to must point at a Requirement; here it points at another Story.
    write_story(root, "E1S1", "  traces_to: [\"E1S2\"]\n");
    write_story(root, "E1S2", "");

    let store = ensure_cache(root, &storage()).unwrap();

    let findings = findings_for(&store, "docs/specs/stories/E1S1.md");
    assert!(
        findings.iter().any(|f| f.code == "invalid_relation_kind"),
        "expected an invalid_relation_kind finding, got {:?}",
        findings
    );
}

#[test]
fn test_hydration_records_dependency_cycle_finding_on_every_participant() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root);
    write_story(root, "E1S1", "  depends_on: [\"E1S2\"]\n");
    write_story(root, "E1S2", "  depends_on: [\"E1S1\"]\n");

    let store = ensure_cache(root, &storage()).unwrap();

    for path in ["docs/specs/stories/E1S1.md", "docs/specs/stories/E1S2.md"] {
        let findings = findings_for(&store, path);
        assert!(
            findings.iter().any(|f| f.code == "dependency_cycle"),
            "expected a dependency_cycle finding on '{}', got {:?}",
            path,
            findings
        );
    }
}

#[test]
fn test_hydration_clears_relation_findings_once_fixed() {
    // E1S1's own file is never rewritten after the first sweep; only a second, unrelated file
    // (E1S9) is added. The dangling finding on E1S1 must still clear, proving the validation
    // pass scans the whole `relations` table every sweep, not just the files reparsed that pass.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root);
    write_story(root, "E1S1", "  depends_on: [\"E1S9\"]\n");

    let store = ensure_cache(root, &storage()).unwrap();
    assert!(findings_for(&store, "docs/specs/stories/E1S1.md")
        .iter()
        .any(|f| f.code == "dangling_relation"));

    // Fix it: add the missing target (E1S1's file is untouched), then re-sweep.
    write_story(root, "E1S9", "");
    let store = ensure_cache(root, &storage()).unwrap();

    assert!(
        !findings_for(&store, "docs/specs/stories/E1S1.md")
            .iter()
            .any(|f| f.code == "dangling_relation"),
        "dangling_relation finding should clear once the target exists"
    );
}

// ---------------------------------------------------------------------------
// Multiple cycles, and cycle detection on a deep chain
// ---------------------------------------------------------------------------

/// `validate_relations_graph` strips each cycle's closing edge and looks again, so two disjoint
/// cycles both get reported. Every other cycle test in this file builds exactly one cycle, so
/// the loop never iterates — collapse it to a single `find_dependency_cycle` call and nothing
/// fails, while a workspace with a second live cycle quietly passes CI.
#[test]
fn test_hydration_records_every_disjoint_dependency_cycle() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root);
    write_story(root, "E1S1", "  depends_on: [\"E1S2\"]\n");
    write_story(root, "E1S2", "  depends_on: [\"E1S1\"]\n");
    write_story(root, "E1S3", "  depends_on: [\"E1S4\"]\n");
    write_story(root, "E1S4", "  depends_on: [\"E1S3\"]\n");

    let store = ensure_cache(root, &storage()).unwrap();

    let mut messages = Vec::new();
    for id in ["E1S1", "E1S2", "E1S3", "E1S4"] {
        let path = format!("docs/specs/stories/{}.md", id);
        let cycle_findings: Vec<FindingRecord> = findings_for(&store, &path)
            .into_iter()
            .filter(|f| f.code == "dependency_cycle")
            .collect();
        assert!(
            !cycle_findings.is_empty(),
            "{} participates in a cycle but has no dependency_cycle finding",
            id
        );
        messages.extend(cycle_findings.into_iter().filter_map(|f| f.message));
    }

    messages.sort();
    messages.dedup();
    assert_eq!(
        messages.len(),
        2,
        "both disjoint cycles must be reported, got {:?}",
        messages
    );
}

/// A file can take part in two cycles at once. The `findings` primary key used to be
/// `(path, code)`, which silently kept only whichever row was written last.
#[test]
fn test_hydration_keeps_both_cycles_a_single_story_participates_in() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    write_qdev_toml(root);
    // E1S1 sits in two cycles: E1S1 -> E1S2 -> E1S1 and E1S1 -> E1S3 -> E1S1.
    write_story(root, "E1S1", "  depends_on: [\"E1S2\", \"E1S3\"]\n");
    write_story(root, "E1S2", "  depends_on: [\"E1S1\"]\n");
    write_story(root, "E1S3", "  depends_on: [\"E1S1\"]\n");

    let store = ensure_cache(root, &storage()).unwrap();

    let cycle_findings: Vec<FindingRecord> = findings_for(&store, "docs/specs/stories/E1S1.md")
        .into_iter()
        .filter(|f| f.code == "dependency_cycle")
        .collect();
    assert_eq!(
        cycle_findings.len(),
        2,
        "both cycles through E1S1 must survive, got {:?}",
        cycle_findings
    );
}

/// The cycle search walks the `depends_on` graph iteratively. A recursive walk would recurse
/// once per link and blow the stack on a long chain, aborting the hydration sweep.
#[test]
fn test_find_dependency_cycle_handles_a_very_deep_chain() {
    const DEPTH: usize = 60_000;

    let mut edges: Vec<(String, String)> = (0..DEPTH)
        .map(|n| (format!("E1S{}", n), format!("E1S{}", n + 1)))
        .collect();
    assert!(
        find_dependency_cycle(&edges).is_none(),
        "a {}-link chain is acyclic",
        DEPTH
    );

    // Close it, and the same deep walk must find the cycle rather than overflow.
    edges.push((format!("E1S{}", DEPTH), "E1S0".to_string()));
    let cycle = find_dependency_cycle(&edges).expect("closing the chain creates a cycle");
    assert_eq!(cycle.first(), cycle.last());
    assert_eq!(cycle.len(), DEPTH + 2);
}
