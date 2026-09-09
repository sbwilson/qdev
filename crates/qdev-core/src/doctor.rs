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
        let mut map = serializer.serialize_map(Some(1 + self.fields.len()))?;
        map.serialize_entry("name", &self.name)?;
        for (key, value) in &self.fields {
            debug_assert!(
                key != "name",
                "DoctorSectionReport field key collides with the reserved \"name\" key"
            );
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

/// Reports cache health: schema version, entity count, sync freshness, and finding count.
pub struct CacheDoctorSection;

impl DoctorSection for CacheDoctorSection {
    fn name(&self) -> &'static str {
        "cache"
    }

    fn run(&self, store: &dyn Store) -> Result<DoctorSectionReport, QdevError> {
        let entity_count = store.list_entities(&EntityFilter::default())?.len();
        let last_synced_at = store.get_last_synced_at()?;
        let finding_count = store.list_findings()?.len();

        Ok(DoctorSectionReport {
            name: self.name().to_string(),
            fields: vec![
                (
                    "schema_version".to_string(),
                    serde_json::Value::from(CACHE_SCHEMA_VERSION),
                ),
                (
                    "entity_count".to_string(),
                    serde_json::Value::from(entity_count),
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
                    serde_json::Value::from(finding_count),
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
