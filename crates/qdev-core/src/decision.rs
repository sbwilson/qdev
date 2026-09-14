//! Unified decision logging domain logic.
//!
//! Every decision record (`DEC-hhhh`) committed to `docs/state/decisions/` is schema-validated,
//! written atomically under the workspace advisory write lock, and synchronized to the SQLite
//! cache (`entities` and `decisions` tables).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::id::allocate_decision_id_in_with_rng;
use crate::schema::{validate_value_detailed, EntityKind};
use crate::store::{DecisionRecord, EntityRecord, SqliteStore, Store};
use crate::write::{
    acquire_workspace_write_lock, canonical_file_name, current_iso8601, directory_for_kind,
    resolve_entity_file, sha256_digest, upsert_cache_and_mark_dirty, workspace_rel_path,
    write_file_atomic, Author,
};

/// Allowed `decision_type` values for decision frontmatter and records.
pub const VALID_DECISION_TYPES: &[&str] = &[
    "human_ruling",
    "agent_assumption",
    "cross_team_override",
    "pivot",
    "review_rejection",
    "lease_override",
];

fn default_true() -> bool {
    true
}

/// Input parameters for logging a decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionInput {
    pub subject_id: String,
    pub decision_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    pub ruling: String,
    pub author: Author,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default = "default_true")]
    pub validate_subject: bool,
}

/// JSON payload shape for `qdev decision log` and domain return value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionLogPayload {
    pub id: String,
    pub subject_id: String,
    pub decision_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub context: String,
    pub ruling: String,
    pub author: Author,
    pub created_at: String,
    pub path: String,
}

/// Logs a schema-validated decision under the workspace advisory write lock,
/// atomically writes `docs/state/decisions/<dec-id>.md`, and updates the SQLite cache.
pub fn log_decision(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    input: &DecisionInput,
) -> Result<DecisionLogPayload, QdevError> {
    log_decision_with_store(workspace_root, storage, input, None)
}

