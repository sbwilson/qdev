//! Structured diagnostics registry for `qdev doctor`.
//!
//! A `DoctorSection` inspects one facet of workspace/cache health and reports it as a flat,
//! deterministically-ordered set of fields. `default_doctor_sections` is the single place new
//! sections are wired in; later epics (gates, leases, hygiene) append their own `DoctorSection`
//! impl there without touching the CLI dispatch or the registry mechanism itself.

use serde::ser::SerializeMap;
use serde::Serialize;

use crate::errors::QdevError;
use crate::store::sqlite::CACHE_SCHEMA_VERSION;
use crate::store::{EntityFilter, Store};

/// One diagnostic section's report: a name plus an ordered list of `(field, value)` pairs.
/// A `Vec` rather than a `HashMap` so JSON output has deterministic field order across runs.
#[derive(Debug, Clone, PartialEq)]
pub struct DoctorSectionReport {
    pub name: String,
    pub fields: Vec<(String, serde_json::Value)>,
}

/// Serializes as a flat JSON object (`name` first, then each field in insertion order). Written
/// by hand rather than derived so the field order is exactly the `Vec` order, not alphabetized
/// the way a `serde_json::Map`/`BTreeMap` would render it without the `preserve_order` feature.
impl Serialize for DoctorSectionReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // A repeated key — whether it collides with the reserved "name" or with another field
        // of the same section — would emit a duplicate JSON key, which strict consumers reject
        // outright. `debug_assert` catches it while developing a new section; in release the
        // later colliding field is dropped rather than corrupting the object.
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        seen.insert("name");
        let emitted: Vec<&(String, serde_json::Value)> = self
            .fields
            .iter()
            .filter(|(key, _)| {
                let fresh = seen.insert(key.as_str());
                debug_assert!(
                    fresh,
                    "DoctorSectionReport field key '{}' is duplicated or collides with the \
                     reserved \"name\" key",
                    key
                );
                fresh
            })
            .collect();
        let mut map = serializer.serialize_map(Some(1 + emitted.len()))?;
        map.serialize_entry("name", &self.name)?;
        for (key, value) in emitted {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// One extensible diagnostic check contributed to `qdev doctor`.
pub trait DoctorSection {
    /// Stable, short section name (e.g. "cache").
    fn name(&self) -> &'static str;

    /// Runs the check against `store` and returns its structured report.
    fn run(&self, store: &dyn Store) -> Result<DoctorSectionReport, QdevError>;
}

/// Reports cache health: the schema version the cache database *actually* carries next to the
/// one this binary expects, any tables missing from it, entity count, sync freshness, and
/// finding count. Reading the observed version from the database rather than echoing the
/// compiled-in constant is what lets this section detect a stale or half-migrated cache at all.
pub struct CacheDoctorSection;

impl DoctorSection for CacheDoctorSection {
    fn name(&self) -> &'static str {
        "cache"
    }

    fn run(&self, store: &dyn Store) -> Result<DoctorSectionReport, QdevError> {
        let observed_schema_version = store.cache_schema_version()?;
        let missing_tables = store.cache_missing_tables()?;

        let schema_status =
            if observed_schema_version == CACHE_SCHEMA_VERSION && missing_tables.is_empty() {
                "ok"
            } else {
                "mismatch"
            };

        // The counts below read tables a half-migrated cache may not have. A diagnostic that
        // fails on a broken cache is no diagnostic at all, so a failed read is reported as
        // `null` beside the `schema_status` that explains it, rather than aborting the section.
        let entity_count = store
            .list_entities(&EntityFilter::default())
            .map(|entities| entities.len())
            .ok();
        let last_synced_at = store.get_last_synced_at().unwrap_or(None);
        let finding_count = store.list_findings().map(|findings| findings.len()).ok();

        Ok(DoctorSectionReport {
            name: self.name().to_string(),
            fields: vec![
                // Named `cache_schema_version`, not `schema_version`: the envelope already
                // has a `schema_version` (a string, the payload contract version), and a
                // section field of the same name but a different type and meaning would be a
                // trap for anything reading the payload generically.
                (
                    "cache_schema_version".to_string(),
                    serde_json::Value::from(observed_schema_version),
                ),
                (
                    "expected_cache_schema_version".to_string(),
                    serde_json::Value::from(CACHE_SCHEMA_VERSION),
                ),
                (
                    "schema_status".to_string(),
                    serde_json::Value::from(schema_status),
                ),
                (
                    "missing_tables".to_string(),
                    serde_json::Value::from(missing_tables),
                ),
                (
                    "entity_count".to_string(),
                    match entity_count {
                        Some(count) => serde_json::Value::from(count),
                        None => serde_json::Value::Null,
                    },
                ),
                (
                    "last_synced_at".to_string(),
                    match last_synced_at {
                        Some(ts) => serde_json::Value::from(ts),
                        None => serde_json::Value::Null,
                    },
                ),
                (
                    "finding_count".to_string(),
                    match finding_count {
                        Some(count) => serde_json::Value::from(count),
                        None => serde_json::Value::Null,
                    },
                ),
            ],
        })
    }
}

/// Builds the default set of doctor sections. Currently just the cache section; later epics
/// append their own `DoctorSection` impl here (gates, leases, hygiene, ...).
pub fn default_doctor_sections() -> Vec<Box<dyn DoctorSection>> {
    vec![Box::new(CacheDoctorSection)]
}
