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

// ---------------------------------------------------------------------------
// entity_file_off_convention: the identity rule every writer resolves by
// ---------------------------------------------------------------------------

/// A file whose name carries its id, in the standard directory for its kind, is the convention —
/// nothing to report.
#[test]
fn test_on_convention_entity_file_yields_no_finding() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E1S2",
            EntityKind::Story,
            "docs/specs/stories/E1S2.md",
        ))
        .unwrap();
    // A `-slug` suffix after the id is part of the convention, not a deviation.
    store
        .upsert_entity(&entity(
            "E1S3",
            EntityKind::Story,
            "docs/specs/stories/E1S3-buffer-layout.md",
        ))
        .unwrap();

    let findings =
        qdev_core::find_off_convention_entity_files(&store, &Config::default().storage).unwrap();
    assert!(findings.is_empty(), "{:?}", findings);
}

/// A filename that does not carry its entity's id is exactly what the write path cannot resolve:
/// reported, at `warning` severity so a workspace that was legal before does not start failing.
#[test]
fn test_filename_not_carrying_its_id_is_a_warning_naming_the_expected_name() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E1S9",
            EntityKind::Story,
            "docs/specs/stories/login-flow.md",
        ))
        .unwrap();

    let findings =
        qdev_core::find_off_convention_entity_files(&store, &Config::default().storage).unwrap();
    assert_eq!(findings.len(), 1, "{:?}", findings);
    assert_eq!(findings[0].code, "entity_file_off_convention");
    assert_eq!(findings[0].severity, "warning");
    assert_eq!(findings[0].path, "docs/specs/stories/login-flow.md");
    let message = findings[0].message.as_deref().unwrap();
    assert!(
        message.contains("docs/specs/stories/login-flow.md"),
        "{message}"
    );
    assert!(message.contains("docs/specs/stories/E1S9.md"), "{message}");
    // Not an error: `qdev validate` must still exit 0 on it.
    assert!(!qdev_core::has_error_finding(&findings));
}

/// The other half of the rule: a file outside every standard entity directory is readable
/// (hydration walks recursively) but unresolvable by the write path.
#[test]
fn test_entity_file_outside_the_standard_directories_is_a_warning() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E1S9",
            EntityKind::Story,
            "docs/specs/misc/E1S9.md",
        ))
        .unwrap();

    let findings =
        qdev_core::find_off_convention_entity_files(&store, &Config::default().storage).unwrap();
    assert_eq!(findings.len(), 1, "{:?}", findings);
    assert_eq!(findings[0].severity, "warning");
    assert!(findings[0]
        .message
        .as_deref()
        .unwrap()
        .contains("outside every standard entity directory"));
}

/// A frontmatter `kind:` that disagrees with the file's directory is a kind question, not an
/// identity one: the write path's cross-kind fallback resolves the id in any standard directory,
/// so this must not be reported — otherwise the kind-disagreement case would start emitting a
/// finding on every sweep.
#[test]
fn test_kind_disagreeing_with_a_standard_directory_is_not_off_convention() {
    let store = SqliteStore::open_in_memory().unwrap();
    // An ADR file (in the ADR directory) whose frontmatter declares `kind: story`, so hydration
    // recorded it as a Story.
    store
        .upsert_entity(&entity(
            "AD-9",
            EntityKind::Story,
            "docs/specs/adrs/AD-9.md",
        ))
        .unwrap();

    let findings =
        qdev_core::find_off_convention_entity_files(&store, &Config::default().storage).unwrap();
    assert!(findings.is_empty(), "{:?}", findings);
}

/// The check honours a configured layout rather than the default one.
#[test]
fn test_off_convention_check_follows_configured_storage_dirs() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity("E1S2", EntityKind::Story, "spec/stories/E1S2.md"))
        .unwrap();

    let mut config = Config::default();
    config.storage.specs_dir = "spec".to_string();
    let findings = qdev_core::find_off_convention_entity_files(&store, &config.storage).unwrap();
    assert!(findings.is_empty(), "{:?}", findings);

    // The same row is off-convention under the default layout.
    let default_findings =
        qdev_core::find_off_convention_entity_files(&store, &Config::default().storage).unwrap();
    assert_eq!(default_findings.len(), 1);
}

/// `run_validation` reports it alongside the other computed checks, and it alone must not change
/// the exit-code decision.
#[test]
fn test_run_validation_includes_the_off_convention_check_without_failing_the_run() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E1S9",
            EntityKind::Story,
            "docs/specs/stories/login-flow.md",
        ))
        .unwrap();

    let findings = qdev_core::run_validation(&store, temp.path(), &Config::default()).unwrap();
    assert_eq!(findings.len(), 1, "{:?}", findings);
    assert_eq!(findings[0].code, "entity_file_off_convention");
    assert!(!qdev_core::has_error_finding(&findings));
}

