use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::schema::EntityKind;
use crate::store::{ScratchpadRecord, SqliteStore, Store};
use crate::write::{
    acquire_write_lock, apply_entity_update, apply_entity_update_checked, current_iso8601,
    directory_for_kind, has_markdown_heading, resolve_entity_file, write_file_atomic, Author,
    EntityUpdateOptions, EntityUpdateResult,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
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
        // Nothing has been checked: with no cache there is no record of any dependency's
        // status, so calling them unmet tells the operator the wrong thing. Say that the
        // evidence is missing, where it lives, and how to build it.
        return Err(QdevError::policy_refusal(
            "story_blocked",
            format!(
                "Story '{}' cannot move to in-progress: no cache at {}, so the status of {} dependencies ({}) has not been checked. Run `qdev sync` and retry.",
                story_id,
                cache_db_path.display(),
                depends_on_ids.len(),
                depends_on_ids.join(", ")
            ),
        )
        .with_details(serde_json::json!({
            "story_id": story_id,
            "blocking_ids": depends_on_ids,
            "cache": cache_db_path.display().to_string(),
            "checked": false,
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

/// The outcome of judging a requested transition against a story file.
struct TransitionDecision {
    from_state: StoryState,
    kind: TransitionKind,
    /// `closes_dw` targets re-derived from the content that was judged.
    dw_ids: Vec<String>,
}

/// Judges a requested story transition against story content as it currently stands: the state
/// machine edge, the justification an out-of-line move needs, the `draft -> ready` readiness
/// gate, the `ready -> in-progress` blocking dependency gate, and the existence of every
/// `closes_dw` target.
///
/// `TransitionEngine::transition` calls this twice — once from the read that decides the
/// transition, and again under the write lock via `apply_entity_update_checked`. The deciding
/// read and the commit can straddle a concurrent commit, and the write path patches the
/// caller's pre-computed status without judging it, so a single pre-lock decision lets two
/// `qdev transition` runs on one story both succeed with the later one silently overwriting
/// the first — including reviving a story that just moved to a terminal state.
fn validate_story_transition(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    content: &str,
    story_id: &str,
    target_state: StoryState,
    justification: &str,
) -> Result<TransitionDecision, QdevError> {
    let frontmatter = crate::schema::extract_frontmatter(content).map_err(|e| {
        QdevError::logical_failure(
            "parse_error",
            format!("Failed to extract frontmatter from '{}': {}", story_id, e),
        )
    })?;

    let current_status_str = frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            QdevError::logical_failure(
                "missing_field",
                format!("Story '{}' is missing required 'status' field", story_id),
            )
        })?;

    let from_state = StoryState::parse(current_status_str).map_err(|_| {
        QdevError::logical_failure(
            "invalid_status",
            format!(
                "Story '{}' has invalid current status '{}'",
                story_id, current_status_str
            ),
        )
    })?;

    // 1. State machine edge, and the justification a backward or terminal move needs.
    let kind = classify_transition(from_state, target_state)?;

    match kind {
        TransitionKind::LegalForward => {}
        TransitionKind::TerminalJump => {
            if justification.is_empty() {
                return Err(QdevError::policy_refusal(
                    "needs_justification",
                    format!(
                        "Transition to terminal state '{}' requires non-empty justification (--justification <reason>)",
                        target_state.as_str()
                    ),
                )
                .with_details(serde_json::json!({
                    "story_id": story_id,
                    "target_status": target_state.as_str(),
                })));
            }
        }
        TransitionKind::Backward => {
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
                    "story_id": story_id,
                    "from_status": from_state.as_str(),
                    "target_status": target_state.as_str(),
                })));
            }
        }
    }

    // 2. `draft -> ready` prerequisites, judged from the same content that decided the edge.
    if from_state == StoryState::Draft && target_state == StoryState::Ready {
        validate_readiness_criteria(content, &frontmatter, story_id)?;
    }

    // 3. `ready -> in-progress` blocking dependencies.
    if from_state == StoryState::Ready && target_state == StoryState::InProgress {
        validate_dependencies(workspace_root, storage, story_id, &frontmatter)?;
    }

    // 4. Every `closes_dw` target must exist before a story can reach `done`.
    let dw_ids = if target_state == StoryState::Done {
        let ids = extract_closes_dw_ids(&frontmatter);
        for dw_id in &ids {
            resolve_entity_file(
                workspace_root,
                Some(EntityKind::DeferredWork),
                dw_id,
                storage,
            )?;
        }
        ids
    } else {
        Vec::new()
    };

    Ok(TransitionDecision {
        from_state,
        kind,
        dw_ids,
    })
}

