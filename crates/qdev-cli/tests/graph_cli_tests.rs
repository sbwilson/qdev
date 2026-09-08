//! `qdev graph --dot` CLI tests, covering an epic-filtered graph, an unfiltered graph, and an
//! empty workspace.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
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
fn test_graph_without_dot_flag_is_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["graph", "--json"])
        .assert()
        .code(3);
}

#[test]
fn test_graph_bare_no_flags_is_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root).args(["graph"]).assert().code(3);
}

#[test]
fn test_graph_dot_with_json_is_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["graph", "--dot", "--json"])
        .assert()
        .code(3);
}
