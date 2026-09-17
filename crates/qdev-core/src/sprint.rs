//! Sprint lifecycle domain logic and story assignment handling.
//!
//! Conforms to AD-7, AD-8, and Story 2.9: Decouples story identity from sprint
//! membership via `docs/state/sprints/sprint-{n}.md` frontmatter and `sprint_assignments`
//! SQLite table. Sprints are written atomically under the workspace advisory write lock,
//! keeping markdown files and SQLite cache synchronized in lock-step.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, StorageConfig};
use crate::errors::QdevError;
use crate::schema::{
    extract_frontmatter, extract_frontmatter_str, validate_value_detailed, EntityKind,
};
use crate::store::{EntityRecord, SprintAssignmentRecord, SprintRecord, Store};
use crate::write::{
    acquire_workspace_write_lock, canonical_file_name, current_iso8601, directory_for_kind,
    patch_frontmatter, resolve_entity_file, sha256_digest, workspace_rel_path, write_file_atomic,
    Author, FrontmatterPatchOptions,
};

/// Normalizes a sprint identifier string (e.g. "6", "sprint-6", or "Sprint-6") to a positive integer.
pub fn normalize_sprint_id(s: &str) -> Result<i64, QdevError> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(QdevError::usage_error("Sprint identifier cannot be empty"));
    }
    let numeric_part = if let Some(rest) = trimmed.strip_prefix("sprint-") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("Sprint-") {
        rest
    } else {
        trimmed
    };
    match numeric_part.parse::<i64>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(QdevError::usage_error(format!(
            "Invalid sprint identifier '{}', expected positive integer or 'sprint-<n>'",
            s
        ))),
    }
}

/// Formats a sprint entity identifier (e.g. 6 -> "sprint-6").
pub fn sprint_entity_id(sprint_num: i64) -> String {
    format!("sprint-{}", sprint_num)
}

/// Resolves sprint selection following strict precedence:
/// 1. Explicit CLI argument / flag
/// 2. `default_sprint` in `qdev.toml`
/// 3. Unique single active sprint in the workspace
/// 4. Usage error (exit 2) when zero or multiple active sprints exist
pub fn resolve_sprint_selection(
    store: &dyn Store,
    config: &Config,
    explicit: Option<&str>,
) -> Result<i64, QdevError> {
    if let Some(s) = explicit {
        return normalize_sprint_id(s);
    }

    if let Some(default_sprint) = config.project.default_sprint {
        return Ok(default_sprint as i64);
    }

    let sprints = store.list_sprints()?;
    let mut active = Vec::new();
    for s in sprints {
        if s.status.as_deref() == Some("active") {
            let entity_id = sprint_entity_id(s.id);
            if store
                .entity_exists_for_derivation(&entity_id)
                .unwrap_or(true)
            {
                active.push(s.id);
            }
        }
    }

    match active.len() {
        1 => Ok(active[0]),
        0 => Err(QdevError::usage_error(
            "No active sprint found; specify --sprint <id>",
        )),
        _ => {
            active.sort();
            let list = active
                .iter()
                .map(|id| format!("sprint-{}", id))
                .collect::<Vec<_>>()
                .join(", ");
            Err(QdevError::usage_error(format!(
                "Multiple active sprints found ({}); specify --sprint <id>",
                list
            )))
        }
    }
}

/// Input options for opening a new sprint.
#[derive(Clone)]
pub struct SprintOpenOptions<'a> {
    pub workspace_root: &'a Path,
    pub storage: &'a StorageConfig,
    pub store: &'a dyn Store,
    pub sprint: i64,
    pub title: &'a str,
    pub release: Option<&'a str>,
    pub author: &'a Author,
}

/// Result of opening a new sprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintOpenResult {
    pub sprint_id: i64,
    pub entity_id: String,
    pub title: String,
    pub status: String,
    pub release: Option<String>,
    pub started_at: String,
    pub file_path: PathBuf,
    pub rel_path: String,
}