/// Sets every not-yet-closed target in `relations.closes_dw` to `status: done`,
/// `resolution: "<story_id>"`, and returns the IDs it actually closed.
///
/// This runs before the story is committed (see `TransitionEngine::transition`) so a DW that
/// cannot be written fails while the story is still in its previous state and the transition
/// can be re-driven. A DW someone else already closed is skipped rather than re-stamped, which
/// keeps a retry idempotent and leaves the `resolution` that closed it first intact.
fn close_deferred_work(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    dw_ids: &[String],
    author: &Author,
) -> Result<Vec<String>, QdevError> {
    let mut closed = Vec::new();

    for dw_id in dw_ids {
        let (_kind, _id, file_path) = resolve_entity_file(
            workspace_root,
            Some(EntityKind::DeferredWork),
            dw_id,
            storage,
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

        if frontmatter
            .get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|status| status == "done")
        {
            continue;
        }

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

        let justification = options
            .justification
            .as_deref()
            .map(str::trim)
            .unwrap_or("");

        // 3. Resolve and read the story file, then judge the transition from it.
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

        let decision = validate_story_transition(
            &options.workspace_root,
            options.storage.as_ref(),
            &content,
            &id,
            target_state,
            justification,
        )?;

        // 4. Close `closes_dw` targets before committing the story: a DW that cannot be
        //    written then fails while the story is still where it was, so the transition can
        //    be re-driven instead of leaving a `done` story with open debt.
        let closed_dw = if target_state == StoryState::Done {
            close_deferred_work(
                &options.workspace_root,
                options.storage.as_ref(),
                &id,
                &decision.dw_ids,
                &options.author,
            )?
        } else {
            Vec::new()
        };

        // 5. Auto-release the story's lease on a terminal target (Story 2.3).
        if target_state.is_terminal() {
            let _ = crate::lease::auto_release_lease(&options.workspace_root, &id);
        }

        // 6. Execute pre_transition hooks synchronously in order; the first error aborts
        //    before any mutation.
        let ctx = TransitionContext {
            workspace_root: options.workspace_root.clone(),
            story_id: id.clone(),
            from_state: decision.from_state,
            to_state: target_state,
            justification: options.justification.clone(),
            author: options.author.clone(),
            storage: options.storage.clone(),
        };

        for hook in &self.pre_hooks {
            hook.run(&ctx)?;
        }

        // 7. Commit through the write path that re-validates under the lock. The decision
        //    above was taken from a read made before the lock, and the patch below carries a
        //    status computed from it, so the same judgement runs again against the content
        //    this write actually reads and patches.
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

        let (update_res, revalidated) = apply_entity_update_checked(&update_opts, |fresh| {
            validate_story_transition(
                &options.workspace_root,
                options.storage.as_ref(),
                fresh,
                &id,
                target_state,
                justification,
            )
        })?;

        // 8. Record why an out-of-line move happened: a backward move or a jump to a terminal
        //    state stays on the decision ledger and in the story's scratchpad. Judged from the
        //    revalidated decision so the record describes the transition that actually
        //    committed, and runs after the story commit — a record of a transition that did
        //    not happen would be worse than none.
        let decision_id = if matches!(
            revalidated.kind,
            TransitionKind::Backward | TransitionKind::TerminalJump
        ) {
            let timestamp = current_iso8601();

            let cache_dir_rel = options
                .storage
                .as_ref()
                .map(|s| s.cache_dir.as_str())
                .unwrap_or(".qdev/cache");
            let lock_path = options
                .workspace_root
                .join(cache_dir_rel)
                .join("write.lock");
            let lock_guard = if lock_path.parent().map(|p| p.is_dir()).unwrap_or(false) {
                Some(acquire_write_lock(&lock_path, Duration::from_millis(5000))?)
            } else {
                None
            };

            append_scratchpad_entry(
                &options.workspace_root,
                options.storage.as_ref(),
                &id,
                justification,
                &options.author,
                &timestamp,
            )?;
            drop(lock_guard);

            let dec_id = create_transition_decision(
                &options.workspace_root,
                options.storage.as_ref(),
                &id,
                revalidated.from_state,
                target_state,
                justification,
                &options.author,
                &timestamp,
            )?;

            Some(dec_id)
        } else {
            None
        };

        // 9. Execute post_transition hooks in registered order, seeing the state the write
        //    actually landed in.
        let post_ctx = TransitionContext {
            from_state: revalidated.from_state,
            ..ctx
        };

        for hook in &self.post_hooks {
            hook.run(&post_ctx, &update_res)?;
        }

        Ok(TransitionPayload {
            id,
            from_status: revalidated.from_state.as_str().to_string(),
            to_status: target_state.as_str().to_string(),
            version: update_res.new_version,
            closed_dw,
            decision_id,
        })
    }
}

