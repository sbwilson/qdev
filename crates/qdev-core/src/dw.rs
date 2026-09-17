//! Deferred work debt and residual anomaly management per ISO 14971 (Story 2.8).
//!
//! Records are stored in `docs/state/dw/<dw-id>.md`, strictly validated against
//! `schemas/dw.json` (`EntityKind::DeferredWork`), written atomically under the workspace
//! advisory write lock, and synchronized immediately to the SQLite cache (`entities` and
//! `deferred_work` tables).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{Config, StorageConfig};
use crate::errors::QdevError;
use crate::id::allocate_deferred_work_id_in_with_rng;
use crate::schema::{validate_value_detailed, EntityKind};
use crate::store::{DeferredWorkRecord, EntityRecord, SqliteStore, Store};
use crate::write::{
    acquire_workspace_write_lock, apply_entity_update, canonical_file_name, current_iso8601,
    directory_for_kind, resolve_entity_file, sha256_digest, upsert_cache_and_mark_dirty,
    workspace_rel_path, write_file_atomic, Author, EntityUpdateOptions,
};

/// Allowed `safety_risk` levels for deferred work frontmatter and records conforming to ISO 14971.
pub const VALID_SAFETY_RISKS: &[&str] =
    &["negligible", "acceptable_with_mitigation", "unacceptable"];

/// Validates that `risk` matches one of the ISO 14971 allowed risk levels.
pub fn validate_safety_risk(risk: &str) -> Result<(), QdevError> {
    let trimmed = risk.trim();
    if !VALID_SAFETY_RISKS.contains(&trimmed) {
        return Err(QdevError::usage_error(format!(
            "Invalid safety risk '{}'; valid risk levels are: {}",
            trimmed,
            VALID_SAFETY_RISKS.join(", ")
        )));
    }
    Ok(())
}

/// Checks whether a rationale is required for a given safety risk under the regulatory configuration.
pub fn is_rationale_required(risk: &str, require_rationale_for: &[String]) -> bool {
    let trimmed = risk.trim();
    if !require_rationale_for.is_empty() {
        require_rationale_for.iter().any(|r| r == trimmed)
    } else {
        trimmed != "negligible"
    }
}

/// Input parameters for creating a deferred work record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddDeferredWorkInput {
    pub title: String,
    pub target_module: String,
    pub safety_risk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_story_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<String>,
    pub author: Author,
}

/// Input parameters for closing a deferred work record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseDeferredWorkInput {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    pub author: Author,
}

/// Filter criteria for querying deferred work records.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListDeferredWorkFilter {
    pub module: Option<String>,
    pub risk: Option<String>,
    pub status: Option<String>,
    pub story: Option<String>,
}

/// JSON payload shape for created or updated deferred work records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredWorkPayload {
    pub id: String,
    pub title: String,
    pub status: String,
    pub target_module: String,
    pub safety_risk: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_story_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<String>,
    pub version: u64,
    pub created_by: Author,
    pub updated_by: Author,
    pub path: String,
}

