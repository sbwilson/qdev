//! `qdev graph --dot` CLI tests, covering an epic-filtered graph, an unfiltered graph, and an
//! empty workspace.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "TestProject",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();
}

fn write_story(root: &Path, id: &str, epic: &str, title: &str, status: &str, relations_yaml: &str) {
    let dir = root.join("docs/specs/stories");
    fs::create_dir_all(&dir).unwrap();
    let relations_block = if relations_yaml.is_empty() {
        String::new()
    } else {
        format!("relations:\n{relations_yaml}")
    };
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "{title}"
status: {status}
version: 1
epic_id: {epic}
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
{relations_block}---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

fn write_epic(root: &Path, id: &str) {
    let dir = root.join("docs/specs/epics");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Epic {id}"
status: planning
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Summary
- Summary.
"#
        ),
    )
    .unwrap();
}

fn write_sprint(root: &Path, num: i64, status: &str, stories: &[&str]) {
    let dir = root.join("docs/state/sprints");
    fs::create_dir_all(&dir).unwrap();
    let mut assignments_yaml = String::new();
    for story in stories {
        assignments_yaml.push_str(&format!(
            "  - story: {story}\n    assigned_at: 2026-09-10\n"
        ));
    }
    let assignments_block = if assignments_yaml.is_empty() {
        String::new()
    } else {
        format!("assignments:\n{assignments_yaml}")
    };
    fs::write(
        dir.join(format!("sprint-{num}.md")),
        format!(
            r#"---
id: sprint-{num}
title: "Sprint {num}"
status: {status}
version: 1
release: 0.1.0
started_at: 2026-09-10
{assignments_block}created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

# Sprint {num}
"#
        ),
    )
    .unwrap();
}