/// Three of the four stale-row skips had no test: every fixture in this file builds
/// `stale: false` rows, so removing the guards from the two deferred-work checks and the
/// off-convention check left the suite green — and with them the divergence this story removes
/// (a sweep reporting a finding derived from pre-edit content a rebuild never reports).
#[test]
fn test_computed_checks_skip_stale_rows() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    // A deferred-work entity whose file no longer parses: the row is retained, marked stale.
    let mut dw = entity(
        "DW-a1b2",
        EntityKind::DeferredWork,
        "docs/state/dw/DW-a1b2.md",
    );
    dw.stale = true;
    store.upsert_entity(&dw).unwrap();
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-a1b2".to_string(),
            // Both an orphan origin *and* an unacceptable risk with no rationale, so a
            // non-stale row here would produce two computed findings.
            origin_story_id: Some("E9S9".to_string()),
            target_module: "core".to_string(),
            status: Some("open".to_string()),
            safety_risk: Some("unacceptable".to_string()),
            rationale: None,
            gate: None,
            resolution: None,
        })
        .unwrap();

    // A story whose filename does not carry its id, also stale.
    let mut off = entity("E1S4", EntityKind::Story, "docs/specs/stories/whatever.md");
    off.stale = true;
    store.upsert_entity(&off).unwrap();

    let config = Config::default();
    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    let computed: Vec<&str> = findings
        .iter()
        .map(|f| f.code.as_str())
        .filter(|c| {
            matches!(
                *c,
                "orphan_deferred_work" | "dw_missing_rationale" | "entity_file_off_convention"
            )
        })
        .collect();
    assert!(
        computed.is_empty(),
        "no computed finding may be derived from a stale row: {computed:?}"
    );

    // The same rows, not stale, must produce all three — otherwise this test would pass for the
    // wrong reason (a fixture that never triggers the checks at all).
    let mut dw_live = dw.clone();
    dw_live.stale = false;
    store.upsert_entity(&dw_live).unwrap();
    let mut off_live = off.clone();
    off_live.stale = false;
    store.upsert_entity(&off_live).unwrap();

    let findings = qdev_core::run_validation(&store, root, &config).unwrap();
    let codes: Vec<&str> = findings.iter().map(|f| f.code.as_str()).collect();
    for expected in [
        "orphan_deferred_work",
        "dw_missing_rationale",
        "entity_file_off_convention",
    ] {
        assert!(
            codes.contains(&expected),
            "a live row must still produce {expected}: {codes:?}"
        );
    }
}

/// The existence *probe* the check makes about a row it merely refers to — the case the test
/// above misses, because its fixtures only ever made the check's own subject stale. A stale
/// origin story is absent for the purpose of deriving a finding (`Store::
/// entity_exists_for_derivation`), so the orphan is reported exactly as a rebuild — which has no
/// row for that story at all — reports it. Each of the three origin states is asserted, so this
/// cannot pass for want of a triggering fixture.
#[test]
fn test_orphan_deferred_work_treats_a_stale_origin_story_as_absent() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let config = Config::default();
    let store = SqliteStore::open_in_memory().unwrap();

    // A *live* deferred-work row: the check's own subject is not what is under test here.
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
            origin_story_id: Some("E1S1".to_string()),
            target_module: "core".to_string(),
            status: Some("open".to_string()),
            // Negligible with no rationale, so `dw_missing_rationale` cannot fire and the
            // assertions below are about the origin probe alone.
            safety_risk: Some("negligible".to_string()),
            rationale: None,
            gate: None,
            resolution: None,
        })
        .unwrap();

    let orphan_codes = |store: &SqliteStore| -> Vec<String> {
        qdev_core::run_validation(store, root, &config)
            .unwrap()
            .into_iter()
            .filter(|f| f.code == "orphan_deferred_work")
            .map(|f| f.path)
            .collect()
    };

    // 1. No origin story at all: reported, as it always was.
    assert_eq!(
        orphan_codes(&store),
        vec!["docs/state/dw/DW-a1b2.md".to_string()],
        "a missing origin story must be reported"
    );

    // 2. The origin story hydrated and *stale* — its file is merge-conflicted, so its retained
    // row describes content the file may no longer have. This is the defect: the probe used to
    // read the retained row as present and swallow the finding on the sweep path only.
    let mut origin = entity("E1S1", EntityKind::Story, "docs/specs/stories/E1S1.md");
    origin.stale = true;
    store.upsert_entity(&origin).unwrap();
    assert_eq!(
        orphan_codes(&store),
        vec!["docs/state/dw/DW-a1b2.md".to_string()],
        "a stale origin story is absent for derivation: the orphan must still be reported"
    );

    // The derivation rule does not leak into reads: `qdev get` still returns the stale row.
    let read_back = store.get_entity("E1S1").unwrap().expect("row is retained");
    assert!(read_back.stale, "a read must still see the stale row");

    // 3. The same origin story, parsing: nothing to report. Without this the test would pass
    // for a check that reported the orphan unconditionally.
    origin.stale = false;
    store.upsert_entity(&origin).unwrap();
    assert!(
        orphan_codes(&store).is_empty(),
        "a live origin story must suppress the finding, exactly as before this change"
    );
}

