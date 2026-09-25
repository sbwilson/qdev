use std::fs;
use std::path::Path;
use std::process::Command;

use qdev_core::config::{Config, GitConfig, ModuleConfig, StorageConfig};
use qdev_core::lease::claim_story;
use qdev_core::preflight::{
    check_branch_freshness, format_preflight_text, is_metadata_exempt, run_preflight,
    PreflightOptions, PreflightPayload, PreflightStatus,
};
use qdev_core::schema::PayloadKind;
use qdev_core::store::{FindingRecord, SqliteStore, Store};
use qdev_core::write::Author;
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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

fn setup_repo(root: &Path) -> Config {
    git(root, &["init", "-b", "main"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "Test User"]);
    git(root, &["config", "commit.gpgsign", "false"]);

    fs::create_dir_all(root.join(".qdev/cache")).unwrap();
    fs::create_dir_all(root.join(".qdev/leases")).unwrap();
    fs::create_dir_all(root.join(".qdev/chores")).unwrap();
    fs::create_dir_all(root.join("docs/specs/stories")).unwrap();
    fs::create_dir_all(root.join("docs/state/decisions")).unwrap();
    fs::create_dir_all(root.join("crates/foundation/src")).unwrap();
    fs::create_dir_all(root.join("crates/bridge/src")).unwrap();

    fs::write(root.join("crates/foundation/src/lib.rs"), "// foundation\n").unwrap();
    fs::write(root.join("crates/bridge/src/lib.rs"), "// bridge\n").unwrap();
    fs::write(root.join("README.md"), "# Test\n").unwrap();

    let story_yaml = "---\nid: E12S4\ntitle: Test Story\nstatus: in_progress\ntarget_modules:\n  - foundation\n---\n# Story\n";
    fs::write(root.join("docs/specs/stories/E12S4.md"), story_yaml).unwrap();

    let toml = r#"
[project]
name = "TestProject"

[git]
remote = "origin"
integration_branch = "develop"
branching_mode = "story-branch"
require_clean_tree_in_scope = true
max_integration_staleness_commits = 20

[storage]
specs_dir = "docs/specs"
state_dir = "docs/state"
cache_dir = ".qdev/cache"

[[modules]]
id = "foundation"
paths = ["crates/foundation/**"]

[[modules]]
id = "bridge"
paths = ["crates/bridge/**"]
"#;
    fs::write(root.join("qdev.toml"), toml).unwrap();

    git(root, &["add", "-A"]);
    git(root, &["commit", "-m", "Initial commit"]);

    Config {
        git: GitConfig {
            remote: "origin".to_string(),
            integration_branch: "develop".to_string(),
            branching_mode: "story-branch".to_string(),
            branch_template: "feature/{story_id}-{slug}".to_string(),
            require_clean_tree_in_scope: true,
            max_integration_staleness_commits: 20,
        },
        storage: StorageConfig {
            specs_dir: "docs/specs".to_string(),
            state_dir: "docs/state".to_string(),
            cache_dir: ".qdev/cache".to_string(),
        },
        modules: vec![
            ModuleConfig {
                id: "foundation".to_string(),
                paths: vec!["crates/foundation/**".to_string()],
                layer: None,
                may_depend_on: Vec::new(),
            },
            ModuleConfig {
                id: "bridge".to_string(),
                paths: vec!["crates/bridge/**".to_string()],
                layer: None,
                may_depend_on: Vec::new(),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn test_is_metadata_exempt() {
    let storage = StorageConfig::default();

    assert!(is_metadata_exempt("qdev.toml", &storage));
    assert!(is_metadata_exempt(".qdev.local.toml", &storage));
    assert!(is_metadata_exempt(".gitignore", &storage));
    assert!(is_metadata_exempt(".qdev/leases/E12S4.json", &storage));
    assert!(is_metadata_exempt(".qdev/cache/cache.sqlite", &storage));
    assert!(is_metadata_exempt(".git/index", &storage));
    assert!(is_metadata_exempt("docs/specs/stories/E12S4.md", &storage));
    assert!(is_metadata_exempt("docs/state/decisions/DEC-1.json", &storage));

    assert!(!is_metadata_exempt("crates/foundation/src/lib.rs", &storage));
    assert!(!is_metadata_exempt("README.md", &storage));
}

#[test]
fn test_preflight_non_git_directory() {
    let temp = TempDir::new().unwrap();
    let config = Config::default();
    let options = PreflightOptions::default();

    let result = run_preflight(temp.path(), &config, &options, None);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.exit_code().as_i32(), 4);
    assert_eq!(err.code(), "not_a_git_repository");
}

#[test]
fn test_preflight_clean_repository() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);

    // Create develop branch so integration branch exists
    git(root, &["branch", "develop"]);

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);
    assert_eq!(
        outcome.summary,
        "working tree in scope, integration branch fresh, zero blocking findings"
    );
    assert!(outcome.diagnostics.is_empty());
    assert_eq!(
        format_preflight_text(&outcome),
        "✓ preflight: working tree in scope, integration branch fresh, zero blocking findings"
    );
}

