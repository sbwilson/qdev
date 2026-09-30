//! Aggregated integration test suite for `qdev-cli`.
//!
//! Compiling all tests into a single test binary dramatically improves
//! compilation and link times on macOS by avoiding linking the entire
//! dependency graph dozens of times.

#[path = "cache_cli_tests.rs"]
mod cache_cli_tests;
#[path = "chore_cli_tests.rs"]
mod chore_cli_tests;
#[path = "cli_tests.rs"]
mod cli_tests;
#[path = "config_cli_tests.rs"]
mod config_cli_tests;
#[path = "constraint_cli_tests.rs"]
mod constraint_cli_tests;
#[path = "context_cli_tests.rs"]
mod context_cli_tests;
#[path = "create_cli_tests.rs"]
mod create_cli_tests;
#[path = "decision_cli_tests.rs"]
mod decision_cli_tests;
#[path = "doctor_cli_tests.rs"]
mod doctor_cli_tests;
#[path = "dw_cli_tests.rs"]
mod dw_cli_tests;
#[path = "evidence_cli_tests.rs"]
mod evidence_cli_tests;
#[path = "gate_baseline_cli_tests.rs"]
mod gate_baseline_cli_tests;
#[path = "gate_cli_tests.rs"]
mod gate_cli_tests;
#[path = "get_cli_tests.rs"]
mod get_cli_tests;
#[path = "graph_cli_tests.rs"]
mod graph_cli_tests;
#[path = "hook_cli_tests.rs"]
mod hook_cli_tests;
#[path = "hygiene_cli_tests.rs"]
mod hygiene_cli_tests;
#[path = "impact_cli_tests.rs"]
mod impact_cli_tests;
#[path = "init_cli_tests.rs"]
mod init_cli_tests;
#[path = "install_skills_cli_tests.rs"]
mod install_skills_cli_tests;
#[path = "lease_cli_tests.rs"]
mod lease_cli_tests;
#[path = "list_cli_tests.rs"]
mod list_cli_tests;
#[path = "modules_cli_tests.rs"]
mod modules_cli_tests;
#[path = "mcp_cli_tests.rs"]
mod mcp_cli_tests;
#[path = "network_tests.rs"]
mod network_tests;
#[path = "next_cli_tests.rs"]
mod next_cli_tests;
#[path = "preflight_cli_tests.rs"]
mod preflight_cli_tests;
#[path = "pulse_cli_tests.rs"]
mod pulse_cli_tests;
#[path = "relate_cli_tests.rs"]
mod relate_cli_tests;
#[path = "review_cli_tests.rs"]
mod review_cli_tests;
#[path = "schema_cli_tests.rs"]
mod schema_cli_tests;
#[path = "schema_payload_cli_tests.rs"]
mod schema_payload_cli_tests;
#[path = "scope_cli_tests.rs"]
mod scope_cli_tests;
#[path = "scratch_cli_tests.rs"]
mod scratch_cli_tests;
#[path = "soup_cli_tests.rs"]
mod soup_cli_tests;
#[path = "sprint_cli_tests.rs"]
mod sprint_cli_tests;
#[path = "sweep_cli_tests.rs"]
mod sweep_cli_tests;
#[path = "sync_cli_tests.rs"]
mod sync_cli_tests;
#[path = "transition_cli_tests.rs"]
mod transition_cli_tests;
#[path = "update_cli_tests.rs"]
mod update_cli_tests;
#[path = "validate_cli_tests.rs"]
mod validate_cli_tests;
#[path = "workspace_guard_cli_tests.rs"]
mod workspace_guard_cli_tests;

#[cfg(test)]
mod suite_roster {
    use std::fs;
    use std::path::Path;

    #[test]
    fn all_test_files_are_registered_in_suite() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let tests_dir = Path::new(manifest_dir).join("tests");
        let suite_source = include_str!("suite.rs");

        let mut unregistered = Vec::new();
        let entries = fs::read_dir(&tests_dir).expect("failed to read tests directory");

        for entry in entries {
            let entry = entry.expect("failed to read dir entry");
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rs") {
                let file_name = path.file_name().unwrap().to_str().unwrap();
                if file_name == "suite.rs" {
                    continue;
                }
                let expected_decl = format!("#[path = \"{}\"]", file_name);
                if !suite_source.contains(&expected_decl) {
                    unregistered.push(file_name.to_string());
                }
            }
        }

        assert!(
            unregistered.is_empty(),
            "Found test files in tests/ not registered in suite.rs: {:?}. With autotests = false, these tests will never run. Add them to tests/suite.rs!",
            unregistered
        );
    }
}
