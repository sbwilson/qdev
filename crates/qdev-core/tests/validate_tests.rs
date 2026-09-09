//! `qdev validate` core logic tests (spec-1-11), covering the I/O & edge-case matrix: the four
//! freshly computed checks, cache-native findings passthrough, and `--changed` filtering against
//! a real git merge-base diff.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use qdev_core::store::{DeferredWorkRecord, EntityRecord, FindingRecord, SqliteStore, Store};
use qdev_core::write::Author;
use qdev_core::{Config, EntityKind, ModuleConfig};
use tempfile::TempDir;

fn entity(id: &str, kind: EntityKind, source_path: &str) -> EntityRecord {
    EntityRecord {
        id: id.to_string(),
        kind,
        title: Some(format!("{} title", id)),
        status: Some("draft".to_string()),
        owners: None,
        source_path: source_path.to_string(),
        content_hash: "hash".to_string(),
        version: 1,
        created_by: Some(Author::new("human", "simon")),
        updated_by: Some(Author::new("human", "simon")),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    }
}

fn write_story(dir: &Path, id: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join(format!("{}.md", id)),
        format!(
            r#"---
id: {id}
title: "Story {id}"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#
        ),
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn test_happy_path_clean_workspace_yields_no_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    let config = Config::default();

    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    assert!(findings.is_empty());
    assert!(!qdev_core::has_error_finding(&findings));
}

// ---------------------------------------------------------------------------
// Duplicate planning ids
// ---------------------------------------------------------------------------

#[test]
fn test_duplicate_ids_report_both_paths_and_name_each_other() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S2");
    // A second file also declaring E1S2 (simulating concurrent planning on separate branches).
    fs::write(
        stories_dir.join("E1S2-dup.md"),
        r#"---
id: E1S2
title: "Duplicate"
status: draft
version: 1
created_by:
  type: human
  id: simon
updated_by:
  type: human
  id: simon
---

## Acceptance Criteria
- AC.
"#,
    )
    .unwrap();

    let store = SqliteStore::open_in_memory().unwrap();
    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();

    let dup_findings: Vec<&FindingRecord> = findings
        .iter()
        .filter(|f| f.code == "duplicate_planning_id")
        .collect();
    assert_eq!(dup_findings.len(), 2);
    assert!(dup_findings.iter().all(|f| f.severity == "error"));
    let paths: Vec<&str> = dup_findings.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.contains(&"docs/specs/stories/E1S2.md"));
    assert!(paths.contains(&"docs/specs/stories/E1S2-dup.md"));
    for f in &dup_findings {
        let other = if f.path.ends_with("E1S2.md") {
            "E1S2-dup.md"
        } else {
            "E1S2.md"
        };
        assert!(f.message.as_deref().unwrap().contains(other));
    }
    assert!(qdev_core::has_error_finding(&findings));
}

// ---------------------------------------------------------------------------
// Cache-native findings pass through verbatim
// ---------------------------------------------------------------------------

#[test]
fn test_cache_native_findings_surfaced_via_list_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_finding(&FindingRecord {
            path: "docs/specs/stories/E1S1.md".to_string(),
            code: "dependency_cycle".to_string(),
            severity: "error".to_string(),
            message: Some("depends_on cycle: E1S1 -> E1S2 -> E1S1".to_string()),
            found_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();
    store
        .upsert_finding(&FindingRecord {
            path: "docs/specs/stories/E1S3.md".to_string(),
            code: "merge_conflict".to_string(),
            severity: "error".to_string(),
            message: Some("conflict markers found".to_string()),
            found_at: "2026-01-01T00:00:00Z".to_string(),
        })
        .unwrap();

    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();

    assert!(findings.iter().any(|f| f.code == "dependency_cycle"));
    assert!(findings.iter().any(|f| f.code == "merge_conflict"));
    assert!(qdev_core::has_error_finding(&findings));
}

// ---------------------------------------------------------------------------
// Orphan deferred work
// ---------------------------------------------------------------------------

#[test]
fn test_orphan_deferred_work_when_origin_story_missing() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "DW-a1b2",
            EntityKind::DeferredWork,
            "docs/state/dw/DW-a1b2.md",
        ))
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-a1b2".to_string(),
            origin_story_id: Some("E9S9".to_string()),
            target_module: "core".to_string(),
            status: Some("open".to_string()),
            safety_risk: Some("negligible".to_string()),
            rationale: None,
            gate: None,
            resolution: None,
        })
        .unwrap();

    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    let orphan: Vec<&FindingRecord> = findings
        .iter()
        .filter(|f| f.code == "orphan_deferred_work")
        .collect();
    assert_eq!(orphan.len(), 1);
    assert_eq!(orphan[0].path, "docs/state/dw/DW-a1b2.md");
    assert_eq!(orphan[0].severity, "error");
}