#[test]
fn test_working_tree_no_lease_no_chore_dirty_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    // Dirty an uncommitted non-exempt file
    fs::write(root.join("README.md"), "# Modified\n").unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Refusal);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].category, "working_tree");
    assert_eq!(outcome.diagnostics[0].code, "out_of_scope_changes");
    assert_eq!(outcome.diagnostics[0].remediation.as_deref(), Some("git stash -u"));
    assert_eq!(outcome.remediation_commands, vec!["git stash -u".to_string()]);
}

#[test]
fn test_working_tree_exempt_files_dirty_passes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    // Dirty an exempt metadata file
    fs::write(root.join("docs/specs/stories/E12S4.md"), "---\nmodified: true\n---\n").unwrap();
    fs::write(root.join(".gitignore"), "target/\n").unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);
}

#[test]
fn test_working_tree_with_story_lease_in_scope_passes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    // Claim lease for E12S4 (target_modules: [foundation])
    let author = Author::new("human", "simon");
    claim_story(root, "E12S4", &author, Some(&config.storage), None).unwrap();

    // Modify a file in foundation (in scope!)
    fs::write(root.join("crates/foundation/src/lib.rs"), "// updated in scope\n").unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);
}

#[test]
fn test_working_tree_with_story_lease_out_of_scope_refusal() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    // Claim lease for E12S4 (target_modules: [foundation])
    let author = Author::new("human", "simon");
    claim_story(root, "E12S4", &author, Some(&config.storage), None).unwrap();

    // Modify a file in bridge (out of scope for E12S4!)
    fs::write(root.join("crates/bridge/src/lib.rs"), "// modified outside scope\n").unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Refusal);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].category, "working_tree");
    assert_eq!(outcome.diagnostics[0].paths, vec!["crates/bridge/src/lib.rs"]);
    assert_eq!(
        outcome.diagnostics[0].remediation.as_deref(),
        Some("git stash push -u -m \"out-of-scope\" -- \"crates/bridge/src/lib.rs\"")
    );
}

#[test]
fn test_working_tree_explicit_story_flag_overrides_scope() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    // No lease held, but pass `--story E12S4`
    fs::write(root.join("crates/foundation/src/lib.rs"), "// updated in foundation\n").unwrap();

    let options = PreflightOptions {
        story: Some("E12S4".to_string()),
    };
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);

    // Now touch bridge
    fs::write(root.join("crates/bridge/src/lib.rs"), "// updated bridge\n").unwrap();
    let outcome2 = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome2.status, PreflightStatus::Refusal);
    assert_eq!(outcome2.diagnostics[0].paths, vec!["crates/bridge/src/lib.rs"]);
}

#[test]
fn test_working_tree_with_open_chore() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    // Start chore with allowlist crates/bridge/**
    let author = Author::new("human", "simon");
    qdev_core::chore::start_chore(&qdev_core::chore::StartChoreInput {
        workspace_root: root,
        storage: Some(&config.storage),
        title: "Tweak bridge".to_string(),
        paths: vec!["crates/bridge/**".to_string()],
        author,
        alongside: false,
    })
    .unwrap();

    // Modify file in bridge -> should pass!
    fs::write(root.join("crates/bridge/src/lib.rs"), "// chore edit\n").unwrap();
    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);

    // Modify file in foundation -> out of chore scope!
    fs::write(root.join("crates/foundation/src/lib.rs"), "// out of chore scope\n").unwrap();
    let outcome2 = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome2.status, PreflightStatus::Refusal);
    assert_eq!(outcome2.diagnostics[0].paths, vec!["crates/foundation/src/lib.rs"]);
}

