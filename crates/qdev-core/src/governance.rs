use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::{Config, StorageConfig};
use crate::errors::QdevError;
use crate::id::{allocate_decision_id_in_with_rng, Identifier};
use crate::lease::{find_workspace_leases, get_lease, StoryLease};
use crate::schema::{extract_frontmatter, validate_frontmatter, validate_value_detailed, EntityKind};
use crate::store::{DecisionRecord, EntityRecord, SqliteStore, Store};
use crate::write::{
    acquire_workspace_write_lock, canonical_file_name, current_iso8601, directory_for_kind,
    kind_for_write, patch_frontmatter, resolve_entity_file, sha256_digest, story_detail_fields,
    upsert_cache_and_mark_dirty, workspace_rel_path, write_file_atomic, Author,
    FrontmatterPatchOptions,
};

/// Scope and governance classification result for an entity mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeClassification {
    pub is_out_of_lease: bool,
    pub is_cross_team: bool,
    pub active_lease: Option<StoryLease>,
    pub target_owners: Vec<String>,
    pub user_teams: Vec<String>,
    pub is_exempt: bool,
}

/// Normalizes an owner or team string by trimming and stripping the "team:" prefix case-insensitively.
pub fn normalize_team_name(team: &str) -> String {
    let trimmed = team.trim();
    if trimmed.len() >= 5 && trimmed[..5].eq_ignore_ascii_case("team:") {
        trimmed[5..].trim().to_string()
    } else {
        trimmed.to_string()
    }
}

/// Normalizes a team name to canonical "team:<name>" form.
pub fn canonical_team_string(team: &str) -> String {
    let norm = normalize_team_name(team);
    format!("team:{}", norm)
}

/// Extracts declared owners from an entity's SQLite record or frontmatter file.
/// If target_id is a child constraint (e.g. E12S4/NG-1) with no direct owners,
/// falls back to its parent entity's owners.
pub fn extract_entity_owners(
    workspace_root: &Path,
    target_id: &str,
    storage: Option<&StorageConfig>,
    opt_store: Option<&dyn Store>,
) -> Vec<String> {
    if let Some(store) = opt_store {
        if let Ok(Some(record)) = store.get_entity(target_id) {
            if !record.stale {
                if let Some(ref owners_json) = record.owners {
                    if let Ok(owners) = serde_json::from_str::<Vec<String>>(owners_json) {
                        if !owners.is_empty() {
                            return owners;
                        }
                    }
                }
            }
        }
    }

    if let Ok((_kind, _id, file_path)) =
        resolve_entity_file(workspace_root, None, target_id, storage)
    {
        if let Ok(content) = fs::read_to_string(&file_path) {
            if let Ok(frontmatter) = extract_frontmatter(&content) {
                if let Some(owners_val) = frontmatter.get("owners") {
                    if let Some(arr) = owners_val.as_array() {
                        let owners: Vec<String> = arr
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                            .filter(|s| !s.is_empty())
                            .collect();
                        if !owners.is_empty() {
                            return owners;
                        }
                    }
                }
            }
        }
    }

    // Fallback: child entity inherits parent owners
    if target_id.contains('/') {
        let parent_id = target_id.split('/').next().unwrap_or(target_id);
        if parent_id != target_id {
            return extract_entity_owners(workspace_root, parent_id, storage, opt_store);
        }
    }

    Vec::new()
}