fn write_lease(root: &Path, story_id: &str, holder: &str) {
    let dir = root.join(".qdev/leases");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("{story_id}.json")),
        serde_json::json!({
            "story_id": story_id,
            "holder": holder,
            "author_type": "human",
            "worktree_path": root.to_str().unwrap(),
            "branch": "main",
            "started_at": "2026-10-01T00:00:00Z",
            "session_token": "tok"
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn test_graph_dot_unfiltered_includes_all_stories_and_edges() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_epic(root, "E2");
    write_story(root, "E1S1", "E1", "Story One", "done", "");
    write_story(
        root,
        "E1S2",
        "E1",
        "Story Two",
        "in-progress",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_story(root, "E2S1", "E2", "Other Epic Story", "draft", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.starts_with("digraph qdev {\n"));
    assert!(output.trim_end().ends_with('}'));
    assert!(output.contains("\"E1S1\""));
    assert!(output.contains("\"E1S2\""));
    assert!(output.contains("\"E2S1\""));
    assert!(output.contains("\"E1S2\" -> \"E1S1\" [label=\"depends_on\"];"));
    assert!(output.contains("fillcolor=\"green\""));
    assert!(output.contains("fillcolor=\"yellow\""));
    assert!(output.contains("fillcolor=\"lightgray\""));
}

#[test]
fn test_graph_dot_epic_filter_excludes_other_epics_and_their_edges() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_epic(root, "E2");
    write_story(root, "E1S1", "E1", "Story One", "done", "");
    write_story(
        root,
        "E1S2",
        "E1",
        "Story Two",
        "in-progress",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_story(
        root,
        "E2S1",
        "E2",
        "Other Epic Story",
        "draft",
        "  depends_on: [\"E1S1\"]\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot", "--epic", "E1"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.contains("\"E1S1\""));
    assert!(output.contains("\"E1S2\""));
    assert!(!output.contains("E2S1"));
    assert!(output.contains("\"E1S2\" -> \"E1S1\" [label=\"depends_on\"];"));
}

#[test]
fn test_graph_dot_epic_filter_excludes_edge_to_story_outside_filter() {
    // Reverse direction of the above: an *included* E1 story depends on an *excluded* E2
    // story. The edge must be dropped since both endpoints must be in the filtered node set.
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_epic(root, "E2");
    write_story(root, "E2S1", "E2", "Other Epic Story", "done", "");
    write_story(
        root,
        "E1S1",
        "E1",
        "Story One",
        "in-progress",
        "  depends_on: [\"E2S1\"]\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot", "--epic", "E1"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.contains("\"E1S1\""));
    assert!(!output.contains("E2S1"));
    assert!(!output.contains("depends_on"));
}

#[test]
fn test_graph_dot_status_colors_and_edge_relations() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_story(root, "E1S1", "E1", "Ready Story", "ready", "");
    write_story(root, "E1S2", "E1", "Review Story", "review", "");
    write_story(root, "E1S3", "E1", "Superseded Story", "superseded", "");
    write_story(root, "E1S4", "E1", "Abandoned Story", "abandoned", "");
    write_story(
        root,
        "E1S5",
        "E1",
        "Extends Story",
        "draft",
        "  extends: [\"E1S1\"]\n",
    );
    write_story(
        root,
        "E1S6",
        "E1",
        "Supersedes Story",
        "draft",
        "  supersedes: [\"E1S2\"]\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot", "--epic", "E1"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.contains("fillcolor=\"lightblue\"")); // ready
    assert!(output.contains("fillcolor=\"orange\"")); // review
    assert!(output.contains("fillcolor=\"gray45\"")); // superseded/abandoned
    assert!(output.contains("style=\"filled,dashed\"")); // superseded/abandoned border
    assert!(output.contains("\"E1S5\" -> \"E1S1\" [label=\"extends\"];"));
    assert!(output.contains("\"E1S6\" -> \"E1S2\" [label=\"supersedes\"];"));
}

#[test]
fn test_graph_dot_escapes_quotes_and_backslashes_in_labels() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    // A title containing a literal double quote and backslash; YAML-escaped in the frontmatter
    // (`\"` / `\\`) so the parsed title is: Story "Quoted" \ Name
    write_story(
        root,
        "E1S1",
        "E1",
        r#"Story \"Quoted\" \\ Name"#,
        "draft",
        "",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    // The DOT-escaped label must appear verbatim: '"' -> '\"', '\' -> '\\'.
    assert!(output.contains(r#"E1S1\nStory \"Quoted\" \\ Name"#));
    // And the node line must still parse as one well-formed quoted DOT attribute (no stray
    // unescaped '"' breaking out of the label early).
    let node_line = output
        .lines()
        .find(|l| l.contains("label=\"E1S1"))
        .expect("node line present");
    assert!(node_line.trim_end().ends_with("];"));
}

#[test]
fn test_graph_dot_empty_workspace_emits_valid_empty_digraph() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert_eq!(output.trim(), "digraph qdev {\n}");
}

#[test]
fn test_graph_json_empty_workspace_emits_valid_envelope() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--json"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(parsed["schema_version"], "1");
    assert_eq!(parsed["nodes"], serde_json::json!([]));
    assert_eq!(parsed["edges"], serde_json::json!([]));
    assert!(parsed.get("critical_path").is_none());
}

#[test]
fn test_graph_dot_sprint_filter() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_story(root, "E1S1", "E1", "Story One", "done", "");
    write_story(
        root,
        "E1S2",
        "E1",
        "Story Two",
        "in-progress",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_story(root, "E1S3", "E1", "Story Three", "ready", "");
    write_sprint(root, 5, "active", &["E1S1", "E1S2"]);
    write_sprint(root, 6, "planning", &["E1S3"]);

    // Test with sprint number "5"
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot", "--sprint", "5"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.contains("\"E1S1\""));
    assert!(output.contains("\"E1S2\""));
    assert!(!output.contains("E1S3"));
    assert!(output.contains("\"E1S2\" -> \"E1S1\" [label=\"depends_on\"];"));

    // Test with canonical "sprint-5"
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    let assert2 = cmd2
        .current_dir(root)
        .args(["graph", "--dot", "--sprint", "sprint-5"])
        .assert()
        .success()
        .code(0);
    let output2 = String::from_utf8(assert2.get_output().stdout.clone()).unwrap();
    assert!(output2.contains("\"E1S1\""));
    assert!(output2.contains("\"E1S2\""));
    assert!(!output2.contains("E1S3"));
}

