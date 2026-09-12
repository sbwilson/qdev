//! `qdev doctor` section tests at the `Store` level (spec-doctor-sees-computed-findings).
//!
//! The broken-cache row of the spec's I/O matrix is only reachable here: every `qdev` invocation
//! in an initialized workspace runs the boot-time `ensure_cache` sweep, which rebuilds a cache
//! whose tables are missing before any command handler opens it — so the CLI can never observe
//! one. `SqliteStore::open` does not create the schema, which makes the state constructible.

use qdev_core::config::Config;
use qdev_core::store::sqlite::SqliteStore;
use qdev_core::{CacheDoctorSection, DoctorSection, ValidationDoctorSection};
use tempfile::TempDir;

fn field<'a>(report: &'a qdev_core::DoctorSectionReport, key: &str) -> &'a serde_json::Value {
    &report
        .fields
        .iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("field '{}' must be present", key))
        .1
}

/// `doctor` is the command a user runs when something is already wrong, so a validation pass it
/// cannot complete must be reported, not fatal.
#[test]
fn test_validation_section_reports_unavailable_on_a_cache_missing_its_tables() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let store = SqliteStore::open(root.join("cache.sqlite")).unwrap();

    let cache = CacheDoctorSection.run(&store).unwrap();
    assert_eq!(*field(&cache, "schema_status"), "mismatch");
    assert!(
        !field(&cache, "missing_tables")
            .as_array()
            .unwrap()
            .is_empty(),
        "the fixture's whole point is a cache with no tables: {:?}",
        cache
    );

    let validation =
        ValidationDoctorSection::new(root.to_path_buf(), Config::default()).run(&store);
    let validation = validation.expect("a failed validation must be reported, not propagated");

    assert_eq!(*field(&validation, "status"), "unavailable");
    assert!(field(&validation, "finding_count").is_null());
    assert!(field(&validation, "findings_by_code").is_null());
    // The reason travels with the status: `cache`'s `schema_status` explains a cache-shaped
    // failure, but nothing else in the payload would explain any other kind.
    assert!(
        field(&validation, "unavailable_reason").is_string(),
        "an unavailable pass must name the error code that stopped it: {:?}",
        validation
    );
}

/// The section keeps its documented field order and reports zero — not `unavailable` — for a
/// healthy but empty workspace, with no reason attached.
#[test]
fn test_validation_section_reports_zero_for_an_empty_healthy_cache() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open_in_memory().unwrap();

    let report = ValidationDoctorSection::new(temp.path().to_path_buf(), Config::default())
        .run(&store)
        .unwrap();

    assert_eq!(report.name, "validation");
    let keys: Vec<&str> = report.fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "status",
            "unavailable_reason",
            "finding_count",
            "findings_by_code"
        ]
    );
    assert_eq!(*field(&report, "status"), "ok");
    assert!(field(&report, "unavailable_reason").is_null());
    assert_eq!(*field(&report, "finding_count"), 0);
    assert_eq!(*field(&report, "findings_by_code"), serde_json::json!({}));
}

/// `default_doctor_sections` stays the single wiring point, and reports `cache` before
/// `validation` before `leases` — the order `qdev doctor --json` serializes them in.
#[test]
fn test_default_doctor_sections_order() {
    let temp = TempDir::new().unwrap();
    let sections = qdev_core::default_doctor_sections(temp.path(), &Config::default());
    let names: Vec<&str> = sections.iter().map(|s| s.name()).collect();
    assert_eq!(names, vec!["cache", "validation", "leases"]);
}