/// Opens a new sprint in `active` status, creating `docs/state/sprints/sprint-{n}.md`
/// and hydrating the SQLite cache.
pub fn open_sprint(options: &SprintOpenOptions) -> Result<SprintOpenResult, QdevError> {
    options.author.validate()?;

    if options.title.trim().is_empty() {
        return Err(QdevError::logical_failure(
            "empty_title",
            "Sprint title cannot be empty",
        ));
    }

    if options.sprint <= 0 {
        return Err(QdevError::logical_failure(
            "invalid_sprint_id",
            "Sprint number must be a positive integer",
        ));
    }

    let entity_id = sprint_entity_id(options.sprint);
    let rel_dir = directory_for_kind(Some(options.storage), EntityKind::Sprint);
    let file_name = canonical_file_name(&entity_id);
    let sprint_file_path = options.workspace_root.join(&rel_dir).join(&file_name);

    if sprint_file_path.symlink_metadata().is_ok() {
        return Err(QdevError::logical_failure(
            "sprint_already_exists",
            format!("Sprint {} already exists", options.sprint),
        ));
    }

    // A retained row whose file has gone does not prove the sprint exists: `entity_exists_for_derivation`
    // is the one answer to that question, so re-opening a sprint that only survives as a stale
    // row recreates it instead of refusing forever.
    if options.store.get_sprint(options.sprint)?.is_some()
        || options.store.entity_exists_for_derivation(&entity_id)?
    {
        return Err(QdevError::logical_failure(
            "sprint_already_exists",
            format!("Sprint {} already exists", options.sprint),
        ));
    }

    let _lock_guard = acquire_workspace_write_lock(options.workspace_root, Some(options.storage))?;

    if sprint_file_path.symlink_metadata().is_ok() {
        return Err(QdevError::logical_failure(
            "sprint_already_exists",
            format!("Sprint {} already exists", options.sprint),
        ));
    }

    let started_at = current_iso8601()[..10].to_string();

    let mut mapping = serde_yaml::Mapping::new();
    mapping.insert(
        serde_yaml::Value::String("id".to_string()),
        serde_yaml::Value::String(entity_id.clone()),
    );
    mapping.insert(
        serde_yaml::Value::String("title".to_string()),
        serde_yaml::Value::String(options.title.trim().to_string()),
    );
    mapping.insert(
        serde_yaml::Value::String("status".to_string()),
        serde_yaml::Value::String("active".to_string()),
    );
    mapping.insert(
        serde_yaml::Value::String("version".to_string()),
        serde_yaml::Value::Number(1.into()),
    );
    if let Some(rel) = options.release.filter(|r| !r.trim().is_empty()) {
        mapping.insert(
            serde_yaml::Value::String("release".to_string()),
            serde_yaml::Value::String(rel.trim().to_string()),
        );
        mapping.insert(
            serde_yaml::Value::String("release_version".to_string()),
            serde_yaml::Value::String(rel.trim().to_string()),
        );
    }
    mapping.insert(
        serde_yaml::Value::String("started_at".to_string()),
        serde_yaml::Value::String(started_at.clone()),
    );
    mapping.insert(
        serde_yaml::Value::String("assignments".to_string()),
        serde_yaml::Value::Sequence(Vec::new()),
    );

    let mut created_by = serde_yaml::Mapping::new();
    created_by.insert(
        serde_yaml::Value::String("type".to_string()),
        serde_yaml::Value::String(options.author.author_type.clone()),
    );
    created_by.insert(
        serde_yaml::Value::String("id".to_string()),
        serde_yaml::Value::String(options.author.id.clone()),
    );
    mapping.insert(
        serde_yaml::Value::String("created_by".to_string()),
        serde_yaml::Value::Mapping(created_by.clone()),
    );
    mapping.insert(
        serde_yaml::Value::String("updated_by".to_string()),
        serde_yaml::Value::Mapping(created_by),
    );

    let yaml_val = serde_yaml::Value::Mapping(mapping);
    let json_val: serde_json::Value = serde_yaml::from_value(yaml_val.clone())
        .map_err(|e| QdevError::infrastructure_failure("yaml_error", e.to_string()))?;
    validate_value_detailed(EntityKind::Sprint, &json_val).map_err(|errs| {
        QdevError::logical_failure(
            "schema_violation",
            format!("Sprint schema validation failed: {:?}", errs),
        )
    })?;

    let fm_str = serde_yaml::to_string(&yaml_val)
        .map_err(|e| QdevError::infrastructure_failure("yaml_error", e.to_string()))?;
    let content = format!(
        "---\n{}---\n\n# Sprint {}: {}\n",
        fm_str,
        options.sprint,
        options.title.trim()
    );

    write_file_atomic(&sprint_file_path, &content)?;

    let content_hash = sha256_digest(content.as_bytes());
    let rel_path_str = workspace_rel_path(&sprint_file_path, options.workspace_root);

    let entity_record = EntityRecord {
        id: entity_id.clone(),
        kind: EntityKind::Sprint,
        title: Some(options.title.trim().to_string()),
        status: Some("active".to_string()),
        owners: None,
        source_path: rel_path_str.clone(),
        content_hash,
        version: 1,
        created_by: Some(options.author.clone()),
        updated_by: Some(options.author.clone()),
        updated_at: current_iso8601(),
        stale: false,
        epic_id: None,
        seq: None,
        appetite: None,
        safety_class: None,
        target_modules: None,
    };
    options.store.upsert_entity(&entity_record)?;

    let sprint_record = SprintRecord {
        id: options.sprint,
        title: Some(options.title.trim().to_string()),
        release_version: options.release.map(|r| r.trim().to_string()),
        status: Some("active".to_string()),
        owners: None,
        started_at: Some(started_at.clone()),
        completed_at: None,
    };
    options.store.upsert_sprint(&sprint_record)?;

    Ok(SprintOpenResult {
        sprint_id: options.sprint,
        entity_id,
        title: options.title.trim().to_string(),
        status: "active".to_string(),
        release: options.release.map(|r| r.trim().to_string()),
        started_at,
        file_path: sprint_file_path,
        rel_path: rel_path_str,
    })
}