/// Resolves effective teams for the current user combining identity teams and project teams mapping.
pub fn resolve_user_teams(
    author: &Author,
    config: &Config,
    workspace_root: Option<&Path>,
) -> Vec<String> {
    let mut teams = Vec::new();
    let mut seen = HashSet::new();

    // 1. Direct identity teams
    for t in &config.identity.teams {
        let norm = normalize_team_name(t);
        if !norm.is_empty() && seen.insert(norm.clone()) {
            teams.push(norm);
        }
    }

    // 2. Candidate user IDs
    let mut user_ids = Vec::new();
    if !author.id.trim().is_empty() {
        user_ids.push(author.id.trim().to_string());
    }
    if !config.identity.developer_id.trim().is_empty() {
        user_ids.push(config.identity.developer_id.trim().to_string());
    }
    if let Some(root) = workspace_root {
        if let Some(email) = crate::config::resolve_git_email(Some(root)) {
            let email_trimmed = email.trim().to_string();
            if !email_trimmed.is_empty() {
                user_ids.push(email_trimmed);
            }
        }
    }

    // 3. Match user IDs in config [teams] mapping
    for (team_name, members) in &config.teams.teams {
        let norm = normalize_team_name(team_name);
        let is_member = members.iter().any(|m| {
            let m_trimmed = m.trim();
            user_ids
                .iter()
                .any(|uid| uid.eq_ignore_ascii_case(m_trimmed))
        });
        if is_member && seen.insert(norm.clone()) {
            teams.push(norm);
        }
    }

    teams
}

/// Checks whether the author/user or any of their teams match the entity's owners.
pub fn is_user_owner(
    author: &Author,
    config: &Config,
    owners: &[String],
    workspace_root: Option<&Path>,
) -> bool {
    if owners.is_empty() {
        return true;
    }

    let mut user_ids = Vec::new();
    if !author.id.trim().is_empty() {
        user_ids.push(author.id.trim().to_string());
    }
    if !config.identity.developer_id.trim().is_empty() {
        user_ids.push(config.identity.developer_id.trim().to_string());
    }
    if let Some(root) = workspace_root {
        if let Some(email) = crate::config::resolve_git_email(Some(root)) {
            let email_trimmed = email.trim().to_string();
            if !email_trimmed.is_empty() {
                user_ids.push(email_trimmed);
            }
        }
    }

    let user_teams = resolve_user_teams(author, config, workspace_root);

    for owner in owners {
        let owner_trimmed = owner.trim();
        let owner_norm = normalize_team_name(owner_trimmed);

        // Check user ID match
        for uid in &user_ids {
            if uid.eq_ignore_ascii_case(owner_trimmed) || uid.eq_ignore_ascii_case(&owner_norm) {
                return true;
            }
        }

        // Check user team match
        for team in &user_teams {
            if team.eq_ignore_ascii_case(&owner_norm) {
                return true;
            }
        }
    }

    false
}

/// Checks whether an entity is exempt from lease override restrictions.
fn is_exempt_from_lease(
    workspace_root: &Path,
    target_id: &str,
    target_kind: Option<EntityKind>,
    workspace_leases: &[StoryLease],
    storage: Option<&StorageConfig>,
    opt_store: Option<&dyn Store>,
) -> bool {
    // 1. Decisions are always exempt from lease overrides
    if target_kind == Some(EntityKind::Decision) || target_id.starts_with("DEC-") {
        return true;
    }

    if workspace_leases.is_empty() {
        return false;
    }

    // 2. Currently leased story itself
    if workspace_leases.iter().any(|l| l.story_id == target_id) {
        return true;
    }

    // 3. Child constraints of the leased story: {story_id}/...
    if target_id.contains('/') {
        let parts: Vec<&str> = target_id.split('/').collect();
        if parts.len() == 2 && workspace_leases.iter().any(|l| l.story_id == parts[0]) {
            return true;
        }
    }

    // 4. Scratchpads of the leased story
    if target_kind == Some(EntityKind::Scratchpad) {
        if workspace_leases.iter().any(|l| {
            l.story_id == target_id
                || target_id.starts_with(&format!("{}.", l.story_id))
                || target_id.starts_with(&format!("{}/", l.story_id))
        }) {
            return true;
        }
    }

    // 5. Deferred work originating from the leased story
    let is_dw = target_kind == Some(EntityKind::DeferredWork) || target_id.starts_with("DW-");
    if is_dw {
        let origin_story_id = if let Some(store) = opt_store {
            store
                .get_deferred_work(target_id)
                .ok()
                .flatten()
                .and_then(|dw| dw.origin_story_id)
        } else {
            None
        };

        let origin_story_id = origin_story_id.or_else(|| {
            if let Ok((_, _, path)) = resolve_entity_file(
                workspace_root,
                Some(EntityKind::DeferredWork),
                target_id,
                storage,
            ) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(fm) = extract_frontmatter(&content) {
                        return fm
                            .get("origin_story_id")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                    }
                }
            }
            None
        });

        if let Some(origin) = origin_story_id {
            if workspace_leases.iter().any(|l| l.story_id == origin) {
                return true;
            }
        }
    }

    false
}