#[test]
fn test_graph_json_sprint_filter_and_schema_validation() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_story(root, "E1S1", "E1", "Story One", "done", "");
    write_story(
        root,
        "E1S2",
        "E1",
        "Story Two",
        "in-progress",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_story(root, "E1S3", "E1", "Story Three", "ready", "");
    write_sprint(root, 5, "active", &["E1S1", "E1S2"]);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--json", "--sprint", "5"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(parsed["schema_version"], "1");
    let nodes = parsed["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 2);
    assert_eq!(nodes[0]["id"], "E1S1");
    assert_eq!(nodes[0]["epic_id"], "E1");
    assert_eq!(nodes[0]["status"], "done");
    assert_eq!(nodes[0]["blocked"], false);
    assert_eq!(nodes[0]["lease_holder"], serde_json::Value::Null);
    assert_eq!(nodes[0]["critical_path"], false);

    assert_eq!(nodes[1]["id"], "E1S2");
    assert_eq!(nodes[1]["status"], "in-progress");
    assert_eq!(nodes[1]["blocked"], false); // E1S1 is done, so E1S2 is unblocked

    let edges = parsed["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["source"], "E1S2");
    assert_eq!(edges[0]["target"], "E1S1");
    assert_eq!(edges[0]["relation"], "depends_on");
    assert_eq!(edges[0]["critical_path"], false);

    // Validate against payload-graph.json
    let schema_val: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/qdev-core/schemas/payload-graph.json"
    ))
    .unwrap();
    let validator = jsonschema::validator_for(&schema_val).unwrap();
    assert!(
        validator.is_valid(&parsed),
        "JSON output must validate against payload-graph.json"
    );
}

#[test]
fn test_graph_dot_blocked_and_leased_story() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    // E1S1 is in-progress (not done!)
    write_story(root, "E1S1", "E1", "Story One", "in-progress", "");
    // E1S2 depends on E1S1 which is NOT done -> E1S2 is BLOCKED
    write_story(
        root,
        "E1S2",
        "E1",
        "Story Two",
        "in-progress",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_lease(root, "E1S2", "simon");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    let node_line = output
        .lines()
        .find(|l| l.contains("\"E1S2\""))
        .expect("E1S2 line present");

    assert!(node_line.contains("peripheries=2"));
    assert!(node_line.contains(r#"label="E1S2\nStory Two\n[BLOCKED]\n[lease: simon]""#));
}

#[test]
fn test_graph_critical_path_dot_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    // Longest chain: E1S3 -> E1S2 -> E1S1 (length 3 nodes, 2 edges)
    // Shorter chain: E1S4 -> E1S5 (length 2 nodes, 1 edge)
    write_story(root, "E1S1", "E1", "Story One", "done", "");
    write_story(
        root,
        "E1S2",
        "E1",
        "Story Two",
        "done",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_story(
        root,
        "E1S3",
        "E1",
        "Story Three",
        "in-progress",
        "  depends_on: [\"E1S2\"]\n",
    );
    write_story(root, "E1S5", "E1", "Story Five", "done", "");
    write_story(
        root,
        "E1S4",
        "E1",
        "Story Four",
        "in-progress",
        "  depends_on: [\"E1S5\"]\n",
    );

    // 1. DOT output with --highlight-critical-path
    let mut cmd_dot = Command::cargo_bin("qdev").unwrap();
    let assert_dot = cmd_dot
        .current_dir(root)
        .args(["graph", "--dot", "--highlight-critical-path"])
        .assert()
        .success()
        .code(0);
    let dot_output = String::from_utf8(assert_dot.get_output().stdout.clone()).unwrap();

    // Critical path nodes have color="red", penwidth=2.0
    for node_id in ["E1S1", "E1S2", "E1S3"] {
        let line = dot_output
            .lines()
            .find(|l| l.contains(&format!("\"{node_id}\" [")))
            .unwrap_or_else(|| panic!("node {node_id} line present"));
        assert!(line.contains("color=\"red\""));
        assert!(line.contains("penwidth=2.0"));
    }

    // Non-critical path nodes do NOT have red border
    for node_id in ["E1S4", "E1S5"] {
        let line = dot_output
            .lines()
            .find(|l| l.contains(&format!("\"{node_id}\" [")))
            .unwrap_or_else(|| panic!("node {node_id} line present"));
        assert!(!line.contains("color=\"red\""));
        assert!(!line.contains("penwidth=2.0"));
    }

    // Critical path edges have color="red", penwidth=2.0
    let edge_3_2 = dot_output
        .lines()
        .find(|l| l.contains("\"E1S3\" -> \"E1S2\""))
        .expect("E1S3 -> E1S2 edge");
    assert!(edge_3_2.contains("color=\"red\""));
    assert!(edge_3_2.contains("penwidth=2.0"));

    let edge_2_1 = dot_output
        .lines()
        .find(|l| l.contains("\"E1S2\" -> \"E1S1\""))
        .expect("E1S2 -> E1S1 edge");
    assert!(edge_2_1.contains("color=\"red\""));
    assert!(edge_2_1.contains("penwidth=2.0"));

    // Non-critical path edge does not have red penwidth
    let edge_4_5 = dot_output
        .lines()
        .find(|l| l.contains("\"E1S4\" -> \"E1S5\""))
        .expect("E1S4 -> E1S5 edge");
    assert!(!edge_4_5.contains("color=\"red\""));
    assert!(!edge_4_5.contains("penwidth=2.0"));

    // 2. JSON output with --highlight-critical-path
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["graph", "--json", "--highlight-critical-path"])
        .assert()
        .success()
        .code(0);
    let json_output = String::from_utf8(assert_json.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json_output).unwrap();

    assert_eq!(
        parsed["critical_path"],
        serde_json::json!(["E1S3", "E1S2", "E1S1"])
    );

    let nodes = parsed["nodes"].as_array().unwrap();
    for n in nodes {
        let id = n["id"].as_str().unwrap();
        let expected_cp = id == "E1S1" || id == "E1S2" || id == "E1S3";
        assert_eq!(
            n["critical_path"].as_bool().unwrap(),
            expected_cp,
            "node {} critical_path check",
            id
        );
    }

    let edges = parsed["edges"].as_array().unwrap();
    for e in edges {
        let src = e["source"].as_str().unwrap();
        let tgt = e["target"].as_str().unwrap();
        let expected_cp = (src == "E1S3" && tgt == "E1S2") || (src == "E1S2" && tgt == "E1S1");
        assert_eq!(
            e["critical_path"].as_bool().unwrap(),
            expected_cp,
            "edge {} -> {} critical_path check",
            src,
            tgt
        );
    }

    // Schema validation
    let schema_val: serde_json::Value = serde_json::from_str(include_str!(
        "../../../crates/qdev-core/schemas/payload-graph.json"
    ))
    .unwrap();
    let validator = jsonschema::validator_for(&schema_val).unwrap();
    assert!(validator.is_valid(&parsed));
}

#[test]
fn test_graph_combined_epic_and_sprint_filters() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_epic(root, "E1");
    write_epic(root, "E2");
    write_story(root, "E1S1", "E1", "E1 S1", "done", "");
    write_story(root, "E1S2", "E1", "E1 S2", "ready", "");
    write_story(root, "E2S1", "E2", "E2 S1", "ready", "");
    // Sprint 5 has E1S1 and E2S1
    write_sprint(root, 5, "active", &["E1S1", "E2S1"]);

    // Intersect --epic E1 --sprint 5: should only match E1S1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot", "--epic", "E1", "--sprint", "5"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(output.contains("\"E1S1\""));
    assert!(!output.contains("E1S2"));
    assert!(!output.contains("E2S1"));
}

