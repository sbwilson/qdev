use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::schema::EntityKind;
use crate::store::{SqliteStore, Store};
use crate::write::{
    apply_entity_update, has_markdown_heading, resolve_entity_file, Author, EntityUpdateOptions,
    EntityUpdateResult,
};

/// All valid lifecycle states for a story entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoryState {
    Draft,
    Ready,
    InProgress,
    Review,
    Done,
    Superseded,
    Abandoned,
}

impl StoryState {
    /// Returns the canonical kebab-case string representation of the state.
    pub fn as_str(&self) -> &'static str {
        match self {
            StoryState::Draft => "draft",
            StoryState::Ready => "ready",
            StoryState::InProgress => "in-progress",
            StoryState::Review => "review",
            StoryState::Done => "done",
            StoryState::Superseded => "superseded",
            StoryState::Abandoned => "abandoned",
        }
    }

    /// Returns true if this state is terminal (done, superseded, abandoned).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            StoryState::Done | StoryState::Superseded | StoryState::Abandoned
        )
    }

    /// Forward pipeline order for non-terminal progression.
    pub fn forward_rank(&self) -> Option<u8> {
        match self {
            StoryState::Draft => Some(0),
            StoryState::Ready => Some(1),
            StoryState::InProgress => Some(2),
            StoryState::Review => Some(3),
            StoryState::Done => Some(4),
            StoryState::Superseded | StoryState::Abandoned => None,
        }
    }

    /// Parses a string into a StoryState variant, accepting case-insensitively with underscores or hyphens.
    pub fn parse(s: &str) -> Result<Self, QdevError> {
        let normalized = s.trim().to_lowercase().replace('_', "-");
        match normalized.as_str() {
            "draft" => Ok(StoryState::Draft),
            "ready" => Ok(StoryState::Ready),
            "in-progress" => Ok(StoryState::InProgress),
            "review" => Ok(StoryState::Review),
            "done" => Ok(StoryState::Done),
            "superseded" => Ok(StoryState::Superseded),
            "abandoned" => Ok(StoryState::Abandoned),
            other => Err(QdevError::usage_error(format!(
                "Unknown story status '{}', expected one of: draft, ready, in-progress, review, done, superseded, abandoned",
                other
            ))),
        }
    }
}

impl fmt::Display for StoryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for StoryState {
    type Err = QdevError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Transition classification within the lifecycle state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    LegalForward,
    TerminalJump,
    Backward,
}

/// Classifies a transition between two story states, returning an error if illegal.
pub fn classify_transition(from: StoryState, to: StoryState) -> Result<TransitionKind, QdevError> {
    if from.is_terminal() {
        return Err(QdevError::logical_failure(
            "invalid_transition",
            format!(
                "Cannot transition out of terminal state '{}'",
                from.as_str()
            ),
        ));
    }

    if from == to {
        return Err(QdevError::logical_failure(
            "invalid_transition",
            format!("Story is already in status '{}'", from.as_str()),
        ));
    }

    if to == StoryState::Superseded || to == StoryState::Abandoned {
        return Ok(TransitionKind::TerminalJump);
    }

    match (from, to) {
        (StoryState::Draft, StoryState::Ready)
        | (StoryState::Ready, StoryState::InProgress)
        | (StoryState::InProgress, StoryState::Review)
        | (StoryState::Review, StoryState::Done) => Ok(TransitionKind::LegalForward),
        _ => {
            if let (Some(from_r), Some(to_r)) = (from.forward_rank(), to.forward_rank()) {
                if to_r < from_r {
                    Ok(TransitionKind::Backward)
                } else {
                    Err(QdevError::logical_failure(
                        "invalid_transition",
                        format!(
                            "Invalid forward transition from '{}' to '{}'; legal forward edges are draft -> ready, ready -> in-progress, in-progress -> review, review -> done",
                            from.as_str(),
                            to.as_str()
                        ),
                    ))
                }
            } else {
                Err(QdevError::logical_failure(
                    "invalid_transition",
                    format!(
                        "Invalid transition from '{}' to '{}'",
                        from.as_str(),
                        to.as_str()
                    ),
                ))
            }
        }
    }
}

/// Context passed to pre- and post-transition lifecycle hooks.
#[derive(Debug, Clone)]
pub struct TransitionContext {
    pub workspace_root: PathBuf,
    pub story_id: String,
    pub from_state: StoryState,
    pub to_state: StoryState,
    pub justification: Option<String>,
    pub author: Author,
    pub storage: Option<StorageConfig>,
}

