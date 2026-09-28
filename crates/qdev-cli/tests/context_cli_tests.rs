//! `qdev context` CLI tests (Story 4.1): the I/O matrix through the real binary — envelope
//! shape against the printed schema, output modes, default budgets, and the error cases.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use qdev_core::store::{SqliteStore, Store};
use serde_json::Value;
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

    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push_str(
        "\n[[modules]]\nid = \"bridge\"\npaths = [\"crates/bridge/**\"]\n\n[[modules]]\nid = \"foundation\"\npaths = [\"crates/foundation/**\"]\n\n[[gates]]\nid = \"c-abi-round-trip\"\ncommand = \"cargo test\"\non_transition = [\"review\"]\n",
    );
    fs::write(toml_path, toml).unwrap();
    write_file(root, "crates/bridge/lib.rs", "// bridge");
    write_file(root, "crates/foundation/lib.rs", "// foundation");
}

fn write_file(root: &Path, rel_path: &str, content: &str) {
    let path = root.join(rel_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// The reference fixture: epic E12 + story E12S4 + a requirement and an ADR it links to.
fn write_story_fixture(root: &Path) {
    write_file(
        root,
        "docs/specs/epics/E12.md",
        r#"---
id: E12
title: "Bridge Layer"
status: active
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Goal
Ship the bridge layer.
"#,
    );

    write_file(
        root,
        "docs/specs/stories/E12S4.md",
        r#"---
id: E12S4
title: "CoreResponse Buffer Layout"
status: ready
version: 3
owners: ["simon", "team:core-platform"]
epic_id: E12
appetite: small
target_modules: ["bridge", "foundation"]
constraints:
  - id: NG-1
    kind: no_go
    text: "Do not implement Swift decoding"
relations:
  traces_to: ["FR-102"]
  governed_by: ["AD-43"]
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- Buffers round-trip.
"#,
    );

    write_file(
        root,
        "docs/specs/requirements/FR-102.md",
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
    );

    write_file(
        root,
        "docs/specs/adrs/AD-43.md",
        r#"---
id: AD-43
title: "Bounded Response Buffers"
status: accepted
version: 1
decision: "Bound the ring at 16 entries"
prevents: "unbounded queue growth"
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---
"#,
    );
}

/// Hydrates the cache from the fixture files. `context` is read-only by contract — it does
/// not trigger the boot sweep — so a fixture written by hand needs an explicit sync first.
fn sync(root: &Path) {
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sync", "--rebuild"])
        .assert()
        .success();
}

fn validate_against_schema(schema: &Value, instance: &Value) {
    let validator = jsonschema::validator_for(schema).expect("schema must compile");
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "instance failed schema validation: {errors:?}\ninstance: {instance}"
    );
}

// ---------------------------------------------------------------------------
// Develop happy path through the binary
// ---------------------------------------------------------------------------

#[test]
fn test_develop_json_envelope_shape_and_order() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context",
            "E12S4",
            "--phase",
            "develop",
            "--budget",
            "1200",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();

    // Envelope + payload top level.
    assert_eq!(val["schema_version"], "1");
    assert_eq!(val["id"], "E12S4");
    assert_eq!(val["phase"], "develop");
    assert_eq!(val["budget"], 1200);
    assert!(val["total_tokens"].as_u64().unwrap() <= 1200);
    assert!(val.get("stats").is_none(), "no --stats: no stats object");

    // Sections in priority order.
    let names: Vec<&str> = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
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
    assert!(val["truncated"].as_array().unwrap().is_empty());

    // The spec body and the inherited constraint are both present.
    let spec = &val["sections"][0]["content"];
    assert!(spec.as_str().unwrap().contains("round-trip"));
    let constraints = val["sections"][1]["content"].as_str().unwrap();
    assert!(constraints.contains("E12S4/NG-1"));

    // Round trip against the printed schema.
    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "context", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();
    validate_against_schema(&schema, &val);
}