/// Input options for assigning stories to a sprint.
#[derive(Clone)]
pub struct SprintAssignOptions<'a> {
    pub workspace_root: &'a Path,
    pub storage: &'a StorageConfig,
    pub store: &'a dyn Store,
    pub sprint: i64,
    pub stories: &'a [String],
    pub author: &'a Author,
}

/// Result of assigning stories to a sprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintAssignResult {
    pub sprint_id: i64,
    pub entity_id: String,
    pub assigned_stories: Vec<String>,
    pub assigned_at: String,
    pub new_version: u64,
}

/// Assigns stories to a sprint, updating `sprint-{n}.md` frontmatter and `sprint_assignments` table.
/// Story files and story identifiers remain strictly immutable.
pub fn assign_to_sprint(options: &SprintAssignOptions) -> Result<SprintAssignResult, QdevError> {
    options.author.validate()?;

    if options.stories.is_empty() {
        return Err(QdevError::usage_error(
            "No stories specified for sprint assignment",
        ));
    }

    let entity_id = sprint_entity_id(options.sprint);
    let (_, _, abs_path) = match resolve_entity_file(
        options.workspace_root,
        Some(EntityKind::Sprint),
        &entity_id,
        Some(options.storage),
    ) {
        Ok(r) => r,
        Err(_) => {
            return Err(QdevError::usage_error_with_code(
                "sprint_not_found",
                format!("Sprint {} not found", options.sprint),
            ));
        }
    };

    for story_id in options.stories {
        // A stale row does not make a story assignable: an entity whose file cannot be read is
        // absent, so the `story_not_found` path is the honest answer.
        match options.store.get_live_entity_for_derivation(story_id)? {
            Some(ref e) if e.kind == EntityKind::Story => {}
            _ => {
                let file_check = resolve_entity_file(
                    options.workspace_root,
                    Some(EntityKind::Story),
                    story_id,
                    Some(options.storage),
                );
                if file_check.is_err() {
                    return Err(QdevError::usage_error_with_code(
                        "story_not_found",
                        format!("Story '{}' not found", story_id),
                    ));
                }
            }
        }
    }

    let _lock_guard = acquire_workspace_write_lock(options.workspace_root, Some(options.storage))?;

    let content = fs::read_to_string(&abs_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to read sprint file '{}': {}", abs_path.display(), e),
        )
    })?;

    let (fm_str, _) = extract_frontmatter_str(&content).map_err(|e| {
        QdevError::logical_failure(
            "missing_frontmatter",
            format!(
                "Failed to locate frontmatter in '{}': {}",
                abs_path.display(),
                e
            ),
        )
    })?;

    let fm_yaml: serde_yaml::Value = serde_yaml::from_str(fm_str).map_err(|e| {
        QdevError::logical_failure(
            "yaml_parse_error",
            format!(
                "Failed to parse frontmatter in '{}': {}",
                abs_path.display(),
                e
            ),
        )
    })?;

    let mut assignments_seq = match fm_yaml.get("assignments") {
        Some(serde_yaml::Value::Sequence(seq)) => seq.clone(),
        _ => Vec::new(),
    };

    let assigned_at = current_iso8601()[..10].to_string();
    let mut assigned_stories = Vec::new();

    for story_id in options.stories {
        let already_assigned = assignments_seq.iter().any(|item| {
            item.get("story")
                .or_else(|| item.get("story_id"))
                .and_then(|v| v.as_str())
                == Some(story_id.as_str())
        });

        if !already_assigned {
            let mut entry = serde_yaml::Mapping::new();
            entry.insert(
                serde_yaml::Value::String("story".to_string()),
                serde_yaml::Value::String(story_id.clone()),
            );
            entry.insert(
                serde_yaml::Value::String("assigned_at".to_string()),
                serde_yaml::Value::String(assigned_at.clone()),
            );
            assignments_seq.push(serde_yaml::Value::Mapping(entry));
        }
        assigned_stories.push(story_id.clone());
    }

    let patch_opts = FrontmatterPatchOptions {
        custom_fields: vec![(
            "assignments".to_string(),
            serde_yaml::Value::Sequence(assignments_seq),
        )],
        author: Some(options.author.clone()),
        ..Default::default()
    };

    let (patched_content, new_version) = patch_frontmatter(&content, &patch_opts)?;

    let json_fm = extract_frontmatter(&patched_content)
        .map_err(|e| QdevError::logical_failure("schema_error", e.to_string()))?;
    validate_value_detailed(EntityKind::Sprint, &json_fm).map_err(|errs| {
        QdevError::logical_failure(
            "schema_violation",
            format!(
                "Sprint schema validation failed after assignment: {:?}",
                errs
            ),
        )
    })?;

    write_file_atomic(&abs_path, &patched_content)?;

    let content_hash = sha256_digest(patched_content.as_bytes());
    let rel_path_str = workspace_rel_path(&abs_path, options.workspace_root);

    // The file we just wrote is the truth; a retained row behind it must not supply a stale
    // title or status, so the record is rebuilt from the fresh frontmatter when the row is stale.
    let mut entity_rec = options
        .store
        .get_live_entity_for_derivation(&entity_id)?
        .unwrap_or(EntityRecord {
            id: entity_id.clone(),
            kind: EntityKind::Sprint,
            title: json_fm
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            status: Some("active".to_string()),
            owners: None,
            source_path: rel_path_str,
            content_hash: content_hash.clone(),
            version: new_version,
            created_by: None,
            updated_by: Some(options.author.clone()),
            updated_at: current_iso8601(),
            stale: false,
            epic_id: None,
            seq: None,
            appetite: None,
            safety_class: None,
            target_modules: None,
        });

    entity_rec.version = new_version;
    entity_rec.content_hash = content_hash;
    entity_rec.updated_by = Some(options.author.clone());
    entity_rec.updated_at = current_iso8601();
    options.store.upsert_entity(&entity_rec)?;

    for story_id in &assigned_stories {
        options
            .store
            .upsert_sprint_assignment(&SprintAssignmentRecord {
                sprint_id: options.sprint,
                story_id: story_id.clone(),
                assigned_at: assigned_at.clone(),
                carried_from: None,
            })?;
    }

    Ok(SprintAssignResult {
        sprint_id: options.sprint,
        entity_id,
        assigned_stories,
        assigned_at,
        new_version,
    })
}

