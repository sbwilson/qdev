use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = StdCommand::new("git")
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

fn setup_workspace(root: &Path) {
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args([
            "init",
            "--non-interactive",
            "--name",
            "HygieneTestProject",
            "--developer",
            "simon",
            "--team",
            "core-platform",
        ])
        .assert()
        .success();

    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "simon@example.com"]);
    git(root, &["config", "user.name", "Simon"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    // Initial commit so HEAD exists
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "initial commit"]);
}

#[test]
fn test_cli_clean_source_file_text_and_json() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let clean_file = root.join("src/clean.rs");
    fs::create_dir_all(clean_file.parent().unwrap()).unwrap();
    fs::write(
        &clean_file,
        r#"// Legitimate single-line comment
fn main() {}
"#,
    )
    .unwrap();

    // Text mode
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "src/clean.rs"])
        .assert()
        .code(0);
    let output_str = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(output_str.contains("Hygiene check: 0 findings"));

    // JSON mode
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["hygiene", "check", "--json", "src/clean.rs"])
        .assert()
        .code(0);
    let json_val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(json_val["schema_version"], "1");
    assert_eq!(json_val["status"], "pass");
    assert_eq!(json_val["summary"], "Hygiene check: 0 findings");
    assert!(json_val["findings"].as_array().unwrap().is_empty());
}

#[test]
fn test_cli_memoir_banner_comment_finding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let banner_file = root.join("src/banner.rs");
    fs::create_dir_all(banner_file.parent().unwrap()).unwrap();
    fs::write(
        &banner_file,
        "// ⭐ **STORY 2.10 — FIRST import**\nfn run() {}\n",
    )
    .unwrap();

    // Text mode: exits 1, reports finding at line 1
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "src/banner.rs"])
        .assert()
        .code(1);
    let text = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(text.contains("src/banner.rs:1: [story_banner]"));
    assert!(text.contains("// ⭐ **STORY 2.10 — FIRST import**"));
    assert!(text.contains("Hygiene check: 1 finding"));

    // JSON mode: exits 1, structured findings
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["hygiene", "check", "--json", "src/banner.rs"])
        .assert()
        .code(1);
    let json_val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(json_val["schema_version"], "1");
    assert_eq!(json_val["status"], "fail");
    assert_eq!(json_val["summary"], "Hygiene check: 1 finding");
    let findings = json_val["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["file"], "src/banner.rs");
    assert_eq!(findings[0]["line"], 1);
    assert_eq!(findings[0]["rule_id"], "story_banner");
    assert_eq!(findings[0]["excerpt"], "// ⭐ **STORY 2.10 — FIRST import**");
}

#[test]
fn test_cli_review_round_finding() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let review_file = root.join("src/review.rs");
    fs::create_dir_all(review_file.parent().unwrap()).unwrap();
    fs::write(
        &review_file,
        "// Review round 2 findings: fixed bug\nfn fixed() {}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "src/review.rs"])
        .assert()
        .code(1);
    let text = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(text.contains("src/review.rs:1: [review_round]"));
    assert!(text.contains("// Review round 2 findings: fixed bug"));
}

#[test]
fn test_cli_comment_block_over_max_lines() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let long_file = root.join("src/long.rs");
    fs::create_dir_all(long_file.parent().unwrap()).unwrap();
    fs::write(
        &long_file,
        r#"// line 1
// line 2
// line 3
// line 4
// line 5
// line 6
// line 7
fn func() {}
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "src/long.rs"])
        .assert()
        .code(1);
    let text = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(text.contains("src/long.rs:1: [max_inline_comment_lines]"));
    assert!(text.contains("// line 1"));
}

#[test]
fn test_cli_forbid_patterns() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Add forbid_patterns to qdev.toml
    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push_str("\n[hygiene]\nforbid_patterns = [\"DISALLOWED_KEYWORD\"]\n");
    fs::write(&toml_path, toml).unwrap();

    let file_with_forbid = root.join("src/forbid.rs");
    fs::create_dir_all(file_with_forbid.parent().unwrap()).unwrap();
    fs::write(
        &file_with_forbid,
        "// Notice: DISALLOWED_KEYWORD used here\nfn test() {}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "src/forbid.rs"])
        .assert()
        .code(1);
    let text = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(text.contains("src/forbid.rs:1: [forbid_patterns]"));
}

#[test]
fn test_cli_valid_compact_citation_exemption() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let citation_file = root.join("src/citation.rs");
    fs::create_dir_all(citation_file.parent().unwrap()).unwrap();
    fs::write(
        &citation_file,
        "// [E12S10] Rust-driven mount (see AD-43)\nfn mount() {}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hygiene", "check", "src/citation.rs"])
        .assert()
        .code(0);
}

#[test]
fn test_cli_fix_flag_exits_2_and_does_not_modify_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let file_path = root.join("src/bad.rs");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    let original_content = "// ⭐ **STORY 2.10 — FIRST import**\nfn run() {}\n";
    fs::write(&file_path, original_content).unwrap();

    // In text mode: exit 2
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "--fix"])
        .assert()
        .code(2);
    let err_msg = String::from_utf8_lossy(&assert.get_output().stderr);
    assert!(err_msg.contains("deferred"));

    // Verify file content was NOT modified
    let content_after = fs::read_to_string(&file_path).unwrap();
    assert_eq!(content_after, original_content);

    // In JSON mode: exit 2 with error envelope
    let mut cmd_json = Command::cargo_bin("qdev").unwrap();
    let assert_json = cmd_json
        .current_dir(root)
        .args(["hygiene", "check", "--fix", "--json"])
        .assert()
        .code(2);
    let json_val: Value = serde_json::from_slice(&assert_json.get_output().stdout).unwrap();
    assert_eq!(json_val["schema_version"], "1");
    assert_eq!(json_val["error"]["code"], "usage_error");
    assert!(json_val["error"]["message"]
        .as_str()
        .unwrap()
        .contains("deferred"));
}