#[test]
fn test_stats_object_only_with_flag_and_under_budget_for_reference_fixture() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context",
            "E12S4",
            "--phase",
            "develop",
            "--budget",
            "1200",
            "--stats",
            "--json",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();

    let stats = &val["stats"];
    assert!(stats.is_object());
    assert_eq!(stats["budget"], 1200);
    assert_eq!(stats["total_tokens"], val["total_tokens"]);
    assert_eq!(stats["over_budget"], false);
    assert_eq!(
        stats["sections"].as_object().unwrap().len(),
        val["sections"].as_array().unwrap().len()
    );
}

#[test]
fn test_default_budgets_per_phase() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    for (phase, budget) in [("specify", 800), ("develop", 1200), ("review", 2500)] {
        let assert = Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args(["context", "E12S4", "--phase", phase, "--json"])
            .assert()
            .success();
        let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        assert_eq!(val["budget"], budget, "phase {phase} default budget");
    }
}

// ---------------------------------------------------------------------------
// Output modes
// ---------------------------------------------------------------------------

#[test]
fn test_text_and_markdown_renderings_are_deterministic() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let text1 = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["context", "E12S4", "--phase", "develop", "--stats"])
        .assert()
        .success()
        .code(0);
    let text1 = std::str::from_utf8(&text1.get_output().stdout).unwrap();
    assert!(text1.contains("== story_spec =="));
    assert!(text1.contains("Stats (estimated tokens)"));

    let markdown = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context",
            "E12S4",
            "--phase",
            "develop",
            "--format",
            "md",
        ])
        .assert()
        .success()
        .code(0);
    let markdown = std::str::from_utf8(&markdown.get_output().stdout).unwrap();
    assert!(markdown.contains("# Context: E12S4 (phase: develop)"));
    assert!(markdown.contains("## story_spec"));

    // Byte-identical across repeated runs.
    let text2 = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["context", "E12S4", "--phase", "develop", "--stats"])
        .assert()
        .success()
        .code(0);
    assert_eq!(
        text1,
        std::str::from_utf8(&text2.get_output().stdout).unwrap(),
        "text renders must be byte-identical"
    );
}

#[test]
fn test_json_and_format_md_together_is_usage_error_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context",
            "E12S4",
            "--phase",
            "develop",
            "--json",
            "--format",
            "md",
        ])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_unknown_format_is_usage_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context", "E12S4", "--phase", "develop", "--format", "html",
        ])
        .assert()
        .failure()
        .code(2);
    let stderr = std::str::from_utf8(&assert.get_output().stderr).unwrap();
    assert!(stderr.contains("md"), "{stderr}");
}

// ---------------------------------------------------------------------------
// I/O matrix error cases
// ---------------------------------------------------------------------------

#[test]
fn test_non_story_target_is_usage_error_naming_kind() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["context", "E12", "--phase", "develop", "--json"])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
    let message = val["error"]["message"].as_str().unwrap();
    assert!(message.contains("epic"), "{message}");
    assert!(message.contains("story"), "{message}");
}

#[test]
fn test_unknown_id_is_logical_error_exit_1() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["context", "E99S9", "--phase", "develop", "--json"])
        .assert()
        .failure()
        .code(1);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "entity_not_found");
}

#[test]
fn test_zero_budget_is_usage_error_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context", "E12S4", "--phase", "develop", "--budget", "0", "--json",
        ])
        .assert()
        .failure()
        .code(2);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(val["error"]["code"], "usage_error");
}

#[test]
fn test_missing_phase_is_clap_usage_error_exit_2() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["context", "E12S4"])
        .assert()
        .failure()
        .code(2);
    assert!(!assert.get_output().stdout.is_empty()
        || !assert.get_output().stderr.is_empty());
}