#[test]
fn test_require_clean_tree_false_skips_scope_check() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = setup_repo(root);
    config.git.require_clean_tree_in_scope = false;
    git(root, &["branch", "develop"]);

    // Dirty tree without lease
    fs::write(root.join("README.md"), "# Modified\n").unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);
}

#[test]
fn test_branch_freshness_integration_branch_behind_remote() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);

    // Create a bare remote
    let remote_dir = TempDir::new().unwrap();
    git(remote_dir.path(), &["init", "--bare"]);
    git(root, &["remote", "add", "origin", remote_dir.path().to_str().unwrap()]);

    // Push develop to remote
    git(root, &["checkout", "-b", "develop"]);
    git(root, &["push", "-u", "origin", "develop"]);

    // Reset local develop back by 1 commit, while remote remains ahead
    fs::write(root.join("crates/foundation/src/lib.rs"), "// commit 2\n").unwrap();
    git(root, &["commit", "-am", "Commit 2 on remote"]);
    git(root, &["push", "origin", "develop"]);

    // Now rewind local develop to HEAD~1
    git(root, &["reset", "--hard", "HEAD~1"]);

    // When on develop branch itself, remediation recommends git pull
    let diags = check_branch_freshness(root, &config, None).unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "integration_branch_behind_remote");
    assert_eq!(
        diags[0].remediation.as_deref(),
        Some("git pull origin develop")
    );

    // When on another branch, remediation recommends git fetch
    git(root, &["checkout", "main"]);
    let diags2 = check_branch_freshness(root, &config, None).unwrap();
    assert_eq!(diags2.len(), 1);
    assert_eq!(diags2[0].code, "integration_branch_behind_remote");
    assert_eq!(
        diags2[0].remediation.as_deref(),
        Some("git fetch origin develop:develop")
    );
}

#[test]
fn test_branch_freshness_merge_base_staleness_limit() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = setup_repo(root);
    config.git.max_integration_staleness_commits = 2;

    // Create develop branch
    git(root, &["checkout", "-b", "develop"]);

    // Create feature branch from develop
    git(root, &["checkout", "-b", "feature/E12S4-test"]);

    // Advance develop by 3 commits (limit is 2)
    git(root, &["checkout", "develop"]);
    for i in 1..=3 {
        fs::write(root.join("crates/foundation/src/lib.rs"), format!("// develop commit {}\n", i)).unwrap();
        git(root, &["commit", "-am", &format!("develop commit {}", i)]);
    }

    // Switch back to feature branch
    git(root, &["checkout", "feature/E12S4-test"]);

    let diags = check_branch_freshness(root, &config, Some("E12S4")).unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "merge_base_stale");
    assert!(diags[0].message.contains("is 3 commits behind develop at merge-base (limit 2)"));
    assert!(diags[0].message.contains("Rebase onto develop before continuing E12S4."));
    assert_eq!(diags[0].remediation.as_deref(), Some("git rebase develop"));

    // Feature branch being ahead of develop is NOT an error
    git(root, &["checkout", "feature/E12S4-test"]);
    fs::write(root.join("crates/bridge/src/lib.rs"), "// feature commit\n").unwrap();
    git(root, &["commit", "-am", "feature commit"]);

    // If we rebase develop onto feature, staleness becomes 0
    git(root, &["rebase", "develop"]);
    let diags2 = check_branch_freshness(root, &config, Some("E12S4")).unwrap();
    assert!(diags2.is_empty());
}

#[test]
fn test_branch_freshness_trunk_mode_behind_remote() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = setup_repo(root);
    config.git.branching_mode = "trunk".to_string();

    let remote_dir = TempDir::new().unwrap();
    git(remote_dir.path(), &["init", "--bare"]);
    git(root, &["remote", "add", "origin", remote_dir.path().to_str().unwrap()]);
    git(root, &["push", "-u", "origin", "main"]);

    // Add commit on remote
    fs::write(root.join("crates/foundation/src/lib.rs"), "// commit on remote\n").unwrap();
    git(root, &["commit", "-am", "Commit on remote"]);
    git(root, &["push", "origin", "main"]);

    // Rewind local
    git(root, &["reset", "--hard", "HEAD~1"]);

    let diags = check_branch_freshness(root, &config, None).unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, "trunk_branch_behind_remote");
    assert_eq!(diags[0].remediation.as_deref(), Some("git pull origin main"));
}