#[test]
fn test_graph_invalid_sprint_format() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["graph", "--dot", "--sprint", "abc"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("usage_error"))
        .stderr(predicate::str::contains("Invalid sprint identifier 'abc'"));
}

#[test]
fn test_graph_empty_filter_values_are_usage_errors() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Empty --epic
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["graph", "--dot", "--epic", ""])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("usage_error"))
        .stderr(predicate::str::contains(
            "'--epic' was given an empty value",
        ));

    // Empty --sprint
    let mut cmd2 = Command::cargo_bin("qdev").unwrap();
    cmd2.current_dir(root)
        .args(["graph", "--dot", "--sprint", ""])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("usage_error"))
        .stderr(predicate::str::contains(
            "'--sprint' was given an empty value",
        ));
}

#[test]
fn test_schema_payload_graph() {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .args(["schema", "payload", "graph"])
        .assert()
        .success()
        .code(0);
    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let schema_json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(schema_json["title"], "Graph Payload Schema");
    assert!(schema_json["properties"]["nodes"].is_object());
    assert!(schema_json["properties"]["edges"].is_object());

    // In JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .args(["schema", "payload", "graph", "--json"])
        .assert()
        .success()
        .code(0);
    let output_json = String::from_utf8(assert_json.get_output().stdout.clone()).unwrap();
    let env_json: serde_json::Value = serde_json::from_str(&output_json).unwrap();
    assert_eq!(env_json["schema_version"], "1");
    assert_eq!(env_json["title"], "Graph Payload Schema");
}

