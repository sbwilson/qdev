//! Aggregated integration test suite for `qdev-core`.
//!
//! Compiling all tests into a single test binary dramatically improves
//! compilation and link times on macOS by avoiding linking the entire
//! dependency graph dozens of times.

#[path = "architecture_tests.rs"]
mod architecture_tests;
#[path = "attribution_conformance_tests.rs"]
mod attribution_conformance_tests;
#[path = "chore_tests.rs"]
mod chore_tests;
#[path = "config_tests.rs"]
mod config_tests;
#[path = "constraint_tests.rs"]
mod constraint_tests;
#[path = "context_tests.rs"]
mod context_tests;
#[path = "dag_tests.rs"]
mod dag_tests;
#[path = "decision_tests.rs"]
mod decision_tests;
#[path = "doctor_tests.rs"]
mod doctor_tests;
#[path = "dw_tests.rs"]
mod dw_tests;
#[path = "evidence_tests.rs"]
mod evidence_tests;
#[path = "gate_adapter_tests.rs"]
mod gate_adapter_tests;
#[path = "gate_runner_tests.rs"]
mod gate_runner_tests;
#[path = "governance_tests.rs"]
mod governance_tests;
#[path = "hook_tests.rs"]
mod hook_tests;
#[path = "hygiene_tests.rs"]
mod hygiene_tests;
#[path = "id_tests.rs"]
mod id_tests;
#[path = "impact_tests.rs"]
mod impact_tests;
#[path = "init_tests.rs"]
mod init_tests;
#[path = "lease_tests.rs"]
mod lease_tests;
#[path = "modules_tests.rs"]
mod modules_tests;
#[path = "next_tests.rs"]
mod next_tests;
#[path = "preflight_tests.rs"]
mod preflight_tests;
#[path = "pulse_tests.rs"]
mod pulse_tests;
#[path = "query_tests.rs"]
mod query_tests;
#[path = "ratchet_tests.rs"]
mod ratchet_tests;
#[path = "review_tests.rs"]
mod review_tests;
#[path = "schema_tests.rs"]
mod schema_tests;
#[path = "scratch_tests.rs"]
mod scratch_tests;
#[path = "skills_tests.rs"]
mod skills_tests;
#[path = "soup_tests.rs"]
mod soup_tests;
#[path = "sprint_tests.rs"]
mod sprint_tests;
#[path = "store_tests.rs"]
mod store_tests;
#[path = "sweep_tests.rs"]
mod sweep_tests;
#[path = "transition_tests.rs"]
mod transition_tests;
#[path = "validate_tests.rs"]
mod validate_tests;
#[path = "write_tests.rs"]
mod write_tests;

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