// ---------------------------------------------------------------------------
// One rule for which ids are in use
// ---------------------------------------------------------------------------

/// The cache is a union member: an id belonging to a hydrated entity is in use even when the
/// filesystem half can no longer see it — the file has been deleted or has become unreadable —
/// so no allocator hands it out again.
#[test]
fn test_ids_in_use_includes_cache_only_ids() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let storage = qdev_core::StorageConfig::default();

    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E1S1",
            EntityKind::Story,
            "docs/specs/stories/E1S1.md",
        ))
        .unwrap();

    // Nothing on disk at all: the id exists only in the cache.
    let without_cache = qdev_core::ids_in_use(root, &storage, None).unwrap();
    assert!(!without_cache.contains("E1S1"));

    let with_cache = qdev_core::ids_in_use(root, &storage, Some(&store)).unwrap();
    assert!(with_cache.contains("E1S1"));

    // And the allocator built on it skips that id rather than colliding with the entity.
    let allocated = qdev_core::allocate_next_story_id_in(root, &storage, 1, Some(&store)).unwrap();
    assert_eq!(allocated.to_string(), "E1S2");
}

/// The off-convention check judges every file hydration reads, so a `.MD` file is judged by the
/// same rule as a `.md` one. Its name is not one the write path resolves (`filename_carries_id`
/// requires `.md`), which is exactly what the warning exists to say — and excluding `.MD` here
/// silenced the one signal that would have named it.
#[test]
fn test_off_convention_check_judges_uppercase_md_extension() {
    let store = SqliteStore::open_in_memory().unwrap();
    store
        .upsert_entity(&entity(
            "E1S2",
            EntityKind::Story,
            "docs/specs/stories/E1S2.MD",
        ))
        .unwrap();

    let findings =
        qdev_core::find_off_convention_entity_files(&store, &qdev_core::StorageConfig::default())
            .unwrap();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, "entity_file_off_convention");
    assert_eq!(findings[0].severity, "warning");
    assert_eq!(findings[0].path, "docs/specs/stories/E1S2.MD");
    let message = findings[0].message.as_deref().unwrap();
    assert!(
        message.contains("docs/specs/stories/E1S2.md"),
        "the expected name must be reported: {message}"
    );

    // A `.md` file whose name does carry its id is still silent — the fix widens which files are
    // judged, not the rule they are judged by.
    let clean = SqliteStore::open_in_memory().unwrap();
    clean
        .upsert_entity(&entity(
            "E1S2",
            EntityKind::Story,
            "docs/specs/stories/E1S2.md",
        ))
        .unwrap();
    assert!(qdev_core::find_off_convention_entity_files(
        &clean,
        &qdev_core::StorageConfig::default()
    )
    .unwrap()
    .is_empty());
}

/// Two files can carry one id in their names without either *declaring* it, and that is an
/// off-convention name rather than a collision: widening the in-use set must not widen
/// `duplicate_planning_id`.
#[test]
fn test_carried_ids_do_not_become_duplicate_findings() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    // Neither parses, so neither declares anything; both carry E1S1.
    fs::write(stories.join("E1S1.md"), "---\nid: E1S1\nbroken: [\n---\n").unwrap();
    fs::write(
        stories.join("E1S1-copy.md"),
        "---\nid: E1S1\nbroken: [\n---\n",
    )
    .unwrap();

    let storage = qdev_core::StorageConfig::default();
    let scan = qdev_core::scan_duplicate_planning_ids(root, &storage).unwrap();
    assert!(scan.groups.is_empty(), "{:?}", scan.groups);
    assert!(scan.all_ids.contains("E1S1"));
}

