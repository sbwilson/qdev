//! Structured diagnostics registry for `qdev doctor`.
//!
//! A `DoctorSection` inspects one facet of workspace/cache health and reports it as a flat,
//! deterministically-ordered set of fields. `default_doctor_sections` is the single place new
//! sections are wired in; later epics (gates, leases, hygiene) append their own `DoctorSection`
//! impl there without touching the CLI dispatch or the registry mechanism itself.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::ser::SerializeMap;
use serde::Serialize;

use crate::config::{Config, LeasesConfig};
use crate::errors::QdevError;
use crate::store::sqlite::CACHE_SCHEMA_VERSION;
use crate::store::{EntityFilter, Store};
use crate::validate::run_validation;

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
    ///
    /// Convention for new sections: report health as a `status` field taking `ok` /
    /// `unavailable` (add further values as needed) plus an `unavailable_reason` carrying the
    /// error code when it is not `ok`, and never return `Err` for a condition the section can
    /// describe — `doctor` is the command a user runs when something is already broken, so a
    /// section that aborts takes the whole diagnostic down with it. (`cache` predates this and
    /// spells its own health `schema_status`; it keeps that name because the field is a
    /// published payload contract.)
    fn run(&self, store: &dyn Store) -> Result<DoctorSectionReport, QdevError>;
}

/// Reports cache health: the schema version the cache database *actually* carries next to the
/// one this binary expects, any tables missing from it, entity count, sync freshness, and
/// finding count. Reading the observed version from the database rather than echoing the
/// compiled-in constant is what lets this section detect a stale or half-migrated cache at all.
///
/// Its `finding_count` is deliberately narrow: it is the number of rows in the `findings` table,
/// i.e. the findings *hydration* recorded. It is not the answer to "is my workspace healthy?" —
/// five of `qdev validate`'s checks are computed fresh at request time and never written
/// to that table, so they cannot appear here. The `validation` section reports those.
pub struct CacheDoctorSection;

impl DoctorSection for CacheDoctorSection {
    fn name(&self) -> &'static str {
        "cache"
    }

    #[allow(clippy::disallowed_methods)] // Cache diagnostics count retained rows rather than deriving state from them.
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

/// Reports what `qdev validate` reports: the total number of findings `run_validation` computes
/// for this workspace, plus a per-code breakdown.
///
/// This exists because the `cache` section structurally cannot answer the question `doctor` is
/// asked. Five checks (`duplicate_planning_id`, `orphan_deferred_work`,
/// `dw_missing_rationale`, `target_module_not_registered`, `entity_file_off_convention`) are
/// computed fresh and never written
/// to the `findings` table, so a table read reports half the evidence — and reports `0` on a
/// workspace with real defects. Running `run_validation` keeps one definition of "what is wrong
/// with this workspace" shared with `qdev validate`, and keeps it read-only: a persisted
/// computed finding would outlive the defect it describes and be wiped by the next sweep.
///
/// `run_validation` needs a workspace root (the duplicate-id scan re-reads the files) and a
/// `Config` (the module registry), neither of which `DoctorSection::run` carries. They are held
/// here rather than added to the trait: the trait is public API that later epics implement, so
/// widening it for one section's needs would break every future implementor.
pub struct ValidationDoctorSection {
    workspace_root: PathBuf,
    config: Config,
}

impl ValidationDoctorSection {
    pub fn new(workspace_root: PathBuf, config: Config) -> Self {
        Self {
            workspace_root,
            config,
        }
    }
}

impl DoctorSection for ValidationDoctorSection {
    fn name(&self) -> &'static str {
        "validation"
    }

    fn run(&self, store: &dyn Store) -> Result<DoctorSectionReport, QdevError> {
        // `doctor` is the command a user runs when something is already wrong, so a validation
        // pass that cannot complete — a half-migrated cache missing a table one of the checks
        // reads — is reported as `status: unavailable` rather than aborting the command and
        // taking the rest of the diagnostic down with it. The error's code travels with it in
        // `unavailable_reason`: the `cache` section's `schema_status` explains a cache-shaped
        // failure, but nothing else in the payload would explain any other kind.
        //
        // This does *not* cover a check that completes while reading less than the whole
        // workspace: the duplicate-id scan skips a file or directory it cannot read and returns
        // what it found, so an unreadable specs directory reports `ok` with a count of zero.
        // `qdev validate` is blind the same way (they share the check), so the two commands
        // still agree — see `deferred-work.md`, filed against the check itself.
        let (status, unavailable_reason, finding_count, by_code) =
            match run_validation(store, &self.workspace_root, &self.config) {
                Ok(findings) => {
                    // `BTreeMap` (and `serde_json::Value::Object`'s own `BTreeMap` backing) keeps
                    // the breakdown in a stable, code-sorted order across runs.
                    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
                    for finding in &findings {
                        *counts.entry(finding.code.clone()).or_insert(0) += 1;
                    }
                    let by_code = serde_json::Value::Object(
                        counts
                            .into_iter()
                            .map(|(code, count)| (code, serde_json::Value::from(count)))
                            .collect(),
                    );
                    (
                        "ok",
                        serde_json::Value::Null,
                        serde_json::Value::from(findings.len()),
                        by_code,
                    )
                }
                Err(e) => (
                    "unavailable",
                    serde_json::Value::from(e.code()),
                    serde_json::Value::Null,
                    serde_json::Value::Null,
                ),
            };

        Ok(DoctorSectionReport {
            name: self.name().to_string(),
            fields: vec![
                ("status".to_string(), serde_json::Value::from(status)),
                ("unavailable_reason".to_string(), unavailable_reason),
                ("finding_count".to_string(), finding_count),
                ("findings_by_code".to_string(), by_code),
            ],
        })
    }
}

