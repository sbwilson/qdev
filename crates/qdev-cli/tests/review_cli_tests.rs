use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn setup(root: &Path) {
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "Review",
            "--developer",
            "simon",
            "--team",
            "core",
        ])
        .assert()
        .success();
}

fn sync(root: &Path) {
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .arg("sync")
        .assert()
        .success();
}

#[test]
fn review_epic_uses_normal_json_envelope_and_refuses_missing_evidence() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    fs::write(root.join("docs/specs/stories/E12S1.md"), "---\nid: E12S1\ntitle: Story\nstatus: done\nversion: 1\nepic_id: E12\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n").unwrap();
    sync(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["review", "epic", "E12", "--json"])
        .assert()
        .failure()
        .code(1)
        .stdout(predicate::str::contains("missing_gate_evidence"));
}

#[test]
fn review_sprint_writes_release_reports() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::write(root.join("docs/state/releases/0.1.0.md"), "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n").unwrap();
    sync(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint",
            "open",
            "1",
            "--title",
            "One",
            "--release",
            "0.1.0",
        ])
        .assert()
        .success();
    let output = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["review", "sprint", "1", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(payload["sprint_id"], 1);
    assert!(root.join("docs/state/releases/0.1.0/rtm.md").is_file());
    assert!(root
        .join("docs/state/releases/0.1.0/anomalies.md")
        .is_file());
}

#[test]
fn alternate_close_requires_reason_and_records_decision_without_baseline() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "open", "1", "--title", "One"])
        .assert()
        .success();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "close", "1", "--status", "paused"])
        .assert()
        .failure()
        .code(3);
    let output = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint",
            "close",
            "1",
            "--status",
            "paused",
            "--reason",
            "Awaiting evidence",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(payload["status"], "paused");
    assert!(payload["decision_id"].as_str().unwrap().starts_with("DEC-"));
    assert!(
        !fs::read_to_string(root.join("docs/state/sprints/sprint-1.md"))
            .unwrap()
            .contains("baseline_snapshot")
    );
}

#[test]
fn sprint_review_propagates_gate_failures_and_infrastructure_without_reports() {
    for (command, code) in [("false", 1), ("definitely-missing-qdev-command", 4)] {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        setup(root);
        fs::create_dir_all(root.join("docs/state/releases")).unwrap();
        fs::write(root.join("docs/state/releases/0.1.0.md"), "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n").unwrap();
        let mut config = fs::read_to_string(root.join("qdev.toml")).unwrap();
        config.push_str(&format!("\n[[gates]]\nid = \"close-gate\"\ncommand = \"{command}\"\non_transition = [\"sprint_close\"]\n"));
        fs::write(root.join("qdev.toml"), config).unwrap();
        sync(root);
        Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args([
                "sprint",
                "open",
                "1",
                "--title",
                "One",
                "--release",
                "0.1.0",
            ])
            .assert()
            .success();
        Command::cargo_bin("qdev")
            .unwrap()
            .current_dir(root)
            .args(["review", "sprint", "1"])
            .assert()
            .failure()
            .code(code);
        assert!(!root.join("docs/state/releases/0.1.0/rtm.md").exists());
    }
}

fn story_file(id: &str, status: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: Story\nstatus: {status}\nversion: 1\nepic_id: E12\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n"
    )
}

fn dw_file(id: &str, risk: &str, rationale: Option<&str>) -> String {
    let rationale = match rationale {
        Some(r) => format!("\nrationale: {r}\n"),
        None => String::new(),
    };
    format!(
        "---\nid: {id}\ntitle: Debt\nstatus: open\nsafety_risk: {risk}\n{rationale}version: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n"
    )
}

#[test]
fn review_epic_success_with_bound_gate() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    fs::write(
        root.join("docs/specs/stories/E12S1.md"),
        story_file("E12S1", "done"),
    )
    .unwrap();
    let mut config = fs::read_to_string(root.join("qdev.toml")).unwrap();
    config.push_str(
        "\n[[gates]]\nid = \"review-gate\"\ncommand = \"true\"\non_transition = [\"review\"]\n",
    );
    fs::write(root.join("qdev.toml"), &config).unwrap();
    sync(root);
    // A passing run attributed to the done story satisfies the bound-gate requirement.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["gate", "run", "review-gate", "--story", "E12S1"])
        .assert()
        .success();
    let output = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["review", "epic", "E12", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(payload["stories"][0]["covered_gate_ids"][0], "review-gate");
}

