//! Every command that reads or writes the cache must refuse to run outside an initialized
//! workspace, with a clean `usage_error` and without creating a stray cache file.
//!
//! `SqliteStore::open` creates `cache.sqlite` (and its parent directory) on demand, so a command
//! that reaches the store outside a workspace litters the directory with an empty, schema-less
//! database and then fails with a raw `sqlite_error: no such table` — exit 4 instead of exit 2.
//! The guard lives in one place (`requires_workspace` in the CLI dispatch); this file pins every
//! arm of it, so flipping one back cannot pass unnoticed.

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// Commands that must be refused outside a workspace, each with a minimal valid argument list.
fn guarded_invocations() -> Vec<Vec<&'static str>> {
    vec![
        vec!["get", "story", "E1S1"],
        vec!["list", "stories"],
        vec!["update", "story", "E1S1", "--status", "ready"],
        vec!["relate", "E1S1", "depends_on", "E1S2"],
        vec!["unrelate", "E1S1", "depends_on", "E1S2"],
        vec!["graph", "--dot"],
        vec!["validate"],
        vec!["sync"],
        vec!["doctor"],
        vec!["claim", "story", "E1S1"],
        vec!["release", "story", "E1S1"],
        vec!["constraint", "add", "E1S1", "--kind", "no_go", "text"],
    ]
}

#[test]
fn test_cache_touching_commands_are_refused_outside_a_workspace() {
    for args in guarded_invocations() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        // Deliberately no `qdev init`.

        let mut full = args.clone();
        full.push("--json");
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        let assert = cmd.current_dir(root).args(&full).assert().failure().code(2);

        let val: Value = serde_json::from_slice(&assert.get_output().stdout)
            .unwrap_or_else(|e| panic!("`qdev {}` must emit a JSON error: {}", args.join(" "), e));
        assert_eq!(
            val["error"]["code"],
            "usage_error",
            "`qdev {}` outside a workspace must be a usage error",
            args.join(" ")
        );
        assert!(
            !root.join(".qdev/cache/cache.sqlite").exists(),
            "`qdev {}` must not create a cache file outside a workspace",
            args.join(" ")
        );
    }
}

/// The deliberate exemptions. `create story` bootstraps into a clean directory (it allocates the
/// first id and creates the cache as it goes), and `status` / `config show` / `schema` report on
/// whatever they find. These are specified behaviour, not gaps in the guard.
#[test]
fn test_bootstrapping_commands_still_work_outside_a_workspace() {
    for args in [
        vec!["create", "story", "E1"],
        vec!["status"],
        vec!["config", "show"],
        vec!["schema", "story"],
    ] {
        let temp = TempDir::new().unwrap();
        let mut cmd = Command::cargo_bin("qdev").unwrap();
        cmd.current_dir(temp.path()).args(&args).assert().success();
    }
}