#[test]
fn test_cli_diff_filter() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Commit a clean file
    let clean_file = root.join("src/clean.rs");
    fs::create_dir_all(clean_file.parent().unwrap()).unwrap();
    fs::write(&clean_file, "// clean\nfn clean() {}\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-m", "add clean file"]);

    // Create a modified file with a banner comment (unstaged diff)
    let dirty_file = root.join("src/dirty.rs");
    fs::write(
        &dirty_file,
        "// ⭐ **STORY 2.10 — FIRST import**\nfn dirty() {}\n",
    )
    .unwrap();

    // Check with --diff detects dirty file
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hygiene", "check", "--diff"])
        .assert()
        .code(1);

    // Remove the dirty file
    fs::remove_file(&dirty_file).unwrap();

    // Clean diff now exits 0
    let mut cmd_clean = Command::cargo_bin("qdev").unwrap();
    cmd_clean
        .current_dir(root)
        .args(["hygiene", "check", "--diff"])
        .assert()
        .code(0);
}

#[test]
fn test_cli_hygiene_disabled() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Set [hygiene] enabled = false
    let toml_path = root.join("qdev.toml");
    let mut toml = fs::read_to_string(&toml_path).unwrap();
    toml.push_str("\n[hygiene]\nenabled = false\n");
    fs::write(&toml_path, toml).unwrap();

    // File with violations
    let file_path = root.join("src/violation.rs");
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(
        &file_path,
        "// ⭐ **STORY 2.10 — FIRST import**\nfn run() {}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["hygiene", "check"])
        .assert()
        .code(0);
}

#[test]
fn test_cli_schema_payload_hygiene() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["schema", "payload", "hygiene"])
        .assert()
        .code(0);
    let output = String::from_utf8_lossy(&assert.get_output().stdout);
    let schema: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(schema["title"], "Hygiene Payload Schema");
}

#[test]
fn test_cli_builtin_gate_hygiene() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Run clean gate
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    cmd.current_dir(root)
        .args(["gate", "run", "qdev-hygiene"])
        .assert()
        .code(0);

    // Introduce dirty file with story banner
    let dirty_file = root.join("src/dirty.rs");
    fs::create_dir_all(dirty_file.parent().unwrap()).unwrap();
    fs::write(
        &dirty_file,
        "// ⭐ **STORY 2.10 — FIRST import**\nfn dirty() {}\n",
    )
    .unwrap();

    // Run gate -> fails with exit 1
    let mut cmd_fail = Command::cargo_bin("qdev").unwrap();
    cmd_fail
        .current_dir(root)
        .args(["gate", "run", "qdev-hygiene"])
        .assert()
        .code(1);
}

#[test]
fn test_cli_bare_workspace_traversal_multi_language() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    // Create a Swift file with an epic banner
    let swift_file = root.join("Sources/App/main.swift");
    fs::create_dir_all(swift_file.parent().unwrap()).unwrap();
    fs::write(
        &swift_file,
        "// ⭐ EPIC 3 - core setup\nfunc run() {}\n",
    )
    .unwrap();

    // Create a Python file with a review round
    let py_file = root.join("scripts/tool.py");
    fs::create_dir_all(py_file.parent().unwrap()).unwrap();
    fs::write(
        &py_file,
        "# Review round 2 findings: fix\nprint('running')\n",
    )
    .unwrap();

    // Bare `qdev hygiene check` with no path arguments
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check"])
        .assert()
        .code(1);

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("main.swift"), "stdout must contain main.swift: {}", stdout);
    assert!(stdout.contains("tool.py"), "stdout must contain tool.py: {}", stdout);
    assert!(stdout.contains("Hygiene check: 2 findings"), "stdout must report 2 findings: {}", stdout);
}

#[test]
fn test_cli_diff_with_explicit_paths() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    setup_workspace(root);

    git(root, &["checkout", "-b", "feature/diff-path-test"]);

    // Create two dirty files
    let file_in_dir = root.join("src/sub/dirty1.rs");
    fs::create_dir_all(file_in_dir.parent().unwrap()).unwrap();
    fs::write(&file_in_dir, "// ⭐ STORY 1.0\nfn d1() {}\n").unwrap();

    let file_other = root.join("src/other/dirty2.rs");
    fs::create_dir_all(file_other.parent().unwrap()).unwrap();
    fs::write(&file_other, "// ⭐ STORY 2.0\nfn d2() {}\n").unwrap();

    git(root, &["add", "."]);

    // Running `--diff src/sub` should only inspect files in `src/sub`
    let mut cmd = Command::cargo_bin("qdev").unwrap();
    let assert = cmd
        .current_dir(root)
        .args(["hygiene", "check", "--diff", "src/sub"])
        .assert()
        .code(1);

    let stdout = String::from_utf8_lossy(&assert.get_output().stdout);
    assert!(stdout.contains("dirty1.rs"));
    assert!(!stdout.contains("dirty2.rs"));
    assert!(stdout.contains("Hygiene check: 1 finding"));
}