/// The `qdev context --json` envelope validates against the printed schema for every phase.
#[test]
fn test_round_trip_context_payload_against_context_output() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let schema_assert = Command::cargo_bin("qdev")
        .unwrap()
        .args(["schema", "payload", "context", "--json"])
        .assert()
        .success();
    let schema: Value = serde_json::from_slice(&schema_assert.get_output().stdout).unwrap();

    for phase in ["specify", "develop", "review"] {
        let assert = Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args([
                "context", "E12S4", "--phase", phase, "--stats", "--budget", "50", "--json",
            ])
            .assert()
            .success();
        let instance: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        validate_against_schema(&schema, &instance);
    }
}

/// The review phase's diff summary is present but empty-with-a-note when no git baseline
/// resolves — exit 0, never an error.
#[test]
fn test_review_diff_empty_without_git_baseline() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);
    // No `git init` in this workspace.

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["context", "E12S4", "--phase", "review", "--json"])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let diff = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "diff")
        .expect("the diff section must be present for review")
        ["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        diff.starts_with("(empty: "),
        "diff must carry a reason note, got: {diff}"
    );
}

/// The truncation record: a tight budget drops lowest-priority sections first, in order.
#[test]
fn test_truncation_lists_dropped_sections_in_order() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);

    // A spec body that alone exceeds the 20-token budget: the first-section exemption keeps
    // it whole, everything else drops, and --stats surfaces the overrun.
    write_file(
        root,
        "docs/specs/stories/E12S4.md",
        &format!(
            "---\nid: E12S4\ntitle: \"CoreResponse Buffer Layout\"\nstatus: ready\nversion: 3\nepic_id: E12\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n\n## Acceptance Criteria\n- {}\n",
            "x".repeat(300)
        ),
    );
    sync(root);

    let assert = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context", "E12S4", "--phase", "develop", "--budget", "20", "--stats", "--json",
        ])
        .assert()
        .success()
        .code(0);
    let val: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();

    let truncated_names: Vec<&str> = val["truncated"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        truncated_names,
        vec![
            "constraints", "modules", "adr_excerpts", "requirements", "scratchpad", "gates",
            "hygiene"
        ]
    );
    let section_names: Vec<&str> = val["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(section_names, vec!["story_spec"]);
    // The spec body (5 lines ≈ 40+ tokens) alone exceeds 20: the exemption keeps it whole and
    // --stats surfaces the overrun.
    assert_eq!(val["stats"]["over_budget"], true);
}

/// `qdev context` is read-only: it must not mutate the cache — a repeat run over the same
/// (unsynced) workspace reports the same thing, and the `sync_meta` table's `last_synced_at`
/// timestamp is untouched across runs.
#[test]
fn test_context_is_read_only_across_repeated_runs() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let cache_db_path = root.join(".qdev/cache/cache.sqlite");
    let store = SqliteStore::open(&cache_db_path).unwrap();
    let stamp_before = store.get_last_synced_at().unwrap();
    assert!(stamp_before.is_some(), "workspace must be synced before test");

    let run = || {
        let output = Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args([
                "context",
                "E12S4",
                "--phase",
                "develop",
                "--stats",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        output.stdout
    };

    assert_eq!(run(), run(), "repeated runs must be byte-identical");

    let stamp_after = store.get_last_synced_at().unwrap();
    assert_eq!(
        stamp_before, stamp_after,
        "sync_meta table's last_synced_at timestamp must be untouched across runs"
    );
}

/// Assert `--format md --stats` table rendering: stdout contains "## stats" and "| section | tokens |".
#[test]
fn test_markdown_stats_table_rendering() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);
    write_story_fixture(root);
    sync(root);

    let markdown = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "context",
            "E12S4",
            "--phase",
            "develop",
            "--stats",
            "--format",
            "md",
        ])
        .assert()
        .success()
        .code(0);
    let stdout = std::str::from_utf8(&markdown.get_output().stdout).unwrap();
    assert!(
        stdout.contains("## stats"),
        "stdout must contain '## stats': {stdout}"
    );
    assert!(
        stdout.contains("| section | tokens |"),
        "stdout must contain '| section | tokens |': {stdout}"
    );
}