#[test]
fn test_no_orphan_finding_when_origin_story_exists() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E9S9",
            EntityKind::Story,
            "docs/specs/stories/E9S9.md",
        ))
        .unwrap();
    store
        .upsert_entity(&entity(
            "DW-a1b2",
            EntityKind::DeferredWork,
            "docs/state/dw/DW-a1b2.md",
        ))
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-a1b2".to_string(),
            origin_story_id: Some("E9S9".to_string()),
            target_module: "core".to_string(),
            status: Some("open".to_string()),
            safety_risk: Some("negligible".to_string()),
            rationale: None,
            gate: None,
            resolution: None,
        })
        .unwrap();

    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    assert!(!findings.iter().any(|f| f.code == "orphan_deferred_work"));
}

// ---------------------------------------------------------------------------
// DW missing rationale
// ---------------------------------------------------------------------------

#[test]
fn test_dw_missing_rationale_for_unacceptable_risk() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "DW-c3d4",
            EntityKind::DeferredWork,
            "docs/state/dw/DW-c3d4.md",
        ))
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-c3d4".to_string(),
            origin_story_id: None,
            target_module: "core".to_string(),
            status: Some("open".to_string()),
            safety_risk: Some("unacceptable".to_string()),
            rationale: None,
            gate: None,
            resolution: None,
        })
        .unwrap();

    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    let missing: Vec<&FindingRecord> = findings
        .iter()
        .filter(|f| f.code == "dw_missing_rationale")
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].path, "docs/state/dw/DW-c3d4.md");
}

#[test]
fn test_dw_with_rationale_and_negligible_risk_is_clean() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "DW-e5f6",
            EntityKind::DeferredWork,
            "docs/state/dw/DW-e5f6.md",
        ))
        .unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-e5f6".to_string(),
            origin_story_id: None,
            target_module: "core".to_string(),
            status: Some("open".to_string()),
            safety_risk: Some("negligible".to_string()),
            rationale: None,
            gate: None,
            resolution: None,
        })
        .unwrap();

    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    assert!(!findings.iter().any(|f| f.code == "dw_missing_rationale"));
}

// ---------------------------------------------------------------------------
// Unregistered target module
// ---------------------------------------------------------------------------