#[test]
fn review_sprint_reports_content_and_grouped_anomalies() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::write(
        root.join("docs/state/releases/0.1.0.md"),
        "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n",
    )
    .unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S1.md"),
        story_file("E12S1", "done"),
    )
    .unwrap();
    fs::write(
        root.join("docs/specs/stories/E12S2.md"),
        story_file("E12S2", "in-progress"),
    )
    .unwrap();
    sync(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint",
            "open",
            "1",
            "--title",
            "One",
            "--release",
            "0.1.0",
        ])
        .assert()
        .success();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "assign", "--sprint", "1", "E12S1", "E12S2"])
        .assert()
        .success();
    fs::create_dir_all(root.join("docs/state/dw")).unwrap();
    fs::write(
        root.join("docs/state/dw/DW-1.md"),
        dw_file("DW-1", "unacceptable", Some("Accepted risk")),
    )
    .unwrap();
    fs::write(
        root.join("docs/state/dw/DW-2.md"),
        dw_file("DW-2", "acceptable_with_mitigation", None),
    )
    .unwrap();
    sync(root);
    let output = Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["review", "sprint", "1", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let payload: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(payload["sprint_id"], 1);
    assert_eq!(payload["release"], "0.1.0");
    // The payload carries the report-ordered open debt, not just two file paths.
    assert_eq!(payload["open_deferred_work"][0]["id"], "DW-1");
    assert_eq!(payload["open_deferred_work"][1]["id"], "DW-2");

    let rtm = fs::read_to_string(root.join("docs/state/releases/0.1.0/rtm.md")).unwrap();
    assert!(
        rtm.contains("| Story | Status | Requirements | Evidence |"),
        "{rtm}"
    );
    assert!(rtm.contains("| E12S1 | done |"), "{rtm}");
    assert!(rtm.contains("| E12S2 | in-progress |"), "{rtm}");

    let anomalies =
        fs::read_to_string(root.join("docs/state/releases/0.1.0/anomalies.md")).unwrap();
    assert!(anomalies.contains("## unacceptable"), "{anomalies}");
    assert!(anomalies.contains("DW-1"), "{anomalies}");
    assert!(
        anomalies.contains("## acceptable_with_mitigation"),
        "{anomalies}"
    );
    assert!(anomalies.contains("DW-2"), "{anomalies}");
}

#[test]
fn alternate_close_persists_reason_and_decision_without_release_baseline() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    fs::create_dir_all(root.join("docs/state/releases")).unwrap();
    fs::write(
        root.join("docs/state/releases/0.1.0.md"),
        "---\nid: 0.1.0\ntitle: Release\nstatus: active\nversion: 1\ncreated_by:\n  type: human\n  id: simon\nupdated_by:\n  type: human\n  id: simon\n---\n",
    )
    .unwrap();
    sync(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint",
            "open",
            "1",
            "--title",
            "One",
            "--release",
            "0.1.0",
        ])
        .assert()
        .success();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint",
            "close",
            "1",
            "--status",
            "paused",
            "--reason",
            "Awaiting vendor evidence",
            "--json",
        ])
        .assert()
        .success();
    // The no-baseline guard is observable only with a linked release: the release file
    // must be untouched by a paused close.
    let release = fs::read_to_string(root.join("docs/state/releases/0.1.0.md")).unwrap();
    assert!(
        !release.contains("baseline_snapshot"),
        "paused close must not snapshot:\n{release}"
    );
    // The reason is durably recorded on the sprint and in an attributed decision.
    let sprint_file = fs::read_to_string(root.join("docs/state/sprints/sprint-1.md")).unwrap();
    assert!(
        sprint_file.contains("close_reason: Awaiting vendor evidence"),
        "{sprint_file}"
    );
    let mut ruling_found = false;
    if let Ok(mut entries) = fs::read_dir(root.join("docs/state/decisions")) {
        while let Some(entry) = entries.next() {
            let content = fs::read_to_string(entry.unwrap().path()).unwrap_or_default();
            if content.contains("Awaiting vendor evidence") {
                ruling_found = true;
            }
        }
    }
    assert!(ruling_found, "decision record must contain the ruling");
}

#[test]
fn close_invalid_status_and_reclosing_are_cleanly_refused() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "open", "1", "--title", "One"])
        .assert()
        .success();
    // Invalid status: usage error with a machine-readable code.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "close", "1", "--status", "bogus", "--json"])
        .assert()
        .failure()
        .code(2)
        .stdout(predicate::str::contains("invalid_close_status"));
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "close", "1"])
        .assert()
        .success();
    // Re-closing a terminal sprint: policy refusal, exit 3.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "close", "1", "--json"])
        .assert()
        .failure()
        .code(3)
        .stdout(predicate::str::contains("sprint_already_closed"));
}

#[test]
fn paused_close_blocked_by_unacceptable_dw_until_resolved() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup(root);
    fs::create_dir_all(root.join("docs/state/dw")).unwrap();
    fs::write(
        root.join("docs/state/dw/DW-1.md"),
        dw_file("DW-1", "unacceptable", None),
    )
    .unwrap();
    sync(root);
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["sprint", "open", "1", "--title", "One"])
        .assert()
        .success();
    // The safety refusal applies to paused closes too.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint", "close", "1", "--status", "paused", "--reason", "Holding", "--json",
        ])
        .assert()
        .failure()
        .code(3)
        .stdout(predicate::str::contains("unacceptable_deferred_work"));
    // Resolving the DW lifts the block — a `done` DW with an empty rationale must not
    // refuse every close forever.
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args(["dw", "close", "DW-1"])
        .assert()
        .success();
    Command::cargo_bin("qdev")
        .unwrap()
        .current_dir(root)
        .args([
            "sprint", "close", "1", "--status", "paused", "--reason", "Holding",
        ])
        .assert()
        .success();
}