#[test]
fn test_validation_health_blocking_error() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    let store = SqliteStore::open_in_memory().unwrap();
    // Insert an error finding into store
    store.upsert_finding(&FindingRecord {
        path: "docs/specs/stories/E99S99.md".to_string(),
        code: "orphan_deferred_work".to_string(),
        severity: "error".to_string(),
        message: Some("Orphan deferred work record".to_string()),
        found_at: "2026-09-25T00:00:00Z".to_string(),
    }).unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, Some(&store)).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Refusal);
    assert_eq!(outcome.diagnostics.len(), 1);
    assert_eq!(outcome.diagnostics[0].category, "validation");
    assert_eq!(outcome.diagnostics[0].code, "blocking_validation_errors");
    assert_eq!(outcome.diagnostics[0].remediation.as_deref(), Some("qdev validate"));
}

#[test]
fn test_preflight_payload_schema_conformance() {
    let schema_str = PayloadKind::Preflight.schema_str();
    let schema_val: serde_json::Value = serde_json::from_str(schema_str).unwrap();
    let validator = jsonschema::validator_for(&schema_val).unwrap();

    // 1. Pass payload
    let pass_payload = PreflightPayload {
        status: "pass".to_string(),
        summary: "working tree in scope, integration branch fresh, zero blocking findings".to_string(),
        diagnostics: Vec::new(),
        remediation_commands: Vec::new(),
    };
    let pass_val = serde_json::to_value(qdev_core::JsonEnvelope::new(pass_payload)).unwrap();
    assert!(validator.is_valid(&pass_val));

    // 2. Refusal payload
    let refusal_payload = PreflightPayload {
        status: "refusal".to_string(),
        summary: "preflight checks failed with 1 issue(s)".to_string(),
        diagnostics: vec![qdev_core::preflight::PreflightDiagnostic {
            category: "working_tree".to_string(),
            code: "out_of_scope_changes".to_string(),
            message: "uncommitted changes outside target modules".to_string(),
            paths: vec!["crates/bridge/src/lib.rs".to_string()],
            remediation: Some("git stash push -u -m \"out-of-scope\" -- \"crates/bridge/src/lib.rs\"".to_string()),
        }],
        remediation_commands: vec!["git stash push -u -m \"out-of-scope\" -- \"crates/bridge/src/lib.rs\"".to_string()],
    };
    let refusal_val = serde_json::to_value(qdev_core::JsonEnvelope::new(refusal_payload)).unwrap();
    assert!(validator.is_valid(&refusal_val));
}

#[test]
fn test_branch_freshness_within_staleness_limit_passes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let mut config = setup_repo(root);
    config.git.max_integration_staleness_commits = 2;

    // Create develop branch
    git(root, &["checkout", "-b", "develop"]);

    // Create feature branch from develop
    git(root, &["checkout", "-b", "feature/E12S4-test"]);

    // Advance develop by 1 commit (limit is 2, so 1 <= 2 passes)
    git(root, &["checkout", "develop"]);
    fs::write(root.join("crates/foundation/src/lib.rs"), "// develop commit 1\n").unwrap();
    git(root, &["commit", "-am", "develop commit 1"]);

    // Switch back to feature branch
    git(root, &["checkout", "feature/E12S4-test"]);

    let diags = check_branch_freshness(root, &config, Some("E12S4")).unwrap();
    assert!(diags.is_empty(), "expected 0 diagnostics when staleness (1) <= limit (2)");

    let options = PreflightOptions {
        story: Some("E12S4".to_string()),
    };
    let outcome = run_preflight(root, &config, &options, None).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);
}

#[test]
fn test_validation_health_warning_does_not_block() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = setup_repo(root);
    git(root, &["branch", "develop"]);

    let store = SqliteStore::open_in_memory().unwrap();
    // Insert a warning finding into store (severity = "warning")
    store.upsert_finding(&FindingRecord {
        path: "docs/specs/stories/E12S4.md".to_string(),
        code: "orphan_warning".to_string(),
        severity: "warning".to_string(),
        message: Some("Non-blocking warning".to_string()),
        found_at: "2026-09-25T00:00:00Z".to_string(),
    }).unwrap();

    let options = PreflightOptions::default();
    let outcome = run_preflight(root, &config, &options, Some(&store)).unwrap();
    assert_eq!(outcome.status, PreflightStatus::Pass);
    assert_eq!(outcome.summary, "working tree in scope, integration branch fresh, zero blocking findings");
}
