//! SOUP audit interpretation and durable record helpers.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::config::{AnnotatedConfig, Config};
use crate::errors::QdevError;
use crate::schema::{extract_frontmatter, validate_value_detailed, EntityKind};
use crate::write::{
    acquire_workspace_write_lock, canonical_file_name, current_iso8601, directory_for_kind,
    patch_frontmatter, patch_frontmatter_removing_fields, resolve_author, resolve_entity_file,
    sha256_digest, workspace_rel_path, write_file_atomic, FrontmatterPatchOptions,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoupAuditFinding {
    pub name: String,
    pub version: String,
    pub cve_status: Option<String>,
    pub license: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoupSummary {
    pub dependencies: usize,
    pub vulnerable: usize,
}

struct PreparedSoupRecord {
    id: String,
    path: std::path::PathBuf,
    content: String,
    entity_version: u64,
    finding: SoupAuditFinding,
    original: Option<String>,
}

/// Reads only the stable cargo-audit vulnerability shape. Unknown or malformed output is ignored.
/// Result of parsing `cargo audit` JSON output.
pub struct SoupAuditParse {
    pub findings: Vec<SoupAuditFinding>,
    /// Set when the command succeeded but its output could not be interpreted as a current
    /// `cargo audit` JSON document. Such a run still exits 0, so without this note an audit
    /// that printed a legacy or otherwise unexpected format would record zero findings and
    /// look clean. Consumers surface it; the findings list stays authoritative either way.
    pub warning: Option<String>,
}

/// Parses `cargo audit` JSON output, reporting when the output is non-empty but uninterpretable.
///
/// Two failure shapes are reported separately because their remediation differs: output that is
/// not JSON at all (wrong command, wrapped output) versus JSON that simply lacks the
/// `/vulnerabilities/list` section (legacy format, `--no-vulnerabilities`, a vendored variant).
/// Empty output is neither: an audit with no findings legitimately produces an empty list.
pub fn parse_cargo_audit_json_reported(output: &str) -> SoupAuditParse {
    if output.trim().is_empty() {
        return SoupAuditParse {
            findings: Vec::new(),
            warning: None,
        };
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return SoupAuditParse {
            findings: Vec::new(),
            warning: Some(
                "audit output was not valid JSON; no findings recorded (audit output unparseable)"
                    .to_string(),
            ),
        };
    };
    let Some(entries) = value
        .pointer("/vulnerabilities/list")
        .and_then(|v| v.as_array())
    else {
        return SoupAuditParse {
            findings: Vec::new(),
            warning: Some(
                "audit JSON had no vulnerabilities/list section; no findings recorded (legacy or unexpected format)"
                    .to_string(),
            ),
        };
    };
    let findings: Vec<_> = entries
        .iter()
        .filter_map(|entry| {
            let package = entry.get("package")?;
            let name = package.get("name")?.as_str()?.to_string();
            let version = package.get("version")?.as_str()?.to_string();
            let advisory = entry.get("advisory");
            // RustSec ids are not CVE ids. Record a CVE only when cargo-audit explicitly supplies one.
            let cve_status = advisory
                .and_then(|a| a.get("aliases"))
                .and_then(|v| v.as_array())
                .map(|aliases| {
                    let mut cves: Vec<_> = aliases
                        .iter()
                        .filter_map(|v| v.as_str())
                        .filter(|v| v.starts_with("CVE-"))
                        .map(str::to_string)
                        .collect();
                    cves.sort();
                    cves.dedup();
                    cves.join(", ")
                })
                .filter(|cves| !cves.is_empty());
            let license = package
                .get("license")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            Some(SoupAuditFinding {
                name,
                version,
                cve_status,
                license,
            })
        })
        .collect();
    let mut merged: BTreeMap<(String, String), SoupAuditFinding> = BTreeMap::new();
    for finding in findings {
        let key = (finding.name.clone(), finding.version.clone());
        match merged.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(finding);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let record = entry.get_mut();
                if record.license.is_none() {
                    record.license = finding.license;
                }
                if let Some(cve) = finding.cve_status {
                    let mut cves: Vec<String> = record
                        .cve_status
                        .as_deref()
                        .unwrap_or("")
                        .split(", ")
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                    cves.extend(cve.split(", ").map(str::to_string));
                    cves.sort();
                    cves.dedup();
                    record.cve_status = Some(cves.join(", "));
                }
            }
        }
    }
    SoupAuditParse {
        findings: merged.into_values().collect(),
        warning: None,
    }
}

/// Convenience wrapper over [`parse_cargo_audit_json_reported`] for callers that only need the
/// findings. Use the `_reported` variant when a silent zero can be mistaken for a clean audit.
pub fn parse_cargo_audit_json(output: &str) -> Vec<SoupAuditFinding> {
    parse_cargo_audit_json_reported(output).findings
}