/// `EntityPresence::Absent` is the entire reason the helper has three states rather than being a
/// boolean, and nothing exercised it: every deferred-work fixture in the suite upserts a matching
/// entity row, so presence was always `Live` or `Stale`. Collapsing `Absent` into `Stale` — one
/// token — would silence both DW checks on a broken cache and leave the suite green.
#[test]
fn test_deferred_work_with_no_entity_row_is_still_reported() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open_in_memory().unwrap();

    // A `deferred_work` row with no `entities` row at all: a broken cache, not a stale file.
    store
        .upsert_deferred_work(&DeferredWorkRecord {
            id: "DW-c3d4".to_string(),
            origin_story_id: Some("E9S9".to_string()),
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

    for expected in ["orphan_deferred_work", "dw_missing_rationale"] {
        let finding = findings
            .iter()
            .find(|f| f.code == expected)
            .unwrap_or_else(|| {
                panic!(
                    "a DW row with no entity row must still be reported ({expected}): {:?}",
                    findings.iter().map(|f| &f.code).collect::<Vec<_>>()
                )
            });
        assert_eq!(
            finding.path, "(unknown path for DW-c3d4)",
            "with no row there is no source path, so the id is named instead"
        );
    }

    // And the three states are distinguished at the store, which is where the rule lives.
    use qdev_core::store::EntityPresence;
    assert_eq!(
        store.entity_presence_for_derivation("DW-c3d4").unwrap(),
        EntityPresence::Absent
    );
}

/// **The stale exception, made enforceable.** `ids_in_use_from_scan` reads the *unfiltered*
/// `list_entities` deliberately, and until this test nothing but a comment protected that: the
/// architecture rule "every derivation site asks the presence helper" makes filtering here look
/// like a consistency fix, and the suite stayed green when it was applied.
///
/// It is not a consistency fix, because this is a different question. The computed checks ask
/// *may a finding be derived from this row's content* — for which a stale row's pre-edit fields
/// are worthless. An allocator asks *is this id taken*, and a stale row is a hydrated entity
/// whose id is precisely the one the filesystem halves can no longer see: its file is unreadable
/// or unparseable, so it declares nothing, and here its name carries nothing either. Filter the
/// listing and the id is handed to a second entity — a duplicate minted by the tool.
///
/// Reverting the mechanism — `.filter(|e| e.exists_for_derivation())` on the listing in
/// `validate::ids_in_use_from_scan` — fails this test on both assertions.
#[test]
fn test_ids_in_use_includes_a_stale_rows_id_and_allocation_skips_it() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let storage = qdev_core::StorageConfig::default();

    // The declared and carried halves must see nothing, or the test would pass for the wrong
    // reason: the file is named so it carries no id, and its content will not parse as
    // frontmatter, so it declares none either. This is the shape a retained stale row has.
    let stories = root.join("docs/specs/stories");
    fs::create_dir_all(&stories).unwrap();
    fs::write(
        stories.join("notes-for-the-first-story.md"),
        "this was a story once; its frontmatter is gone\n",
    )
    .unwrap();

    let filesystem_only = qdev_core::ids_in_use(root, &storage, None).unwrap();
    assert!(
        !filesystem_only.contains("E1S1"),
        "fixture is wrong: the filesystem halves must not supply the id, or this test would \
         pass with the cache half filtered out entirely: {filesystem_only:?}"
    );

    let store = SqliteStore::open_in_memory().unwrap();
    let mut row = entity(
        "E1S1",
        EntityKind::Story,
        "docs/specs/stories/notes-for-the-first-story.md",
    );
    row.stale = true;
    store.upsert_entity(&row).unwrap();

    // The stale row is treated as absent for *derivation* — that rule is unchanged, and pinning
    // it here is what makes the divergence below deliberate rather than an oversight.
    assert!(
        !store
            .get_entity("E1S1")
            .unwrap()
            .unwrap()
            .exists_for_derivation(),
        "a stale row does not exist for derivation"
    );

    // ...and its id is still taken.
    let with_cache = qdev_core::ids_in_use(root, &storage, Some(&store)).unwrap();
    assert!(
        with_cache.contains("E1S1"),
        "a stale row's id is still owned by the workspace: {with_cache:?}"
    );

    // Which is the only thing an allocator was ever asking about.
    let allocated = qdev_core::allocate_next_story_id_in(root, &storage, 1, Some(&store)).unwrap();
    assert_eq!(
        allocated.to_string(),
        "E1S2",
        "allocation must skip the stale row's id rather than mint a duplicate of it"
    );
}