/// Synchronous hook executed prior to updating the story frontmatter.
pub trait PreTransitionHook: Send + Sync {
    fn run(&self, ctx: &TransitionContext) -> Result<(), QdevError>;
}

impl<F> PreTransitionHook for F
where
    F: Fn(&TransitionContext) -> Result<(), QdevError> + Send + Sync,
{
    fn run(&self, ctx: &TransitionContext) -> Result<(), QdevError> {
        self(ctx)
    }
}

/// Synchronous hook executed after updating the story frontmatter and closing DW.
pub trait PostTransitionHook: Send + Sync {
    fn run(&self, ctx: &TransitionContext, result: &EntityUpdateResult) -> Result<(), QdevError>;
}

impl<F> PostTransitionHook for F
where
    F: Fn(&TransitionContext, &EntityUpdateResult) -> Result<(), QdevError> + Send + Sync,
{
    fn run(&self, ctx: &TransitionContext, result: &EntityUpdateResult) -> Result<(), QdevError> {
        self(ctx, result)
    }
}

/// Options controlling a story lifecycle transition.
#[derive(Debug, Clone)]
pub struct TransitionOptions {
    pub workspace_root: PathBuf,
    pub storage: Option<StorageConfig>,
    pub entity_kind: String,
    pub story_id: String,
    pub target_status: String,
    pub justification: Option<String>,
    pub author: Author,
    pub if_version: Option<u64>,
}

/// Successful output payload emitted by story transition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransitionPayload {
    pub id: String,
    pub from_status: String,
    pub to_status: String,
    pub version: u64,
    pub closed_dw: Vec<String>,
}

/// Validates that a story meets the prerequisites for transitioning from `draft` to `ready`.
fn validate_readiness_criteria(
    content: &str,
    frontmatter: &serde_json::Value,
    story_id: &str,
) -> Result<(), QdevError> {
    let mut missing = Vec::new();

    // 1. Acceptance Criteria markdown heading
    if !has_markdown_heading(content, "Acceptance Criteria") {
        missing.push("acceptance_criteria");
    }

    // 2. Non-empty valid appetite
    let valid_appetite = frontmatter
        .get("appetite")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty() && matches!(s, "tiny" | "small" | "medium" | "deep"))
        .unwrap_or(false);
    if !valid_appetite {
        missing.push("appetite");
    }

    // 3. At least one target_modules entry
    let valid_target_modules = frontmatter
        .get("target_modules")
        .and_then(|v| v.as_array())
        .map(|arr| {
            !arr.is_empty()
                && arr
                    .iter()
                    .any(|v| v.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false))
        })
        .unwrap_or(false);
    if !valid_target_modules {
        missing.push("target_modules");
    }

    if !missing.is_empty() {
        let reasons: Vec<String> = missing
            .iter()
            .map(|m| match *m {
                "acceptance_criteria" => "missing '## Acceptance Criteria' section".to_string(),
                "appetite" => "missing or invalid 'appetite'".to_string(),
                "target_modules" => "missing or empty 'target_modules'".to_string(),
                _ => m.to_string(),
            })
            .collect();

        return Err(QdevError::logical_failure(
            "readiness_criteria_unmet",
            format!(
                "Story '{}' readiness criteria unmet: {}",
                story_id,
                reasons.join(", ")
            ),
        )
        .with_details(serde_json::json!({
            "story_id": story_id,
            "missing": missing,
        })));
    }

    Ok(())
}

/// Validates that all dependencies are satisfied before transitioning from `ready` to `in-progress`.
fn validate_dependencies(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    frontmatter: &serde_json::Value,
) -> Result<(), QdevError> {
    let mut depends_on_ids: Vec<String> = Vec::new();

    // Read relations from frontmatter
    if let Some(relations) = frontmatter.get("relations").and_then(|r| r.as_object()) {
        if let Some(deps) = relations.get("depends_on") {
            if let Some(arr) = deps.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str() {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() && !depends_on_ids.contains(&trimmed.to_string()) {
                            depends_on_ids.push(trimmed.to_string());
                        }
                    }
                }
            } else if let Some(s) = deps.as_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() && !depends_on_ids.contains(&trimmed.to_string()) {
                    depends_on_ids.push(trimmed.to_string());
                }
            }
        }
    }

    let cache_dir_rel = storage
        .map(|s| s.cache_dir.as_str())
        .unwrap_or(".qdev/cache");
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");

    if cache_db_path.exists() {
        let store = SqliteStore::open(&cache_db_path)?;
        // Also inspect recorded store relations
        let records = store.get_relations_for_source(story_id)?;
        for row in records {
            if row.relation == "depends_on" && !depends_on_ids.contains(&row.target_id) {
                depends_on_ids.push(row.target_id);
            }
        }

        let mut blocking_ids = Vec::new();
        for dep_id in &depends_on_ids {
            let live_entity = store.get_live_entity_for_derivation(dep_id)?;
            let status = live_entity.and_then(|e| e.status);
            if status.as_deref() != Some("done") {
                blocking_ids.push(dep_id.clone());
            }
        }

        if !blocking_ids.is_empty() {
            return Err(QdevError::policy_refusal(
                "story_blocked",
                format!(
                    "Story '{}' is blocked by unmet dependencies: {}",
                    story_id,
                    blocking_ids.join(", ")
                ),
            )
            .with_details(serde_json::json!({
                "story_id": story_id,
                "blocking_ids": blocking_ids,
            })));
        }
    } else if !depends_on_ids.is_empty() {
        return Err(QdevError::policy_refusal(
            "story_blocked",
            format!(
                "Story '{}' is blocked by unmet dependencies: {}",
                story_id,
                depends_on_ids.join(", ")
            ),
        )
        .with_details(serde_json::json!({
            "story_id": story_id,
            "blocking_ids": depends_on_ids,
        })));
    }

    Ok(())
}