/// Reports story lease health: total active lease count, stale threshold, stale lease count,
/// and stale lease details for leases older than `stale_age_days`.
pub struct LeasesDoctorSection {
    workspace_root: PathBuf,
    config: LeasesConfig,
}

impl LeasesDoctorSection {
    pub fn new(workspace_root: PathBuf, config: LeasesConfig) -> Self {
        Self {
            workspace_root,
            config,
        }
    }
}

impl DoctorSection for LeasesDoctorSection {
    fn name(&self) -> &'static str {
        "leases"
    }

    fn run(&self, _store: &dyn Store) -> Result<DoctorSectionReport, QdevError> {
        match crate::lease::list_leases(&self.workspace_root) {
            Ok(leases) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                let stale_threshold_secs = (self.config.stale_age_days as i64) * 86400;

                let mut stale_leases = Vec::new();
                for lease in &leases {
                    let started_at_ts = crate::lease::parse_iso8601_to_timestamp(&lease.started_at)
                        .unwrap_or(0);
                    let age_secs = (now - started_at_ts).max(0);
                    let age_days = age_secs / 86400;
                    if age_secs >= stale_threshold_secs {
                        stale_leases.push(serde_json::json!({
                            "story_id": lease.story_id,
                            "holder": lease.holder,
                            "worktree_path": lease.worktree_path,
                            "started_at": lease.started_at,
                            "age_days": age_days,
                        }));
                    }
                }

                stale_leases.sort_by(|a, b| {
                    a["story_id"].as_str().cmp(&b["story_id"].as_str())
                });

                let active_count = leases.len();
                let stale_count = stale_leases.len();

                Ok(DoctorSectionReport {
                    name: self.name().to_string(),
                    fields: vec![
                        ("status".to_string(), serde_json::Value::from("ok")),
                        ("unavailable_reason".to_string(), serde_json::Value::Null),
                        ("active_count".to_string(), serde_json::Value::from(active_count)),
                        ("stale_count".to_string(), serde_json::Value::from(stale_count)),
                        ("stale_age_days".to_string(), serde_json::Value::from(self.config.stale_age_days)),
                        ("stale_leases".to_string(), serde_json::Value::from(stale_leases)),
                    ],
                })
            }
            Err(e) => Ok(DoctorSectionReport {
                name: self.name().to_string(),
                fields: vec![
                    ("status".to_string(), serde_json::Value::from("unavailable")),
                    ("unavailable_reason".to_string(), serde_json::Value::from(e.code())),
                    ("active_count".to_string(), serde_json::Value::Null),
                    ("stale_count".to_string(), serde_json::Value::Null),
                    ("stale_age_days".to_string(), serde_json::Value::from(self.config.stale_age_days)),
                    ("stale_leases".to_string(), serde_json::Value::Null),
                ],
            }),
        }
    }
}

/// Builds the default set of doctor sections, in the order `qdev doctor` reports them: `cache`
/// first, then `validation`, then `leases`. This stays the single wiring point — later epics append
/// their own `DoctorSection` impl here (gates, hygiene, ...) and take whatever context they need
/// from the arguments already threaded through, without widening the trait.
pub fn default_doctor_sections(
    workspace_root: &Path,
    config: &Config,
) -> Vec<Box<dyn DoctorSection>> {
    vec![
        Box::new(CacheDoctorSection),
        Box::new(ValidationDoctorSection::new(
            workspace_root.to_path_buf(),
            config.clone(),
        )),
        Box::new(LeasesDoctorSection::new(
            workspace_root.to_path_buf(),
            config.leases.clone(),
        )),
    ]
}