/// Classifies an entity mutation against workspace leases and declared team ownership.
pub fn classify_mutation(
    workspace_root: &Path,
    target_id: &str,
    target_kind: Option<EntityKind>,
    author: &Author,
    config: &Config,
    opt_store: Option<&dyn Store>,
) -> Result<ScopeClassification, QdevError> {
    let workspace_leases = find_workspace_leases(workspace_root).unwrap_or_default();
    let target_owners =
        extract_entity_owners(workspace_root, target_id, Some(&config.storage), opt_store);
    let user_teams = resolve_user_teams(author, config, Some(workspace_root));

    let resolved_kind = target_kind.or_else(|| {
        if let Some(store) = opt_store {
            if let Ok(Some(record)) = store.get_entity(target_id) {
                return Some(record.kind);
            }
        }
        if let Ok((k, _, _)) =
            resolve_entity_file(workspace_root, None, target_id, Some(&config.storage))
        {
            return Some(k);
        }
        if let Ok(ident) = target_id.parse::<Identifier>() {
            match ident.kind() {
                crate::id::IdentifierKind::Prd => Some(EntityKind::Prd),
                crate::id::IdentifierKind::FunctionalRequirement
                | crate::id::IdentifierKind::NonFunctionalRequirement => {
                    Some(EntityKind::Requirement)
                }
                crate::id::IdentifierKind::Epic => Some(EntityKind::Epic),
                crate::id::IdentifierKind::Story => Some(EntityKind::Story),
                crate::id::IdentifierKind::Adr => Some(EntityKind::Adr),
                crate::id::IdentifierKind::Hazard => Some(EntityKind::Hazard),
                crate::id::IdentifierKind::DeferredWork => Some(EntityKind::DeferredWork),
                crate::id::IdentifierKind::Decision => Some(EntityKind::Decision),
                crate::id::IdentifierKind::Constraint => None,
            }
        } else {
            None
        }
    });

    let is_exempt = is_exempt_from_lease(
        workspace_root,
        target_id,
        resolved_kind,
        &workspace_leases,
        Some(&config.storage),
        opt_store,
    );

    let (is_out_of_lease, active_lease) = if is_exempt {
        (false, workspace_leases.first().cloned())
    } else if !workspace_leases.is_empty() {
        (true, workspace_leases.first().cloned())
    } else {
        let story_to_check = if target_id.contains('/') {
            target_id.split('/').next().unwrap_or(target_id)
        } else {
            target_id
        };
        if let Some(other_lease) = get_lease(workspace_root, story_to_check) {
            (true, Some(other_lease))
        } else {
            (false, None)
        }
    };

    let is_cross_team = if target_owners.is_empty() {
        false
    } else {
        !is_user_owner(author, config, &target_owners, Some(workspace_root))
    };

    Ok(ScopeClassification {
        is_out_of_lease,
        is_cross_team,
        active_lease,
        target_owners,
        user_teams,
        is_exempt,
    })
}