/// Logs a decision with an optional pre-opened `Store` reference.
pub fn log_decision_with_store(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    input: &DecisionInput,
    opt_store: Option<&dyn Store>,
) -> Result<DecisionLogPayload, QdevError> {
    // 1. Validate author
    input.author.validate()?;

    // 2. Validate decision_type
    let trimmed_type = input.decision_type.trim();
    if !VALID_DECISION_TYPES.contains(&trimmed_type) {
        return Err(QdevError::usage_error(format!(
            "Invalid decision type '{}'; allowed types are: {}",
            trimmed_type,
            VALID_DECISION_TYPES.join(", ")
        )));
    }

    // 3. Validate ruling
    let trimmed_ruling = input.ruling.trim();
    if trimmed_ruling.is_empty() {
        return Err(QdevError::usage_error(
            "Decision ruling cannot be empty or whitespace-only",
        ));
    }

    // 4. Validate subject entity existence in workspace
    let trimmed_subject = input.subject_id.trim();
    if trimmed_subject.is_empty() {
        return Err(QdevError::usage_error("Subject entity ID cannot be empty"));
    }

    let canonical_subject_id = if input.validate_subject {
        let (_, id, _) = resolve_entity_file(workspace_root, None, trimmed_subject, storage)?;
        id
    } else {
        trimmed_subject.to_string()
    };

    // 5. Derive context (default: "Decision on <subject_id>")
    let context_str = match &input.context {
        Some(c) if !c.trim().is_empty() => c.trim().to_string(),
        _ => format!("Decision on {}", canonical_subject_id),
    };

    // 6. Derive title (from input.title, input.topic, or decision_type)
    let title = match &input.title {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => match &input.topic {
            Some(top) if !top.trim().is_empty() => top.trim().to_string(),
            _ => match trimmed_type {
                "cross_team_override" => {
                    format!("Cross-team override on {}", canonical_subject_id)
                }
                "lease_override" => format!("Lease override on {}", canonical_subject_id),
                "review_rejection" => format!("Review rejection on {}", canonical_subject_id),
                "pivot" => format!("Pivot on {}", canonical_subject_id),
                other => format!("Decision ({}) on {}", other, canonical_subject_id),
            },
        },
    };

    // 7. Acquire workspace advisory write lock
    let _lock_guard = acquire_workspace_write_lock(workspace_root, storage)?;

    // 8. Determine store / cache availability
    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    let cache_dir_rel = st.cache_dir.as_str();
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");
    let owned_store = if opt_store.is_none() && cache_db_path.is_file() {
        Some(SqliteStore::open(&cache_db_path)?)
    } else {
        None
    };
    let store_ref: Option<&dyn Store> = opt_store.or(owned_store.as_ref().map(|s| s as &dyn Store));

    // 9. Allocate DEC-hhhh identifier
    let mut rng = rand::rng();
    let dec_ident = allocate_decision_id_in_with_rng(workspace_root, st, &mut rng, store_ref)?;
    let dec_id = dec_ident.to_string();

    let timestamp = input
        .timestamp
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(current_iso8601);

    // 10. Build frontmatter JSON
    let mut frontmatter_map = serde_json::Map::new();
    frontmatter_map.insert("id".to_string(), serde_json::json!(dec_id));
    frontmatter_map.insert("title".to_string(), serde_json::json!(title));
    frontmatter_map.insert("status".to_string(), serde_json::json!("active"));
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
        "subject_id".to_string(),
        serde_json::json!(canonical_subject_id),
    );
    frontmatter_map.insert(
        "decision_type".to_string(),
        serde_json::json!(trimmed_type),
    );
    if let Some(ref top) = input.topic {
        let trimmed_topic = top.trim();
        if !trimmed_topic.is_empty() {
            frontmatter_map.insert("topic".to_string(), serde_json::json!(trimmed_topic));
        }
    }
    frontmatter_map.insert("context".to_string(), serde_json::json!(context_str));
    frontmatter_map.insert("ruling".to_string(), serde_json::json!(trimmed_ruling));
    frontmatter_map.insert("created_at".to_string(), serde_json::json!(timestamp));

    let frontmatter_json = serde_json::Value::Object(frontmatter_map);

    // 11. Schema validation against EntityKind::Decision (decision.json)
    validate_value_detailed(EntityKind::Decision, &frontmatter_json).map_err(|errs| {
        QdevError::logical_failure(
            "schema_violation",
            format!(
                "Decision frontmatter schema validation failed: {}",
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
            format!("Failed to serialize decision frontmatter: {}", e),
        )
    })?;

    // 13. Format markdown body
    let dec_content = if trimmed_type == "pivot" || trimmed_type == "review_rejection" {
        format!(
            "---\n{}---\n\n# {}\n\nTransition: {}\n\n{}\n",
            frontmatter_yaml, title, context_str, trimmed_ruling
        )
    } else {
        format!(
            "---\n{}---\n\n# {}\n\nContext: {}\n\n{}\n",
            frontmatter_yaml, title, context_str, trimmed_ruling
        )
    };

    // 14. Write atomic file under docs/state/decisions/
    let rel_dir = directory_for_kind(storage, EntityKind::Decision);
    let file_name = canonical_file_name(&dec_id);
    let dec_file_path = workspace_root.join(&rel_dir).join(&file_name);

    write_file_atomic(&dec_file_path, &dec_content)?;

    // 15. Upsert cache if available
    let rel_source_path = workspace_rel_path(&dec_file_path, workspace_root);
    let content_hash = sha256_digest(dec_content.as_bytes());

    let decision_entity_record = EntityRecord {
        id: dec_id.clone(),
        kind: EntityKind::Decision,
        title: Some(title),
        status: Some("active".to_string()),
        owners: None,
        source_path: rel_source_path.clone(),
        content_hash,
        version: 1,
        created_by: Some(input.author.clone()),
        updated_by: Some(input.author.clone()),
        updated_at: timestamp.clone(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    };

    let decision_record = DecisionRecord {
        id: dec_id.clone(),
        subject_id: canonical_subject_id.clone(),
        decision_type: Some(trimmed_type.to_string()),
        topic: input
            .topic
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty()),
        context: Some(context_str.clone()),
        ruling: Some(trimmed_ruling.to_string()),
        author_type: Some(input.author.author_type.clone()),
        author_id: Some(input.author.id.clone()),
        created_at: Some(timestamp.clone()),
    };

    if cache_db_path.is_file() {
        upsert_cache_and_mark_dirty(&cache_db_path, &decision_entity_record)?;
        if let Some(store) = store_ref {
            store.upsert_decision(&decision_record)?;
        }
    } else if let Some(store) = store_ref {
        store.upsert_entity(&decision_entity_record)?;
        store.upsert_decision(&decision_record)?;
    }

    Ok(DecisionLogPayload {
        id: dec_id,
        subject_id: canonical_subject_id,
        decision_type: trimmed_type.to_string(),
        topic: input
            .topic
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty()),
        context: context_str,
        ruling: trimmed_ruling.to_string(),
        author: input.author.clone(),
        created_at: timestamp,
        path: rel_source_path,
    })
}