#[test]
fn test_graph_bare_no_flags_is_a_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["graph"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("usage_error"))
        .stderr(predicate::str::contains(
            "qdev graph requires either '--dot' or '--json' output format",
        ));
}

#[test]
fn test_graph_dot_with_json_is_a_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["graph", "--dot", "--json"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("usage_error"));
}

#[test]
fn test_critical_path_equal_length_chain_tie_breaking() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E1S1", "E1", "Story 1", "ready", "");
    write_story(
        root,
        "E1S2",
        "E1",
        "Story 2",
        "ready",
        "  depends_on: [\"E1S1\"]\n",
    );
    write_story(
        root,
        "E1S3",
        "E1",
        "Story 3",
        "ready",
        "  depends_on: [\"E1S2\"]\n",
    );

    write_story(root, "E1S4", "E1", "Story 4", "ready", "");
    write_story(
        root,
        "E1S5",
        "E1",
        "Story 5",
        "ready",
        "  depends_on: [\"E1S4\"]\n",
    );
    write_story(
        root,
        "E1S6",
        "E1",
        "Story 6",
        "ready",
        "  depends_on: [\"E1S5\"]\n",
    );

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--json", "--highlight-critical-path"])
        .assert()
        .success()
        .code(0);

    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let env_json: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
        env_json["critical_path"],
        serde_json::json!(["E1S3", "E1S2", "E1S1"])
    );

    let nodes = env_json["nodes"].as_array().unwrap();
    for node in nodes {
        let id = node["id"].as_str().unwrap();
        let cp = node["critical_path"].as_bool().unwrap();
        if id == "E1S1" || id == "E1S2" || id == "E1S3" {
            assert!(cp, "node {} must be on critical path", id);
        } else {
            assert!(!cp, "node {} must not be on critical path", id);
        }
    }
}

#[test]
fn test_zero_edge_graph_with_highlight_critical_path() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E1S1", "E1", "Story 1", "ready", "");
    write_story(root, "E1S2", "E1", "Story 2", "ready", "");

    // In JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["graph", "--json", "--highlight-critical-path"])
        .assert()
        .success()
        .code(0);

    let output_json = String::from_utf8(assert_json.get_output().stdout.clone()).unwrap();
    let env_json: serde_json::Value = serde_json::from_str(&output_json).unwrap();
    assert_eq!(env_json["critical_path"], serde_json::json!([]));
    for node in env_json["nodes"].as_array().unwrap() {
        assert_eq!(node["critical_path"], false);
    }

    // In DOT mode
    let mut cmd_dot = Command::cargo_bin("qdev").unwrap();
    let assert_dot = cmd_dot
        .current_dir(root)
        .args(["graph", "--dot", "--highlight-critical-path"])
        .assert()
        .success()
        .code(0);

    let dot = String::from_utf8(assert_dot.get_output().stdout.clone()).unwrap();
    assert!(
        !dot.contains("color=\"red\""),
        "zero-edge graph must not highlight any nodes red"
    );
}

#[test]
fn test_graph_json_with_active_lease() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E1S1", "E1", "Story 1", "ready", "");
    write_story(root, "E1S2", "E1", "Story 2", "ready", "");

    // Claim E1S1
    let mut claim_cmd = Command::cargo_bin("qdev").unwrap();
    claim_cmd
        .current_dir(root)
        .args(["claim", "E1S1"])
        .assert()
        .success();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--json"])
        .assert()
        .success()
        .code(0);

    let output = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let env_json: serde_json::Value = serde_json::from_str(&output).unwrap();
    let nodes = env_json["nodes"].as_array().unwrap();

    let s1 = nodes.iter().find(|n| n["id"] == "E1S1").unwrap();
    assert_eq!(s1["lease_holder"], "simon");

    let s2 = nodes.iter().find(|n| n["id"] == "E1S2").unwrap();
    assert!(s2["lease_holder"].is_null());
}

#[test]
fn test_graph_dot_escapes_newlines_in_title() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    write_story(root, "E1S1", "E1", "Line 1\\nLine 2\\rLine 3", "ready", "");

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["graph", "--dot"])
        .assert()
        .success()
        .code(0);

    let dot = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(dot.contains("Line 1\\nLine 2Line 3"));
    assert!(!dot.contains('\r'));
}