/// A projected deferred work item in listing output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeferredWorkItem {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub target_module: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_risk: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_story_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<String>,
    pub version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Payload envelope for `qdev dw list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListDeferredWorkPayload {
    pub items: Vec<DeferredWorkItem>,
}

/// Adds a schema-validated deferred work record under the workspace advisory write lock,
/// atomically writes `docs/state/dw/DW-hhhh.md`, and synchronizes SQLite cache.
pub fn add_deferred_work(
    workspace_root: &Path,
    config: &Config,
    input: &AddDeferredWorkInput,
) -> Result<DeferredWorkPayload, QdevError> {
    add_deferred_work_with_store(workspace_root, config, input, None)
}

/// Adds a deferred work record with an optional pre-opened `Store` reference.
pub fn add_deferred_work_with_store(
    workspace_root: &Path,
    config: &Config,
    input: &AddDeferredWorkInput,
    opt_store: Option<&dyn Store>,
) -> Result<DeferredWorkPayload, QdevError> {
    // 1. Validate author
    input.author.validate()?;

    // 2. Validate title
    let trimmed_title = input.title.trim();
    if trimmed_title.is_empty() {
        return Err(QdevError::usage_error("Title cannot be empty"));
    }

    // 3. Validate safety risk
    let trimmed_risk = input.safety_risk.trim();
    validate_safety_risk(trimmed_risk)?;

    // 4. Validate target module against config.modules
    let trimmed_module = input.target_module.trim();
    if trimmed_module.is_empty() {
        return Err(QdevError::usage_error("Target module cannot be empty"));
    }
    if !config.modules.iter().any(|m| m.id == trimmed_module) {
        return Err(QdevError::usage_error(format!(
            "Module '{}' is not registered in configuration",
            trimmed_module
        )));
    }

    // 5. Validate origin story if specified
    if let Some(ref story_id) = input.origin_story_id {
        let trimmed_story = story_id.trim();
        if trimmed_story.is_empty() {
            return Err(QdevError::usage_error("Origin story ID cannot be empty"));
        }
        if resolve_entity_file(
            workspace_root,
            Some(EntityKind::Story),
            trimmed_story,
            Some(&config.storage),
        )
        .is_err()
        {
            return Err(QdevError::usage_error_with_code(
                "entity_not_found",
                format!("Origin story '{}' not found in workspace", trimmed_story),
            ));
        }
    }

    // 6. Enforce rationale gate based on regulatory configuration
    let trimmed_rationale = input
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let rationale_needed =
        is_rationale_required(trimmed_risk, &config.regulatory.require_rationale_for);

    if rationale_needed && trimmed_rationale.is_none() {
        return Err(QdevError::logical_failure(
            "regulatory.require_rationale_for",
            format!(
                "Safety risk '{}' requires a rationale per regulatory configuration (regulatory.require_rationale_for)",
                trimmed_risk
            ),
        ));
    }

    // 7. Acquire workspace advisory write lock
    let _lock_guard = acquire_workspace_write_lock(workspace_root, Some(&config.storage))?;

    // 8. Store / cache reference
    let cache_dir_rel = config.storage.cache_dir.as_str();
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");
    let owned_store = if opt_store.is_none() && cache_db_path.is_file() {
        Some(SqliteStore::open(&cache_db_path)?)
    } else {
        None
    };
    let store_ref: Option<&dyn Store> = opt_store.or(owned_store.as_ref().map(|s| s as &dyn Store));

    // 9. Allocate DW-hhhh identifier
    let mut rng = rand::rng();
    let dw_ident = allocate_deferred_work_id_in_with_rng(
        workspace_root,
        &config.storage,
        &mut rng,
        store_ref,
    )?;
    let dw_id = dw_ident.to_string();
    let timestamp = current_iso8601();

    // 10. Build frontmatter JSON
    let mut frontmatter_map = serde_json::Map::new();
    frontmatter_map.insert("id".to_string(), serde_json::json!(dw_id));
    frontmatter_map.insert("title".to_string(), serde_json::json!(trimmed_title));
    frontmatter_map.insert("status".to_string(), serde_json::json!("open"));
    frontmatter_map.insert("version".to_string(), serde_json::json!(1));
    frontmatter_map.insert(
        "created_by".to_string(),
        serde_json::json!({
            "type": input.author.author_type,
            "id": input.author.id,
        }),
    );
    frontmatter_map.insert(
        "updated_by".to_string(),
        serde_json::json!({
            "type": input.author.author_type,
            "id": input.author.id,
        }),
    );
    frontmatter_map.insert(
        "target_module".to_string(),
        serde_json::json!(trimmed_module),
    );
    if let Some(ref story_id) = input.origin_story_id {
        frontmatter_map.insert(
            "origin_story_id".to_string(),
            serde_json::json!(story_id.trim()),
        );
    }
    frontmatter_map.insert("safety_risk".to_string(), serde_json::json!(trimmed_risk));
    if let Some(r) = trimmed_rationale {
        frontmatter_map.insert("rationale".to_string(), serde_json::json!(r));
    }
    if let Some(ref g) = input.gate {
        let trimmed_g = g.trim();
        if !trimmed_g.is_empty() {
            frontmatter_map.insert("gate".to_string(), serde_json::json!(trimmed_g));
        }
    }
    if !input.owners.is_empty() {
        frontmatter_map.insert("owners".to_string(), serde_json::json!(input.owners));
    }

    let frontmatter_json = serde_json::Value::Object(frontmatter_map);

    // 11. Schema validation against EntityKind::DeferredWork
    validate_value_detailed(EntityKind::DeferredWork, &frontmatter_json).map_err(|errs| {
        QdevError::logical_failure(
            "schema_violation",
            format!(
                "Deferred work frontmatter schema validation failed: {}",
                errs.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        )
    })?;

    // 12. Serialize frontmatter YAML
    let frontmatter_yaml = serde_yaml::to_string(&frontmatter_json).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize deferred work frontmatter: {}", e),
        )
    })?;

    // 13. Format markdown body
    let dw_content = format!("---\n{}---\n\n# {}\n", frontmatter_yaml, trimmed_title);

    // 14. Write atomic file under docs/state/dw/
    let rel_dir = directory_for_kind(Some(&config.storage), EntityKind::DeferredWork);
    let file_name = canonical_file_name(&dw_id);
    let dw_file_path = workspace_root.join(&rel_dir).join(&file_name);

    write_file_atomic(&dw_file_path, &dw_content)?;

    // 15. Upsert cache for both entities and deferred_work tables
    let rel_source_path = workspace_rel_path(&dw_file_path, workspace_root);
    let content_hash = sha256_digest(dw_content.as_bytes());

    let target_modules_json = serde_json::to_string(&vec![trimmed_module.to_string()]).ok();
    let owners_json = if input.owners.is_empty() {
        None
    } else {
        serde_json::to_string(&input.owners).ok()
    };

    let entity_record = EntityRecord {
        id: dw_id.clone(),
        kind: EntityKind::DeferredWork,
        title: Some(trimmed_title.to_string()),
        status: Some("open".to_string()),
        owners: owners_json,
        source_path: rel_source_path.clone(),
        content_hash,
        version: 1,
        created_by: Some(input.author.clone()),
        updated_by: Some(input.author.clone()),
        updated_at: timestamp,
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: target_modules_json,
    };

    let dw_record = DeferredWorkRecord {
        id: dw_id.clone(),
        origin_story_id: input.origin_story_id.as_ref().map(|s| s.trim().to_string()),
        target_module: trimmed_module.to_string(),
        status: Some("open".to_string()),
        safety_risk: Some(trimmed_risk.to_string()),
        rationale: trimmed_rationale.map(str::to_string),
        gate: input
            .gate
            .as_ref()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty()),
        resolution: None,
    };

    if cache_db_path.is_file() {
        upsert_cache_and_mark_dirty(&cache_db_path, &entity_record)?;
    } else if let Some(store) = store_ref {
        store.upsert_entity(&entity_record)?;
    }

    if let Some(store) = store_ref {
        store.upsert_deferred_work(&dw_record)?;
    } else if cache_db_path.is_file() {
        let store = SqliteStore::open(&cache_db_path)?;
        store.upsert_deferred_work(&dw_record)?;
    }

    Ok(DeferredWorkPayload {
        id: dw_id,
        title: trimmed_title.to_string(),
        status: "open".to_string(),
        target_module: trimmed_module.to_string(),
        safety_risk: trimmed_risk.to_string(),
        origin_story_id: input.origin_story_id.as_ref().map(|s| s.trim().to_string()),
        rationale: trimmed_rationale.map(str::to_string),
        gate: input
            .gate
            .as_ref()
            .map(|g| g.trim().to_string())
            .filter(|g| !g.is_empty()),
        resolution: None,
        owners: input.owners.clone(),
        version: 1,
        created_by: input.author.clone(),
        updated_by: input.author.clone(),
        path: rel_source_path,
    })
}

/// Closes an existing deferred work record with status `done` or `wont_fix`,
/// updating frontmatter and synchronizing SQLite cache.
pub fn close_deferred_work(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    input: &CloseDeferredWorkInput,
) -> Result<DeferredWorkPayload, QdevError> {
    // 1. Validate author
    input.author.validate()?;

    // 2. Validate DW id
    let trimmed_id = input.id.trim();
    if trimmed_id.is_empty() {
        return Err(QdevError::usage_error("Deferred work ID cannot be empty"));
    }

    let (_, canonical_id, _) = match resolve_entity_file(
        workspace_root,
        Some(EntityKind::DeferredWork),
        trimmed_id,
        storage,
    ) {
        Ok(res) => res,
        Err(_) => {
            return Err(QdevError::usage_error_with_code(
                "entity_not_found",
                format!("Deferred work entity '{}' not found", trimmed_id),
            ));
        }
    };

    // 3. Validate close status (defaults to "done")
    let target_status = input
        .status
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("done");

    if target_status != "done" && target_status != "wont_fix" {
        return Err(QdevError::usage_error(format!(
            "Invalid close status '{}'; allowed statuses are 'done' or 'wont_fix'",
            target_status
        )));
    }

    // 4. Validate justification / resolution for wont_fix
    let resolution_val = match (input.justification.as_deref(), input.resolution.as_deref()) {
        (Some(j), _) if !j.trim().is_empty() => Some(j.trim().to_string()),
        (_, Some(r)) if !r.trim().is_empty() => Some(r.trim().to_string()),
        _ => None,
    };

    if target_status == "wont_fix" && resolution_val.is_none() {
        return Err(QdevError::usage_error(
            "Closing deferred work as 'wont_fix' requires a non-empty --justification (or --resolution)",
        ));
    }

    // 5. Apply entity update
    let mut custom_fields = Vec::new();
    if let Some(ref res) = resolution_val {
        custom_fields.push((
            "resolution".to_string(),
            serde_yaml::Value::String(res.clone()),
        ));
    }

    let opts = EntityUpdateOptions {
        workspace_root: workspace_root.to_path_buf(),
        storage: storage.cloned(),
        entity_kind: Some(EntityKind::DeferredWork),
        entity_id: canonical_id.clone(),
        status: Some(target_status.to_string()),
        title: None,
        custom_fields,
        section: None,
        section_file: None,
        if_version: None,
        author: input.author.clone(),
    };

    let update_res = apply_entity_update(&opts)?;
    let fm = &update_res.updated_frontmatter;

    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let target_module = fm
        .get("target_module")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let safety_risk = fm
        .get("safety_risk")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let origin_story_id = fm
        .get("origin_story_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let rationale = fm
        .get("rationale")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let gate = fm.get("gate").and_then(|v| v.as_str()).map(str::to_string);
    let resolution = fm
        .get("resolution")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let owners = fm
        .get("owners")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();
    let created_by = fm
        .get("created_by")
        .and_then(|v| serde_json::from_value::<Author>(v.clone()).ok())
        .unwrap_or_else(|| input.author.clone());

    Ok(DeferredWorkPayload {
        id: canonical_id,
        title,
        status: target_status.to_string(),
        target_module,
        safety_risk,
        origin_story_id,
        rationale,
        gate,
        resolution,
        owners,
        version: update_res.new_version,
        created_by,
        updated_by: input.author.clone(),
        path: update_res.rel_path,
    })
}

/// Lists deferred work records matching the given filter.
#[allow(clippy::disallowed_methods)] // list_deferred_work_records is a reporting path and must surface stale rows.
pub fn list_deferred_work_records(
    store: &dyn Store,
    filter: &ListDeferredWorkFilter,
) -> Result<Vec<DeferredWorkItem>, QdevError> {
    let records = store.list_deferred_work()?;
    let mut items = Vec::new();

    for rec in records {
        if let Some(ref m) = filter.module {
            if rec.target_module != *m {
                continue;
            }
        }
        if let Some(ref r) = filter.risk {
            if rec.safety_risk.as_deref() != Some(r.as_str()) {
                continue;
            }
        }
        if let Some(ref s) = filter.status {
            if rec.status.as_deref() != Some(s.as_str()) {
                continue;
            }
        }
        if let Some(ref st) = filter.story {
            if rec.origin_story_id.as_deref() != Some(st.as_str()) {
                continue;
            }
        }

        let entity = store.get_entity(&rec.id)?;
        let (title, version, owners, path) = match entity {
            Some(e) => {
                let owners = e
                    .owners
                    .as_deref()
                    .and_then(|o| serde_json::from_str::<Vec<String>>(o).ok())
                    .unwrap_or_default();
                (e.title, e.version, owners, Some(e.source_path))
            }
            None => (None, 1, Vec::new(), None),
        };

        items.push(DeferredWorkItem {
            id: rec.id,
            title,
            status: rec.status,
            target_module: rec.target_module,
            safety_risk: rec.safety_risk,
            origin_story_id: rec.origin_story_id,
            rationale: rec.rationale,
            gate: rec.gate,
            resolution: rec.resolution,
            owners,
            version,
            path,
        });
    }

    Ok(items)
}