#[test]
fn test_unregistered_target_module_reported() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    let mut story = entity("E1S1", EntityKind::Story, "docs/specs/stories/E1S1.md");
    story.epic_id = Some("E1".to_string());
    story.seq = Some(1);
    story.target_modules = Some(r#"["ghost"]"#.to_string());
    store.upsert_entity(&story).unwrap();

    let config = Config {
        modules: vec![ModuleConfig {
            id: "core".to_string(),
            paths: vec!["crates/core/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        }],
        ..Config::default()
    };

    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    let unreg: Vec<&FindingRecord> = findings
        .iter()
        .filter(|f| f.code == "target_module_not_registered")
        .collect();
    assert_eq!(unreg.len(), 1);
    assert_eq!(unreg[0].path, "docs/specs/stories/E1S1.md");
    assert!(unreg[0].message.as_deref().unwrap().contains("ghost"));
}

#[test]
fn test_registered_target_module_is_clean() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();
    let mut story = entity("E1S1", EntityKind::Story, "docs/specs/stories/E1S1.md");
    story.epic_id = Some("E1".to_string());
    story.seq = Some(1);
    story.target_modules = Some(r#"["core"]"#.to_string());
    store.upsert_entity(&story).unwrap();

    let config = Config {
        modules: vec![ModuleConfig {
            id: "core".to_string(),
            paths: vec!["crates/core/**".to_string()],
            layer: None,
            may_depend_on: vec![],
        }],
        ..Config::default()
    };

    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    assert!(!findings
        .iter()
        .any(|f| f.code == "target_module_not_registered"));
}

// ---------------------------------------------------------------------------
// --changed filtering against a real throwaway git repo
// ---------------------------------------------------------------------------

fn git(root: &Path, args: &[&str]) {
    let mut full_args = vec!["-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false"];
    full_args.extend_from_slice(args);
    let status = Command::new("git")
        .args(&full_args)
        .current_dir(root)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

/// Builds a throwaway git repo fixture with its own commits and branches, per spec: `--changed`
/// tests must not depend on the ambient CI checkout.
fn init_git_fixture(root: &Path) {
    git(root, &["init", "-q", "-b", "develop"]);
    fs::write(root.join("README.md"), "root\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "initial"]);
    git(root, &["checkout", "-q", "-b", "feature"]);
}

#[test]
fn test_changed_excludes_findings_outside_merge_base_diff() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_fixture(root);

    // The feature branch has diverged from develop but nothing has been committed on it yet, so
    // the merge-base diff is empty: findings for files that exist (from the shared baseline
    // commit) but were never touched on this branch must all be excluded.
    let changed = qdev_core::git_changed_files(root, "develop").unwrap();
    assert!(changed.is_empty());

    let findings = vec![
        FindingRecord {
            path: "docs/specs/stories/E1S1.md".to_string(),
            code: "duplicate_planning_id".to_string(),
            severity: "error".to_string(),
            message: None,
            found_at: "t".to_string(),
        },
        FindingRecord {
            path: "docs/specs/stories/E1S2.md".to_string(),
            code: "duplicate_planning_id".to_string(),
            severity: "error".to_string(),
            message: None,
            found_at: "t".to_string(),
        },
    ];
    let filtered = qdev_core::filter_by_changed(findings, &changed);
    assert!(filtered.is_empty());
}

#[test]
fn test_changed_keeps_findings_touching_diffed_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_fixture(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    git(root, &["add", "."]);
    git(
        root,
        &["commit", "-q", "-m", "touch E1S1 on feature branch"],
    );

    let changed = qdev_core::git_changed_files(root, "develop").unwrap();
    assert!(changed.contains("docs/specs/stories/E1S1.md"));

    let findings = vec![
        FindingRecord {
            path: "docs/specs/stories/E1S1.md".to_string(),
            code: "duplicate_planning_id".to_string(),
            severity: "error".to_string(),
            message: None,
            found_at: "t".to_string(),
        },
        FindingRecord {
            path: "docs/specs/stories/E1S9.md".to_string(),
            code: "duplicate_planning_id".to_string(),
            severity: "error".to_string(),
            message: None,
            found_at: "t".to_string(),
        },
    ];
    let filtered = qdev_core::filter_by_changed(findings, &changed);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].path, "docs/specs/stories/E1S1.md");
}

#[test]
fn test_changed_missing_branch_is_infrastructure_failure() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_fixture(root);

    let err = qdev_core::git_changed_files(root, "does-not-exist").unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::InfrastructureFailure);
}

#[test]
fn test_changed_not_a_git_repo_is_infrastructure_failure() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    // No `git init` here: root is a plain directory, not a repository.
    let err = qdev_core::git_changed_files(root, "develop").unwrap_err();
    assert_eq!(err.exit_code(), qdev_core::ExitCode::InfrastructureFailure);
}

// ---------------------------------------------------------------------------
// git_changed_files: untracked files, subdirectory workspaces, quoted paths
// ---------------------------------------------------------------------------

/// `git diff --name-only` reports tracked changes only, so a file that was just created and
/// never `git add`ed — the most common way a fresh `duplicate_planning_id` appears — is picked
/// up by the `git ls-files --others` half. Every other `--changed` test commits first, so
/// deleting that half leaves them all green while the gate silently stops firing.
#[test]
fn test_changed_includes_untracked_files() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_fixture(root);

    let stories_dir = root.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    // Deliberately not staged or committed.

    let changed = qdev_core::git_changed_files(root, "develop").unwrap();
    assert!(
        changed.contains("docs/specs/stories/E1S1.md"),
        "an untracked new file must count as changed, got {:?}",
        changed
    );
}