/// Extracts trimmed and deduplicated deferred work IDs from `relations.closes_dw`.
fn extract_closes_dw_ids(frontmatter: &serde_json::Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(relations) = frontmatter.get("relations").and_then(|r| r.as_object()) {
        if let Some(closes_dw) = relations.get("closes_dw") {
            match closes_dw {
                serde_json::Value::Array(arr) => {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            let trimmed = s.trim();
                            if !trimmed.is_empty() && !ids.contains(&trimmed.to_string()) {
                                ids.push(trimmed.to_string());
                            }
                        }
                    }
                }
                serde_json::Value::String(s) => {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() && !ids.contains(&trimmed.to_string()) {
                        ids.push(trimmed.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    ids
}

/// Sets all target entities in `relations.closes_dw` to `status: done`, `resolution: "<story_id>"`.
fn close_deferred_work(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    dw_ids: &[String],
    author: &Author,
) -> Result<Vec<String>, QdevError> {
    let mut closed = Vec::new();

    for dw_id in dw_ids {
        let dw_opts = EntityUpdateOptions {
            workspace_root: workspace_root.to_path_buf(),
            storage: storage.cloned(),
            entity_kind: Some(EntityKind::DeferredWork),
            entity_id: dw_id.clone(),
            status: Some("done".to_string()),
            title: None,
            custom_fields: vec![(
                "resolution".to_string(),
                crate::serde_yaml::Value::String(story_id.to_string()),
            )],
            section: None,
            section_file: None,
            if_version: None,
            author: author.clone(),
        };
        apply_entity_update(&dw_opts)?;
        closed.push(dw_id.clone());
    }

    Ok(closed)
}

/// Orchestrator for validating and executing story state transitions.
#[derive(Default)]
pub struct TransitionEngine {
    pre_hooks: Vec<Box<dyn PreTransitionHook>>,
    post_hooks: Vec<Box<dyn PostTransitionHook>>,
}

impl TransitionEngine {
    pub fn new() -> Self {
        Self {
            pre_hooks: Vec::new(),
            post_hooks: Vec::new(),
        }
    }

    pub fn register_pre_hook(&mut self, hook: Box<dyn PreTransitionHook>) {
        self.pre_hooks.push(hook);
    }

    pub fn register_post_hook(&mut self, hook: Box<dyn PostTransitionHook>) {
        self.post_hooks.push(hook);
    }

    pub fn add_pre_hook<H: PreTransitionHook + 'static>(&mut self, hook: H) {
        self.pre_hooks.push(Box::new(hook));
    }

    pub fn add_post_hook<H: PostTransitionHook + 'static>(&mut self, hook: H) {
        self.post_hooks.push(Box::new(hook));
    }

    pub fn transition(&self, options: &TransitionOptions) -> Result<TransitionPayload, QdevError> {
        // 1. Verify entity kind is "story"
        if options.entity_kind != "story" {
            return Err(QdevError::usage_error(format!(
                "Transition command only supports 'story' entities, got '{}'",
                options.entity_kind
            )));
        }

        // 2. Parse requested target status
        let target_state = StoryState::from_str(&options.target_status)?;

        // 3. Resolve and read story file
        let (_kind, id, file_path) = resolve_entity_file(
            &options.workspace_root,
            Some(EntityKind::Story),
            &options.story_id,
            options.storage.as_ref(),
        )?;

        let content = fs::read_to_string(&file_path).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to read entity file '{}': {}",
                    file_path.display(),
                    e
                ),
            )
        })?;

        let frontmatter = crate::schema::extract_frontmatter(&content).map_err(|e| {
            QdevError::logical_failure(
                "parse_error",
                format!(
                    "Failed to extract frontmatter from '{}': {}",
                    file_path.display(),
                    e
                ),
            )
        })?;

        let current_status_str = frontmatter
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                QdevError::logical_failure(
                    "missing_field",
                    format!("Story '{}' is missing required 'status' field", id),
                )
            })?;

        let from_state = StoryState::parse(current_status_str).map_err(|_| {
            QdevError::logical_failure(
                "invalid_status",
                format!(
                    "Story '{}' has invalid current status '{}'",
                    id, current_status_str
                ),
            )
        })?;

        // 4. Validate transition against state machine graph and justification requirement
        let transition_kind = classify_transition(from_state, target_state)?;

        match transition_kind {
            TransitionKind::TerminalJump => {
                let justification = options.justification.as_deref().unwrap_or("").trim();
                if justification.is_empty() {
                    return Err(QdevError::policy_refusal(
                        "needs_justification",
                        format!(
                            "Transition to terminal state '{}' requires non-empty justification (--justification <reason>)",
                            target_state.as_str()
                        ),
                    )
                    .with_details(serde_json::json!({
                        "story_id": id,
                        "target_status": target_state.as_str(),
                    })));
                }
            }
            TransitionKind::Backward => {
                let justification = options.justification.as_deref().unwrap_or("").trim();
                if justification.is_empty() {
                    return Err(QdevError::policy_refusal(
                        "needs_justification",
                        format!(
                            "Backward transition from '{}' to '{}' requires non-empty justification (--justification <reason>)",
                            from_state.as_str(),
                            target_state.as_str()
                        ),
                    )
                    .with_details(serde_json::json!({
                        "story_id": id,
                        "from_status": from_state.as_str(),
                        "target_status": target_state.as_str(),
                    })));
                }
            }
            TransitionKind::LegalForward => {}
        }

        // 5. Enforce draft -> ready prerequisites
        if from_state == StoryState::Draft && target_state == StoryState::Ready {
            validate_readiness_criteria(&content, &frontmatter, &id)?;
        }

        // 6. Enforce ready -> in-progress dependencies
        if from_state == StoryState::Ready && target_state == StoryState::InProgress {
            validate_dependencies(
                &options.workspace_root,
                options.storage.as_ref(),
                &id,
                &frontmatter,
            )?;
        }

        // 7. Pre-validate closes_dw targets if transitioning to done
        let dw_ids_to_close = if target_state == StoryState::Done {
            let ids = extract_closes_dw_ids(&frontmatter);
            for dw_id in &ids {
                resolve_entity_file(
                    &options.workspace_root,
                    Some(EntityKind::DeferredWork),
                    dw_id,
                    options.storage.as_ref(),
                )?;
            }
            ids
        } else {
            Vec::new()
        };

        // 8. Execute pre_transition hooks synchronously in order
        let ctx = TransitionContext {
            workspace_root: options.workspace_root.clone(),
            story_id: id.clone(),
            from_state,
            to_state: target_state,
            justification: options.justification.clone(),
            author: options.author.clone(),
            storage: options.storage.clone(),
        };

        for hook in &self.pre_hooks {
            hook.run(&ctx)?;
        }

        // 9. Mutate story frontmatter through apply_entity_update
        let update_opts = EntityUpdateOptions {
            workspace_root: options.workspace_root.clone(),
            storage: options.storage.clone(),
            entity_kind: Some(EntityKind::Story),
            entity_id: id.clone(),
            status: Some(target_state.as_str().to_string()),
            title: None,
            custom_fields: Vec::new(),
            section: None,
            section_file: None,
            if_version: options.if_version,
            author: options.author.clone(),
        };

        let update_res = apply_entity_update(&update_opts)?;

        // 10. If target is done, resolve closes_dw targets
        let closed_dw = if target_state == StoryState::Done {
            close_deferred_work(
                &options.workspace_root,
                options.storage.as_ref(),
                &id,
                &dw_ids_to_close,
                &options.author,
            )?
        } else {
            Vec::new()
        };

        // 11. Execute post_transition hooks
        for hook in &self.post_hooks {
            hook.run(&ctx, &update_res)?;
        }

        Ok(TransitionPayload {
            id,
            from_status: from_state.as_str().to_string(),
            to_status: target_state.as_str().to_string(),
            version: update_res.new_version,
            closed_dw,
        })
    }
}