/// Logs a committed `DEC-` record with `decision_type` (`cross_team_override` or `lease_override`).
pub fn create_governance_override_decision(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    target_id: &str,
    decision_type: &str,
    justification: &str,
    author: &Author,
    context_override: Option<&str>,
) -> Result<String, QdevError> {
    author.validate()?;

    let trimmed_just = justification.trim();
    if trimmed_just.is_empty() {
        return Err(QdevError::policy_refusal(
            "needs_justification",
            "--override requires a non-empty --justification",
        ));
    }

    let _lock_guard = acquire_workspace_write_lock(workspace_root, storage)?;

    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    let cache_dir_rel = st.cache_dir.as_str();
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");
    let opt_store = if cache_db_path.is_file() {
        Some(SqliteStore::open(&cache_db_path)?)
    } else {
        None
    };

    let mut rng = rand::rng();
    let dec_ident = allocate_decision_id_in_with_rng(
        workspace_root,
        st,
        &mut rng,
        opt_store.as_ref().map(|s| s as &dyn Store),
    )?;
    let dec_id = dec_ident.to_string();

    let title = match decision_type {
        "cross_team_override" => format!("Cross-team override on {}", target_id),
        "lease_override" => format!("Lease override on {}", target_id),
        other => format!("Governance override ({}) on {}", other, target_id),
    };

    let context_desc = match context_override {
        Some(c) => c.to_string(),
        None => match decision_type {
            "cross_team_override" => format!(
                "Cross-team mutation on entity {} overridden by {}",
                target_id, author.id
            ),
            "lease_override" => format!(
                "Out-of-lease mutation on entity {} overridden by {}",
                target_id, author.id
            ),
            other => format!(
                "Governance override ({}) on entity {} overridden by {}",
                other, target_id, author.id
            ),
        },
    };

    let timestamp = current_iso8601();

    let frontmatter_json = serde_json::json!({
        "id": dec_id,
        "title": title,
        "status": "active",
        "version": 1,
        "created_by": {
            "type": author.author_type,
            "id": author.id,
        },
        "updated_by": {
            "type": author.author_type,
            "id": author.id,
        },
        "subject_id": target_id,
        "decision_type": decision_type,
        "context": context_desc,
        "ruling": trimmed_just,
        "created_at": timestamp,
    });

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

    let frontmatter_yaml = serde_yaml::to_string(&frontmatter_json).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize decision frontmatter: {}", e),
        )
    })?;

    let dec_content = format!(
        "---\n{}---\n\n# {}\n\nContext: {}\n\n{}\n",
        frontmatter_yaml, title, context_desc, trimmed_just
    );

    let rel_dir = directory_for_kind(storage, EntityKind::Decision);
    let file_name = canonical_file_name(&dec_id);
    let dec_file_path = workspace_root.join(&rel_dir).join(&file_name);

    write_file_atomic(&dec_file_path, &dec_content)?;

    if let Some(ref store) = opt_store {
        let rel_source_path = workspace_rel_path(&dec_file_path, workspace_root);
        let content_hash = sha256_digest(dec_content.as_bytes());

        let decision_entity_record = EntityRecord {
            id: dec_id.clone(),
            kind: EntityKind::Decision,
            title: Some(title),
            status: Some("active".to_string()),
            owners: None,
            source_path: rel_source_path,
            content_hash,
            version: 1,
            created_by: Some(author.clone()),
            updated_by: Some(author.clone()),
            updated_at: timestamp.to_string(),
            stale: false,
            epic_id: None,
            seq: None,
            appetite: None,
            safety_class: None,
            target_modules: None,
        };
        upsert_cache_and_mark_dirty(&cache_db_path, &decision_entity_record)?;

        let decision_record = DecisionRecord {
            id: dec_id.clone(),
            subject_id: target_id.to_string(),
            decision_type: Some(decision_type.to_string()),
            topic: None,
            context: Some(context_desc),
            ruling: Some(trimmed_just.to_string()),
            author_type: Some(author.author_type.clone()),
            author_id: Some(author.id.clone()),
            created_at: Some(timestamp.to_string()),
        };
        store.upsert_decision(&decision_record)?;
    }

    Ok(dec_id)
}