/// Findings carry workspace-relative paths, but `git diff` reports repository-relative ones.
/// When the workspace is a subdirectory of its repository the two only line up because of
/// `--relative`; without it the changed set can never match a finding and `--changed` reports
/// nothing at all, however broken the workspace is.
#[test]
fn test_changed_paths_are_workspace_relative_in_a_subdirectory_workspace() {
    let temp = TempDir::new().unwrap();
    let repo_root = temp.path();
    init_git_fixture(repo_root);

    let workspace = repo_root.join("packages/app");
    fs::create_dir_all(&workspace).unwrap();
    let stories_dir = workspace.join("docs/specs/stories");
    write_story(&stories_dir, "E1S1");
    // Committed, so the path comes from `git diff` — the half that reports repository-relative
    // paths. (`git ls-files --others` is already relative to the working directory, so an
    // untracked file would pass this test even with the bug.)
    git(repo_root, &["add", "."]);
    git(repo_root, &["commit", "-q", "-m", "add app story"]);

    let changed = qdev_core::git_changed_files(&workspace, "develop").unwrap();
    assert!(
        changed.contains("docs/specs/stories/E1S1.md"),
        "paths must be relative to the workspace, not the repository root; got {:?}",
        changed
    );
    assert!(
        !changed.contains("packages/app/docs/specs/stories/E1S1.md"),
        "repository-relative paths would never match a finding's path; got {:?}",
        changed
    );
}

/// git octal-escapes non-ASCII paths unless `core.quotePath` is off, and an escaped path never
/// matches the finding it belongs to.
#[test]
fn test_changed_includes_non_ascii_paths_unescaped() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    init_git_fixture(root);

    let stories_dir = root.join("docs/specs/stories");
    fs::create_dir_all(&stories_dir).unwrap();
    // One untracked and one committed-then-modified, so both halves of `git_changed_files` are
    // exercised: `git ls-files --others` for the first, `git diff --name-only` for the second.
    // Only the diff half needs `core.quotePath=false`, so covering just the untracked file
    // would leave that flag unverified.
    fs::write(stories_dir.join("café.md"), "---\nid: E1S1\n---\n").unwrap();
    fs::write(stories_dir.join("naïve.md"), "---\nid: E1S2\n---\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-q", "-m", "add naive story"]);
    fs::write(
        stories_dir.join("naïve.md"),
        "---\nid: E1S2\ntitle: x\n---\n",
    )
    .unwrap();
    // café.md was committed by that `git add .` too, so re-create it as an untracked file.
    git(
        root,
        &["rm", "-q", "--cached", "docs/specs/stories/café.md"],
    );

    let changed = qdev_core::git_changed_files(root, "develop").unwrap();
    assert!(
        changed.contains("docs/specs/stories/naïve.md"),
        "a modified non-ASCII path must come back unescaped from `git diff`, got {:?}",
        changed
    );
    assert!(
        changed.contains("docs/specs/stories/café.md"),
        "an untracked non-ASCII path must come back unescaped, got {:?}",
        changed
    );
}

// ---------------------------------------------------------------------------
// filter_by_changed / sort_findings
// ---------------------------------------------------------------------------

fn cycle_finding(path: &str, message: Option<&str>) -> FindingRecord {
    FindingRecord {
        path: path.to_string(),
        code: "dependency_cycle".to_string(),
        severity: "error".to_string(),
        message: message.map(str::to_string),
        found_at: "t".to_string(),
    }
}

/// Cycle findings are grouped by message so a cycle is kept or dropped whole. Two *different*
/// message-less cycles must not collapse into one group, or an untouched cycle gets reported as
/// changed on the strength of an unrelated one.
#[test]
fn test_message_less_cycles_are_not_merged_into_one_group() {
    let mut changed = HashSet::new();
    changed.insert("a.md".to_string());

    let findings = vec![cycle_finding("a.md", None), cycle_finding("b.md", None)];
    let kept = qdev_core::filter_by_changed(findings, &changed);

    assert_eq!(
        kept.len(),
        1,
        "only the cycle touching a changed path may be kept, got {:?}",
        kept
    );
    assert_eq!(kept[0].path, "a.md");
}