fn record_id(finding: &SoupAuditFinding) -> String {
    format!("{}-{}", finding.name, finding.version)
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

pub fn persist_soup_records(
    workspace_root: &Path,
    config: &Config,
    findings: &[SoupAuditFinding],
    release: Option<&str>,
) -> Result<SoupSummary, QdevError> {
    let _lock = acquire_workspace_write_lock(workspace_root, Some(&config.storage))?;
    let annotated = AnnotatedConfig::new(config.clone(), Default::default());
    let author = resolve_author(None, None, &annotated, workspace_root)?;
    let cache = workspace_root
        .join(&config.storage.cache_dir)
        .join("cache.sqlite");
    let mut normalized_ids = BTreeMap::new();
    for finding in findings {
        let id = record_id(finding);
        let identity = (finding.name.as_str(), finding.version.as_str());
        if let Some(previous) = normalized_ids.insert(id.clone(), identity) {
            if previous != identity {
                return Err(QdevError::usage_error(format!(
                    "SOUP dependencies '{}' and '{}@{}' normalize to the same record ID '{}'; refusing to overwrite either record",
                    format!("{}@{}", previous.0, previous.1),
                    finding.name,
                    finding.version,
                    id
                )));
            }
        }
    }
    let dir = workspace_root.join(directory_for_kind(Some(&config.storage), EntityKind::Soup));
    fs::create_dir_all(&dir)
        .map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?;
    // Prepare and schema-validate the complete batch before touching any durable record. This
    // ensures malformed later findings cannot leave an earlier finding partially persisted.
    let mut prepared = Vec::with_capacity(findings.len());
    for finding in findings {
        let id = record_id(finding);
        let file_name = canonical_file_name(&id);
        let path = dir.join(&file_name);
        let content = format!("---\nid: {:?}\nstatus: audited\nversion: 1\ncreated_by:\n  type: {:?}\n  id: {:?}\nupdated_by:\n  type: {:?}\n  id: {:?}\nname: {:?}\ndependency_version: {:?}\n{}{}{}---\n\n# {} {}\n", id, author.author_type, author.id, author.author_type, author.id, finding.name, finding.version,
            finding.license.as_ref().map(|v| format!("license: {:?}\n", v)).unwrap_or_default(),
            finding.cve_status.as_ref().map(|v| format!("cve_status: {:?}\n", v)).unwrap_or_default(),
            release.map(|v| format!("evaluated_for_release: {:?}\n", v)).unwrap_or_default(), finding.name, finding.version);
        let (written, entity_version, original) = if path.exists() {
            let existing = fs::read_to_string(&path)
                .map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?;
            let mut custom_fields = vec![
                (
                    "name".to_string(),
                    serde_yaml::Value::String(finding.name.clone()),
                ),
                (
                    "dependency_version".to_string(),
                    serde_yaml::Value::String(finding.version.clone()),
                ),
            ];
            if let Some(cve) = &finding.cve_status {
                custom_fields.push((
                    "cve_status".to_string(),
                    serde_yaml::Value::String(cve.clone()),
                ));
            }
            if let Some(license) = &finding.license {
                custom_fields.push((
                    "license".to_string(),
                    serde_yaml::Value::String(license.clone()),
                ));
            }
            if let Some(release) = release {
                custom_fields.push((
                    "evaluated_for_release".to_string(),
                    serde_yaml::Value::String(release.to_string()),
                ));
            }
            let fields_to_remove: Vec<&str> = ["license", "cve_status"]
                .into_iter()
                .filter(|field| match *field {
                    "license" => finding.license.is_none(),
                    _ => finding.cve_status.is_none(),
                })
                .collect();
            patch_frontmatter_removing_fields(
                &existing,
                &FrontmatterPatchOptions {
                    custom_fields,
                    author: Some(author.clone()),
                    ..Default::default()
                },
                &fields_to_remove,
            ).map(|(content, version)| (content, version, Some(existing)))?
        } else {
            (content, 1, None)
        };
        let fm = extract_frontmatter(&written)
            .map_err(|e| QdevError::logical_failure("schema_error", e.to_string()))?;
        validate_value_detailed(EntityKind::Soup, &fm).map_err(|e| {
            QdevError::logical_failure(
                "schema_violation",
                format!("SOUP schema validation failed: {:?}", e),
            )
        })?;
        prepared.push(PreparedSoupRecord {
            id,
            path,
            content: written,
            entity_version,
            finding: finding.clone(),
            original,
        });
    }

    let mut written = Vec::new();
    for record in &prepared {
        if let Err(error) = write_file_atomic(&record.path, &record.content) {
            rollback_records(&written)?;
            return Err(error);
        }
        written.push(record);
    }
    if let Err(error) = sync_soup_batch(&cache, workspace_root, &prepared, &author, release) {
        rollback_records(&written)?;
        return Err(error);
    }
    Ok(SoupSummary {
        dependencies: findings.len(),
        vulnerable: findings.iter().filter(|f| f.cve_status.is_some()).count(),
    })
}

fn rollback_records(records: &[&PreparedSoupRecord]) -> Result<(), QdevError> {
    for record in records.iter().rev() {
        match &record.original { Some(content) => write_file_atomic(&record.path, content)?, None => { if record.path.exists() { fs::remove_file(&record.path).map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?; } } }
    }
    Ok(())
}

fn sync_soup_batch(cache: &Path, workspace_root: &Path, records: &[PreparedSoupRecord], author: &crate::write::Author, release: Option<&str>) -> Result<(), QdevError> {
    if !cache.exists() { return Ok(()); }
    let store = crate::store::SqliteStore::open(cache)?;
    store.with_conn_mut(|conn| {
        let tx = conn.transaction().map_err(|e| QdevError::infrastructure_failure("sqlite_error", e.to_string()))?;
        for record in records {
            tx.execute("INSERT INTO entities (id, kind, title, status, source_path, content_hash, version, created_by_type, created_by_id, updated_by_type, updated_by_id, updated_at, stale) VALUES (?1, 'soup', ?2, 'audited', ?3, ?4, ?5, ?6, ?7, ?6, ?7, ?8, 0) ON CONFLICT(id) DO UPDATE SET title=excluded.title,status=excluded.status,source_path=excluded.source_path,content_hash=excluded.content_hash,version=excluded.version,updated_by_type=excluded.updated_by_type,updated_by_id=excluded.updated_by_id,updated_at=excluded.updated_at,stale=0", rusqlite::params![record.id, format!("{} {}", record.finding.name, record.finding.version), workspace_rel_path(&record.path, workspace_root), sha256_digest(record.content.as_bytes()), record.entity_version, author.author_type, author.id, current_iso8601()]).map_err(|e| QdevError::infrastructure_failure("sqlite_error", e.to_string()))?;
            tx.execute("INSERT INTO soup_dependencies (id, name, version, license, cve_status, introduced_by_story, evaluated_for_release) VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6) ON CONFLICT(id) DO UPDATE SET name=excluded.name,version=excluded.version,license=excluded.license,cve_status=excluded.cve_status,introduced_by_story=NULL,evaluated_for_release=excluded.evaluated_for_release", rusqlite::params![record.id, record.finding.name, record.finding.version, record.finding.license, record.finding.cve_status, release]).map_err(|e| QdevError::infrastructure_failure("sqlite_error", e.to_string()))?;
            tx.execute("INSERT INTO dirty_entities (id, dirty_at) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET dirty_at=excluded.dirty_at", rusqlite::params![record.id, current_iso8601()]).map_err(|e| QdevError::infrastructure_failure("sqlite_error", e.to_string()))?;
            tx.execute("DELETE FROM sync_state WHERE path = ?1", rusqlite::params![workspace_rel_path(&record.path, workspace_root)]).map_err(|e| QdevError::infrastructure_failure("sqlite_error", e.to_string()))?;
        }
        tx.commit().map_err(|e| QdevError::infrastructure_failure("sqlite_error", e.to_string()))
    })
}

pub fn record_sbom_artifact(
    workspace_root: &Path,
    config: &Config,
    release: &str,
    artifact: &str,
) -> Result<(), QdevError> {
    let artifact_path = Path::new(artifact);
    if artifact.trim().is_empty()
        || artifact_path.is_absolute()
        || artifact_path.components().any(|component| component == Component::ParentDir)
    {
        return Err(QdevError::usage_error(format!(
            "SBOM artifact '{}' must be a workspace-relative path without parent traversal",
            artifact
        )));
    }
    let candidate = workspace_root.join(artifact_path);
    if !candidate.is_file() {
        return Err(QdevError::usage_error(format!(
            "SBOM artifact '{}' was not produced",
            artifact
        )));
    }
    let canonical_root = workspace_root
        .canonicalize()
        .map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?;
    let canonical_artifact = candidate
        .canonicalize()
        .map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?;
    if !canonical_artifact.starts_with(&canonical_root) {
        return Err(QdevError::usage_error(format!(
            "SBOM artifact '{}' resolves outside the workspace",
            artifact
        )));
    }
    let _lock = acquire_workspace_write_lock(workspace_root, Some(&config.storage))?;
    let (_, _, path) = resolve_entity_file(
        workspace_root,
        Some(EntityKind::Release),
        release,
        Some(&config.storage),
    )?;
    let content = fs::read_to_string(&path)
        .map_err(|e| QdevError::infrastructure_failure("io_error", e.to_string()))?;
    let annotated = AnnotatedConfig::new(config.clone(), Default::default());
    let author = resolve_author(None, None, &annotated, workspace_root)?;
    let (patched, _) = patch_frontmatter(
        &content,
        &FrontmatterPatchOptions {
            custom_fields: vec![(
                "sbom_artifact_path".to_string(),
                serde_yaml::Value::String(artifact.to_string()),
            )],
            author: Some(author),
            ..Default::default()
        },
    )?;
    let fm = extract_frontmatter(&patched)
        .map_err(|e| QdevError::logical_failure("schema_error", e.to_string()))?;
    validate_value_detailed(EntityKind::Release, &fm).map_err(|e| {
        QdevError::logical_failure(
            "schema_violation",
            format!("Release schema validation failed: {:?}", e),
        )
    })?;
    write_file_atomic(&path, &patched)
}