/// Appends a user's team to an entity's `owners` list in frontmatter and updates the cache.
pub fn add_team_to_entity_owners(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    entity_id: &str,
    team: &str,
    author: &Author,
    if_version: Option<u64>,
) -> Result<Vec<String>, QdevError> {
    author.validate()?;

    let trimmed_team = team.trim();
    if trimmed_team.is_empty() {
        return Err(QdevError::usage_error("Team name cannot be empty"));
    }
    let team_to_add = if trimmed_team.len() >= 5 && trimmed_team[..5].eq_ignore_ascii_case("team:")
    {
        canonical_team_string(trimmed_team)
    } else {
        trimmed_team.to_string()
    };

    let (kind, id, file_path) =
        resolve_entity_file(workspace_root, None, entity_id, storage)?;

    let _lock_guard = acquire_workspace_write_lock(workspace_root, storage)?;

    let existing_content = fs::read_to_string(&file_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to read entity file '{}': {}",
                file_path.display(),
                e
            ),
        )
    })?;

    let frontmatter = extract_frontmatter(&existing_content).map_err(|e| {
        QdevError::infrastructure_failure(
            "parse_error",
            format!("Failed to parse entity frontmatter: {}", e),
        )
    })?;
    let mut current_owners: Vec<String> = frontmatter
        .get("owners")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let team_norm = normalize_team_name(&team_to_add);
    let already_present = current_owners.iter().any(|o| {
        let o_norm = normalize_team_name(o);
        o == &team_to_add || o_norm == team_norm
    });

    if already_present {
        return Ok(current_owners);
    }

    current_owners.push(team_to_add);

    let owners_yaml = serde_yaml::Value::Sequence(
        current_owners
            .iter()
            .map(|s| serde_yaml::Value::String(s.clone()))
            .collect(),
    );

    let patch_opts = FrontmatterPatchOptions {
        status: None,
        title: None,
        custom_fields: vec![("owners".to_string(), owners_yaml)],
        author: Some(author.clone()),
        if_version,
    };

    let (patched_content, new_version) = patch_frontmatter(&existing_content, &patch_opts)?;

    let write_kind = kind_for_write(&file_path, &patched_content, kind);
    validate_frontmatter(write_kind, &patched_content).map_err(|errs| {
        QdevError::logical_failure(
            "schema_validation_failed",
            format!("Updated frontmatter failed schema validation: {:?}", errs),
        )
    })?;

    write_file_atomic(&file_path, &patched_content)?;

    // Update cache if database exists
    let default_storage = StorageConfig::default();
    let st = storage.unwrap_or(&default_storage);
    let cache_dir_rel = st.cache_dir.as_str();
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");
    if cache_db_path.is_file() {
        let content_hash = sha256_digest(patched_content.as_bytes());
        let updated_frontmatter = extract_frontmatter(&patched_content).map_err(|e| {
            QdevError::infrastructure_failure(
                "parse_error",
                format!("Failed to parse updated frontmatter: {}", e),
            )
        })?;
        let title_val = updated_frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let status_val = updated_frontmatter
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let owners_val = updated_frontmatter.get("owners").map(|v| v.to_string());
        let c_author = updated_frontmatter
            .get("created_by")
            .and_then(|v| serde_json::from_value::<Author>(v.clone()).ok());
        let u_author = Some(author.clone());
        let (epic_id, seq, appetite, safety_class, target_modules) =
            story_detail_fields(write_kind, &id, &updated_frontmatter);
        let rel_path = workspace_rel_path(&file_path, workspace_root);
        let record = EntityRecord {
            id: id.clone(),
            kind: write_kind,
            title: title_val,
            status: status_val,
            owners: owners_val,
            source_path: rel_path,
            content_hash,
            version: new_version,
            created_by: c_author,
            updated_by: u_author,
            updated_at: current_iso8601(),
            stale: false,
            epic_id,
            seq,
            appetite,
            safety_class,
            target_modules,
        };
        upsert_cache_and_mark_dirty(&cache_db_path, &record)?;
    }

    Ok(current_owners)
}