/// `(path, code)` is not a total order once a path can hold two findings of the same code, so
/// sorting on it alone leaves the JSON output's row order down to the input order.
#[test]
fn test_sort_findings_is_a_total_order_over_same_path_and_code() {
    let make = |message: &str| FindingRecord {
        path: "a.md".to_string(),
        code: "dangling_relation".to_string(),
        severity: "error".to_string(),
        message: Some(message.to_string()),
        found_at: "t".to_string(),
    };

    let mut forwards = vec![make("beta"), make("alpha")];
    let mut backwards = vec![make("alpha"), make("beta")];
    qdev_core::sort_findings(&mut forwards);
    qdev_core::sort_findings(&mut backwards);

    assert_eq!(forwards, backwards, "sort must not depend on input order");
    assert_eq!(forwards[0].message.as_deref(), Some("alpha"));
}

// ---------------------------------------------------------------------------
// rewrite_frontmatter_id edge cases
// ---------------------------------------------------------------------------

/// Only a `#` outside quotes and preceded by whitespace opens a YAML comment. Treating any `#`
/// as one re-emits half the old value as a comment on the rewritten line.
#[test]
fn test_rewrite_frontmatter_id_keeps_a_hash_inside_a_quoted_value() {
    let content = "---\nid: \"E1S1#draft\"\ntitle: x\n---\n\nbody\n";
    let out = qdev_core::rewrite_frontmatter_id(content, "E1S9").unwrap();
    assert!(out.contains("id: E1S9\n"), "got {:?}", out);
    assert!(!out.contains('#'), "no comment may be invented: {:?}", out);
}

#[test]
fn test_rewrite_frontmatter_id_preserves_a_real_trailing_comment() {
    let content = "---\nid: E1S1 # allocated by hand\ntitle: x\n---\n\nbody\n";
    let out = qdev_core::rewrite_frontmatter_id(content, "E1S9").unwrap();
    assert!(
        out.contains("id: E1S9 # allocated by hand\n"),
        "got {:?}",
        out
    );
}

/// The line ending comes from the id line itself. Deriving it from any CRLF anywhere in the
/// file rewrites this one line with CRLF in an otherwise-LF document, for pure diff noise.
#[test]
fn test_rewrite_frontmatter_id_keeps_the_id_lines_own_line_ending() {
    let content = "---\nid: E1S1\ntitle: x\n---\n\nbody with a stray \r\n carriage return\n";
    let out = qdev_core::rewrite_frontmatter_id(content, "E1S9").unwrap();
    assert!(out.contains("id: E1S9\n"), "got {:?}", out);
    assert!(!out.contains("id: E1S9\r\n"), "got {:?}", out);
}

// ---------------------------------------------------------------------------
// glob_match
// ---------------------------------------------------------------------------

/// A bare directory path is the natural thing to write in `config.modules[].paths`. Anchored
/// matching made it match nothing at all, so `--fix-ids` reported zero citations as though
/// there were none to find.
#[test]
fn test_glob_match_treats_a_bare_directory_as_everything_under_it() {
    assert!(qdev_core::glob_match(
        "crates/qdev-core",
        "crates/qdev-core"
    ));
    assert!(qdev_core::glob_match(
        "crates/qdev-core",
        "crates/qdev-core/src/lib.rs"
    ));
    assert!(!qdev_core::glob_match(
        "crates/qdev-core",
        "crates/qdev-cli/src/main.rs"
    ));
    // A prefix that is not a path segment must not match.
    assert!(!qdev_core::glob_match(
        "crates/qdev",
        "crates/qdev-core/x.rs"
    ));
}

#[test]
fn test_glob_match_still_handles_wildcards() {
    assert!(qdev_core::glob_match("src/**", "src/a/b.rs"));
    assert!(qdev_core::glob_match("src/*.rs", "src/lib.rs"));
    assert!(!qdev_core::glob_match("src/*.rs", "src/a/lib.rs"));
}

// ---------------------------------------------------------------------------
// rewrite_citations
// ---------------------------------------------------------------------------

/// The id is substituted inside whatever the configured pattern matched. Emitting a fixed
/// `[NEWID]` corrupts any citation syntax that is not bracket-delimited.
#[test]
fn test_rewrite_citations_preserves_a_non_bracket_citation_syntax() {
    let pattern = qdev_core::regex::Regex::new(r"\{\{([A-Z0-9]+)\}\}").unwrap();
    let (out, count) =
        qdev_core::rewrite_citations("see {{E1S1}} and {{E1S2}}", &pattern, "E1S1", "E1S9");
    assert_eq!(count, 1);
    assert_eq!(out, "see {{E1S9}} and {{E1S2}}");
}