/// Input options for closing a sprint.
#[derive(Clone)]
pub struct SprintCloseOptions<'a> {
    pub workspace_root: &'a Path,
    pub storage: &'a StorageConfig,
    pub store: &'a dyn Store,
    pub sprint: i64,
    pub carry_over_target: Option<i64>,
    pub author: &'a Author,
}

/// Result of closing a sprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintCloseResult {
    pub sprint_id: i64,
    pub status: String,
    pub completed_at: String,
    pub carry_over_target: Option<i64>,
    pub carried_stories: Vec<String>,
}

/// Closes a sprint, marks it `completed`, and executes carry-over for any non-done stories.
/// Story files and story identifiers are strictly immutable.
pub fn close_sprint(options: &SprintCloseOptions) -> Result<SprintCloseResult, QdevError> {
    options.author.validate()?;

    let entity_id = sprint_entity_id(options.sprint);
    let (_, _, abs_path) = match resolve_entity_file(
        options.workspace_root,
        Some(EntityKind::Sprint),
        &entity_id,
        Some(options.storage),
    ) {
        Ok(r) => r,
        Err(_) => {
            return Err(QdevError::usage_error_with_code(
                "sprint_not_found",
                format!("Sprint {} not found", options.sprint),
            ));
        }
    };

    let target_abs_path = if let Some(target_num) = options.carry_over_target {
        if target_num == options.sprint {
            return Err(QdevError::usage_error(
                "Cannot carry over stories to the sprint being closed",
            ));
        }
        let target_entity_id = sprint_entity_id(target_num);
        match resolve_entity_file(
            options.workspace_root,
            Some(EntityKind::Sprint),
            &target_entity_id,
            Some(options.storage),
        ) {
            Ok((_, _, p)) => Some((target_num, target_entity_id, p)),
            Err(_) => {
                return Err(QdevError::usage_error_with_code(
                    "sprint_not_found",
                    format!("Sprint {} not found", target_num),
                ));
            }
        }
    } else {
        None
    };

    let _lock_guard = acquire_workspace_write_lock(options.workspace_root, Some(options.storage))?;

    let completed_at = current_iso8601()[..10].to_string();

    let closing_content = fs::read_to_string(&abs_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to read sprint file '{}': {}", abs_path.display(), e),
        )
    })?;

    let patch_opts = FrontmatterPatchOptions {
        status: Some("completed".to_string()),
        custom_fields: vec![(
            "completed_at".to_string(),
            serde_yaml::Value::String(completed_at.clone()),
        )],
        author: Some(options.author.clone()),
        ..Default::default()
    };

    let (patched_closing, closing_ver) = patch_frontmatter(&closing_content, &patch_opts)?;

    let json_fm = extract_frontmatter(&patched_closing)
        .map_err(|e| QdevError::logical_failure("schema_error", e.to_string()))?;
    validate_value_detailed(EntityKind::Sprint, &json_fm).map_err(|errs| {
        QdevError::logical_failure(
            "schema_violation",
            format!("Sprint schema validation failed after closing: {:?}", errs),
        )
    })?;

    write_file_atomic(&abs_path, &patched_closing)?;

    let closing_hash = sha256_digest(patched_closing.as_bytes());
    let closing_rel_path = workspace_rel_path(&abs_path, options.workspace_root);

    // Same rule as `assign_to_sprint`: the freshly written sprint file outranks a retained row.
    let mut entity_rec = options
        .store
        .get_live_entity_for_derivation(&entity_id)?
        .unwrap_or(EntityRecord {
            id: entity_id.clone(),
            kind: EntityKind::Sprint,
            title: json_fm
                .get("title")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            status: Some("completed".to_string()),
            owners: None,
            source_path: closing_rel_path,
            content_hash: closing_hash.clone(),
            version: closing_ver,
            created_by: None,
            updated_by: Some(options.author.clone()),
            updated_at: current_iso8601(),
            stale: false,
            epic_id: None,
            seq: None,
            appetite: None,
            safety_class: None,
            target_modules: None,
        });

    entity_rec.status = Some("completed".to_string());
    entity_rec.version = closing_ver;
    entity_rec.content_hash = closing_hash;
    entity_rec.updated_by = Some(options.author.clone());
    entity_rec.updated_at = current_iso8601();
    options.store.upsert_entity(&entity_rec)?;

    let mut sprint_rec = options
        .store
        .get_sprint(options.sprint)?
        .unwrap_or_default();
    sprint_rec.id = options.sprint;
    sprint_rec.status = Some("completed".to_string());
    sprint_rec.completed_at = Some(completed_at.clone());
    options.store.upsert_sprint(&sprint_rec)?;

    let mut carried_stories = Vec::new();

    if let Some((target_num, target_entity_id, target_path)) = target_abs_path {
        let existing_assignments = options.store.get_sprint_assignments(options.sprint)?;

        for a in existing_assignments {
            // Carrying a story over derives from the story's live status; a retained row is not
            // evidence that the story is done, so an unreadable story stays carried rather than
            // being silently dropped from the next sprint.
            let story_entity = options.store.get_live_entity_for_derivation(&a.story_id)?;
            let is_done = story_entity.as_ref().and_then(|e| e.status.as_deref()) == Some("done");
            if !is_done {
                carried_stories.push(a.story_id);
            }
        }

        if !carried_stories.is_empty() {
            let target_content = fs::read_to_string(&target_path).map_err(|e| {
                QdevError::infrastructure_failure(
                    "io_error",
                    format!(
                        "Failed to read target sprint file '{}': {}",
                        target_path.display(),
                        e
                    ),
                )
            })?;

            let (target_fm_str, _) = extract_frontmatter_str(&target_content).map_err(|e| {
                QdevError::logical_failure(
                    "missing_frontmatter",
                    format!(
                        "Failed to locate frontmatter in target sprint '{}': {}",
                        target_path.display(),
                        e
                    ),
                )
            })?;

            let target_fm_yaml: serde_yaml::Value =
                serde_yaml::from_str(target_fm_str).map_err(|e| {
                    QdevError::logical_failure(
                        "yaml_parse_error",
                        format!(
                            "Failed to parse frontmatter in target sprint '{}': {}",
                            target_path.display(),
                            e
                        ),
                    )
                })?;

            let mut target_assignments_seq = match target_fm_yaml.get("assignments") {
                Some(serde_yaml::Value::Sequence(seq)) => seq.clone(),
                _ => Vec::new(),
            };

            let assigned_at = current_iso8601()[..10].to_string();
            for story_id in &carried_stories {
                let already = target_assignments_seq.iter().any(|item| {
                    item.get("story")
                        .or_else(|| item.get("story_id"))
                        .and_then(|v| v.as_str())
                        == Some(story_id.as_str())
                });

                if !already {
                    let mut entry = serde_yaml::Mapping::new();
                    entry.insert(
                        serde_yaml::Value::String("story".to_string()),
                        serde_yaml::Value::String(story_id.clone()),
                    );
                    entry.insert(
                        serde_yaml::Value::String("assigned_at".to_string()),
                        serde_yaml::Value::String(assigned_at.clone()),
                    );
                    entry.insert(
                        serde_yaml::Value::String("carried_from".to_string()),
                        serde_yaml::Value::Number(options.sprint.into()),
                    );
                    target_assignments_seq.push(serde_yaml::Value::Mapping(entry));
                }
            }

            let target_patch_opts = FrontmatterPatchOptions {
                custom_fields: vec![(
                    "assignments".to_string(),
                    serde_yaml::Value::Sequence(target_assignments_seq),
                )],
                author: Some(options.author.clone()),
                ..Default::default()
            };

            let (patched_target, target_ver) =
                patch_frontmatter(&target_content, &target_patch_opts)?;

            let target_json_fm = extract_frontmatter(&patched_target)
                .map_err(|e| QdevError::logical_failure("schema_error", e.to_string()))?;
            validate_value_detailed(EntityKind::Sprint, &target_json_fm).map_err(|errs| {
                QdevError::logical_failure(
                    "schema_violation",
                    format!(
                        "Target sprint schema validation failed after carry-over: {:?}",
                        errs
                    ),
                )
            })?;

            write_file_atomic(&target_path, &patched_target)?;

            let target_hash = sha256_digest(patched_target.as_bytes());
            let target_rel_path = workspace_rel_path(&target_path, options.workspace_root);

            // Same rule as above: the target sprint's record comes from the file just written,
            // not from a row that may have gone stale.
            let mut target_rec = options
                .store
                .get_live_entity_for_derivation(&target_entity_id)?
                .unwrap_or(EntityRecord {
                    id: target_entity_id.clone(),
                    kind: EntityKind::Sprint,
                    title: target_json_fm
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    status: Some("active".to_string()),
                    owners: None,
                    source_path: target_rel_path,
                    content_hash: target_hash.clone(),
                    version: target_ver,
                    created_by: None,
                    updated_by: Some(options.author.clone()),
                    updated_at: current_iso8601(),
                    stale: false,
                    epic_id: None,
                    seq: None,
                    appetite: None,
                    safety_class: None,
                    target_modules: None,
                });

            target_rec.version = target_ver;
            target_rec.content_hash = target_hash;
            target_rec.updated_by = Some(options.author.clone());
            target_rec.updated_at = current_iso8601();
            options.store.upsert_entity(&target_rec)?;

            for story_id in &carried_stories {
                options
                    .store
                    .upsert_sprint_assignment(&SprintAssignmentRecord {
                        sprint_id: target_num,
                        story_id: story_id.clone(),
                        assigned_at: assigned_at.clone(),
                        carried_from: Some(options.sprint),
                    })?;
            }
        }
    }

    Ok(SprintCloseResult {
        sprint_id: options.sprint,
        status: "completed".to_string(),
        completed_at,
        carry_over_target: options.carry_over_target,
        carried_stories,
    })
}