/// Appends a transition entry to the story's dedicated scratchpad JSONL ledger at
/// `docs/state/scratch/<story-id>.jsonl` and syncs the row to the SQLite cache if present.
pub fn append_scratchpad_entry(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    justification: &str,
    author: &Author,
    timestamp: &str,
) -> Result<u32, QdevError> {
    let scratch_dir = workspace_root.join(directory_for_kind(storage, EntityKind::Scratchpad));
    if !scratch_dir.exists() {
        fs::create_dir_all(&scratch_dir).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create scratchpad directory '{}': {}",
                    scratch_dir.display(),
                    e
                ),
            )
        })?;
    }

    let scratch_file_path = scratch_dir.join(format!("{}.jsonl", story_id));
    let mut next_seq: u32 = 1;
    let mut staged = String::new();

    if scratch_file_path.exists() {
        let existing_content = fs::read_to_string(&scratch_file_path).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to read scratchpad file '{}': {}",
                    scratch_file_path.display(),
                    e
                ),
            )
        })?;

        let mut max_seq: u32 = 0;
        for (line_idx, line) in existing_content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let val = serde_json::from_str::<serde_json::Value>(trimmed).map_err(|e| {
                QdevError::logical_failure(
                    "parse_error",
                    format!(
                        "Malformed scratchpad entry in '{}' at line {}: {}",
                        scratch_file_path.display(),
                        line_idx + 1,
                        e
                    ),
                )
            })?;
            let seq_u64 = val.get("seq").and_then(|v| v.as_u64()).ok_or_else(|| {
                QdevError::logical_failure(
                    "parse_error",
                    format!(
                        "Malformed scratchpad entry in '{}' at line {}: missing or invalid 'seq'",
                        scratch_file_path.display(),
                        line_idx + 1
                    ),
                )
            })?;
            let seq_u32 = u32::try_from(seq_u64).map_err(|_| {
                QdevError::logical_failure(
                    "parse_error",
                    format!(
                        "Scratchpad sequence out of bounds in '{}' at line {}",
                        scratch_file_path.display(),
                        line_idx + 1
                    ),
                )
            })?;
            max_seq = max_seq.max(seq_u32);
        }
        next_seq = max_seq.checked_add(1).ok_or_else(|| {
            QdevError::logical_failure(
                "overflow",
                format!(
                    "Scratchpad sequence overflow in '{}'",
                    scratch_file_path.display()
                ),
            )
        })?;
        staged = existing_content;
    }

    let entry = serde_json::json!({
        "seq": next_seq,
        "at": timestamp,
        "author": {
            "type": author.author_type,
            "id": author.id,
        },
        "kind": "transition",
        "text": justification,
    });

    let entry_str = serde_json::to_string(&entry).map_err(|e| {
        QdevError::infrastructure_failure(
            "serialization_error",
            format!("Failed to serialize scratchpad entry: {}", e),
        )
    })?;

    if !staged.is_empty() && !staged.ends_with('\n') {
        staged.push('\n');
    }
    staged.push_str(&entry_str);
    staged.push('\n');

    write_file_atomic(&scratch_file_path, &staged)?;

    let cache_dir_rel = storage
        .map(|s| s.cache_dir.as_str())
        .unwrap_or(".qdev/cache");
    let cache_db_path = workspace_root.join(cache_dir_rel).join("cache.sqlite");
    if cache_db_path.is_file() {
        let store = SqliteStore::open(&cache_db_path)?;
        let record = ScratchpadRecord {
            story_id: story_id.to_string(),
            seq: next_seq,
            at: timestamp.to_string(),
            author_type: Some(author.author_type.clone()),
            author_id: Some(author.id.clone()),
            kind: Some("transition".to_string()),
            text: Some(justification.to_string()),
        };
        store.upsert_scratchpad_entry(&record)?;
    }

    Ok(next_seq)
}

/// Creates a committed DEC- record in `docs/state/decisions/` recording the rationale for a
/// story transition that needed justification — a backward move or a jump to a terminal state.
/// Validated against the `decision.json` schema and synced to the SQLite cache when one exists.
#[allow(clippy::too_many_arguments)]
pub fn create_transition_decision(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    from_state: StoryState,
    to_state: StoryState,
    justification: &str,
    author: &Author,
    timestamp: &str,
) -> Result<String, QdevError> {
    // A terminal jump is its own kind of ruling: the story is not being moved back or
    // rejected, it is being ended. Recording it as `pivot` would hide the difference.
    let decision_type = match to_state {
        StoryState::Abandoned => "story_abandoned",
        StoryState::Superseded => "story_superseded",
        _ if from_state == StoryState::Review => "review_rejection",
        _ => "pivot",
    };

    let title = match decision_type {
        "story_abandoned" => format!("Story {} abandoned", story_id),
        "story_superseded" => format!("Story {} superseded", story_id),
        "review_rejection" => format!("Review rejection on story {}", story_id),
        _ => format!("Pivot on story {}", story_id),
    };

    let trajectory = format!("{} -> {}", from_state.as_str(), to_state.as_str());

    let input = crate::decision::DecisionInput {
        subject_id: story_id.to_string(),
        decision_type: decision_type.to_string(),
        topic: None,
        context: Some(trajectory),
        ruling: justification.to_string(),
        author: author.clone(),
        title: Some(title),
        timestamp: Some(timestamp.to_string()),
        validate_subject: false,
    };

    let payload = crate::decision::log_decision(workspace_root, storage, &input)?;
    Ok(payload.id)
}

/// Alias for [`create_transition_decision`].
#[allow(clippy::too_many_arguments)]
pub fn record_transition_decision(
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    story_id: &str,
    from_state: StoryState,
    to_state: StoryState,
    justification: &str,
    author: &Author,
    timestamp: &str,
) -> Result<String, QdevError> {
    create_transition_decision(
        workspace_root,
        storage,
        story_id,
        from_state,
        to_state,
        justification,
        author,
        timestamp,
    )
}
