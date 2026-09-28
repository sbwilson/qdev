//! Token-budgeted context projection backing `qdev context` (Story 4.1).
//!
//! `qdev context <story-id> --phase <specify|develop|review> [--budget N] [--stats]
//! [--format md]` is the single source of agent context: it projects a deterministic,
//! token-budgeted payload of priority-ordered sections from the hydrated cache plus the
//! entity files, truncating lowest-priority sections first and listing what was truncated
//! (`docs/architecture.md` §12).
//!
//! The projection is **read-only**: `build_context` performs no cache writes, no file
//! writes, and no git mutations — its I/O is limited to reading entity files and (for the
//! review phase) `git diff`/`git status` invocations. Everything it reports is a pure
//! function of the cache state, the files on disk, and the options, so repeated builds
//! are byte-identical: fixed section priority order, ids/seq/path-sorted collections, and
//! no timestamps in the output.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::errors::QdevError;
use crate::gate::git::{
    is_inside_work_tree, merge_base_commit, ref_exists, run_git_unquoted,
    status_porcelain_untracked_all,
};
use crate::gate::{BUILTIN_GATE_DEPS, BUILTIN_GATE_HYGIENE, BUILTIN_GATE_SCOPE};
use crate::modules::ModuleRegistry;
use crate::schema::{extract_frontmatter, extract_frontmatter_str, EntityKind};
use crate::scratch::{estimate_tokens, summarize_scratch_entries, ScratchpadEntry};
use crate::store::{
    ConstraintRecord, EntityFilter, EntityRecord, GateRunRecord, RelationRecord, Store,
};

/// The development phases `qdev context` projects, per `docs/architecture.md` §12.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPhase {
    Specify,
    Develop,
    Review,
}

impl ContextPhase {
    /// Parses a phase name loosely (case-insensitive, trimmed); an unrecognized value is a
    /// usage error naming the valid phases.
    pub fn from_str_loose(s: &str) -> Result<Self, QdevError> {
        match s.trim().to_lowercase().as_str() {
            "specify" => Ok(Self::Specify),
            "develop" => Ok(Self::Develop),
            "review" => Ok(Self::Review),
            other => Err(QdevError::usage_error(format!(
                "Unknown --phase value '{}', expected one of: specify, develop, review",
                other
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Specify => "specify",
            Self::Develop => "develop",
            Self::Review => "review",
        }
    }

    /// Default token budget per phase when `--budget` is omitted (§12 typical budgets).
    pub fn default_budget(self) -> u32 {
        match self {
            Self::Specify => 800,
            Self::Develop => 1200,
            Self::Review => 2500,
        }
    }
}

/// Built-in hygiene directive embedded in the `develop` projection, verbatim from
/// `docs/compliance-and-safety.md` §1. Deliberately a code constant — the `[hygiene]`
/// config section does not own it (config-overridability arrives in a later story;
/// both ship this same default text).
pub const HYGIENE_DIRECTIVE: &str = "Write standard code comments. Cite entities with compact \
bracket tags such as `[E12S4]`, `[AD-43]`, `[DEC-2b91]`. Never write narrative history, story \
summaries, or review commentary in code; put reasoning in the scratchpad with `qdev scratch \
append`.";

/// Options controlling what `build_context` projects and how it is shaped.
#[derive(Debug, Clone, Copy)]
pub struct ContextOptions {
    /// The phase whose section set is projected (specify / develop / review).
    pub phase: ContextPhase,
    /// Explicit `--budget`; `None` selects the phase's default budget.
    pub budget: Option<u32>,
    /// `--stats`: include the `stats` object in the payload and the stats table in text mode.
    pub stats: bool,
}

/// One section of the projection, in fixed priority order (priority 1 = highest).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSection {
    /// Canonical section name for the phase (e.g. `story_spec`, `constraints`).
    pub name: String,
    /// 1-based priority rank within the phase; sections are emitted in ascending order.
    pub priority: u32,
    /// Estimated tokens of the content actually emitted (after any line-trim), via the
    /// documented `chars/4` estimator.
    pub tokens: u32,
    /// True when the budget walk trimmed or dropped this section; the `truncated` list then
    /// records its full pre-trim token count.
    pub truncated: bool,
    /// The section text; `"(none)"` when the section has no data.
    pub content: String,
}

/// A section the budget walk trimmed or dropped, recorded in drop order (lowest priority
/// first). `tokens` is the section's full pre-trim estimate — what the agent is owed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncatedSection {
    pub name: String,
    pub tokens: u32,
}

/// The `--stats` object: present in the payload only when `--stats` is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextStats {
    pub budget: u32,
    /// Estimated tokens of the emitted payload (sum of the sections' emitted tokens).
    pub total_tokens: u32,
    /// True when the total exceeds the budget (possible only via the first-section exemption).
    pub over_budget: bool,
    /// Per-section emitted tokens, name-sorted.
    pub sections: BTreeMap<String, u32>,
}

/// The `qdev context` payload: priority-ordered sections, the truncation record, and the
/// optional stats block. Rendered as a JSON envelope by the CLI, or as deterministic
/// text/Markdown via [`render_context_text`] / [`render_context_markdown`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPayload {
    /// The target story id.
    pub id: String,
    pub phase: ContextPhase,
    /// The effective budget: `--budget` or the phase default.
    pub budget: u32,
    /// Estimated tokens of the emitted payload (sum of the sections' emitted tokens).
    pub total_tokens: u32,
    /// Sections in priority order (1 = highest). Sections the budget dropped entirely are
    /// absent here and named in `truncated` instead.
    pub sections: Vec<ContextSection>,
    /// Sections trimmed or dropped by the budget walk, in drop order (lowest priority first).
    #[serde(default)]
    pub truncated: Vec<TruncatedSection>,
    /// The stats block, present only when `--stats` was requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stats: Option<ContextStats>,
}

/// A section with its full (untrimmed) content, before the budget walk.
struct SectionDraft {
    name: &'static str,
    /// `None` content means "no data": the section renders its `(none)` marker.
    content: Option<String>,
}

impl SectionDraft {
    fn none(name: &'static str) -> Self {
        Self {
            name,
            content: None,
        }
    }

    fn text(name: &'static str, content: &str) -> Self {
        if content.trim().is_empty() {
            Self::none(name)
        } else {
            Self {
                name,
                content: Some(content.to_string()),
            }
        }
    }
}

/// Reads a file and splits it into (parsed frontmatter, body). The file must be a markdown
/// document with frontmatter; any failure yields `None` — a dangling or unreadable reference
/// never fails the payload.
fn read_entity_content(
    workspace_root: &Path,
    source_path: &str,
) -> Option<(serde_json::Value, String)> {
    let content = std::fs::read_to_string(workspace_root.join(source_path)).ok()?;
    let (frontmatter_str, body) = extract_frontmatter_str(&content).ok()?;
    let frontmatter = extract_frontmatter(frontmatter_str).ok()?;
    Some((frontmatter, body.to_string()))
}

/// Extracts the body of a markdown `## <heading>` section (any case), up to the next level-2
/// heading. Returns the trimmed section text, or `None` when absent.
fn extract_markdown_section(body: &str, headings: &[&str]) -> Option<String> {
    let lines = body.lines().collect::<Vec<_>>();
    let mut start: Option<usize> = None;
    let mut in_code_block = false;
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            let heading = heading.trim();
            if start.is_none()
                && headings.iter().any(|h| h.eq_ignore_ascii_case(heading))
            {
                start = Some(idx);
            } else if start.is_some() {
                break; // reached the next level-2 heading
            }
        }
    }
    let start = start?;
    let mut end = lines.len();
    in_code_block = false;
    for (idx, line) in lines.iter().enumerate().skip(start + 1) {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if !in_code_block && trimmed.starts_with("## ") {
            end = idx;
            break;
        }
    }
    let text = lines[start + 1..end].to_vec().join("\n").trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

/// The `(none)` marker for a section with no data.
const NONE_MARKER: &str = "(none)";

// ---------------------------------------------------------------------------
// Section assembly
// ---------------------------------------------------------------------------

/// Full pre-trim text of the section content, `(none)`-marked when empty.
fn section_content(draft: &SectionDraft) -> String {
    draft
        .content
        .clone()
        .unwrap_or_else(|| NONE_MARKER.to_string())
}

/// Resolves the target story: a missing row is a logical error (exit 1), a row of another
/// kind is a usage error (exit 2) naming the kind.
fn resolve_story(store: &dyn Store, id: &str) -> Result<EntityRecord, QdevError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(QdevError::usage_error("Story ID cannot be empty"));
    }
    // Reporting path: the projection must surface the target even a retained stale row, so
    // the raw stale-inclusive read is deliberate here (the stale flag never appears in the
    // payload — the file, re-read below, is the source of truth for the spec body).
    #[allow(clippy::disallowed_methods)]
    let entity = store.get_entity(trimmed)?;
    let entity = entity.ok_or_else(|| {
        QdevError::logical_failure(
            "entity_not_found",
            format!("Entity '{}' not found — no story with that id in the workspace", trimmed),
        )
    })?;
    if entity.kind != EntityKind::Story {
        return Err(QdevError::usage_error(format!(
            "Entity '{}' is kind '{}', not 'story' — qdev context requires a story id",
            trimmed,
            entity.kind.as_str()
        )));
    }
    Ok(entity)
}

/// The story's spec body: the file's markdown body with the frontmatter stripped. An
/// unreadable or unparseable file degrades to a note — it never fails the payload.
fn story_spec_section(workspace_root: &Path, entity: &EntityRecord) -> String {
    match std::fs::read_to_string(workspace_root.join(&entity.source_path)) {
        Ok(content) => match extract_frontmatter_str(&content) {
            Ok((_frontmatter, body)) => {
                let trimmed = body.trim();
                if trimmed.is_empty() {
                    NONE_MARKER.to_string()
                } else {
                    trimmed.to_string()
                }
            }
            Err(_) => "(spec file has no parseable frontmatter — run `qdev sync`)"
                .to_string(),
        },
        Err(_) => "(spec file unreadable — run `qdev sync`)".to_string(),
    }
}

/// One line per constraint, own constraints first then inherited (tagged), id-ordered within
/// each group — the `qdev get` pattern (`get_constraints_for_owner` orders by id).
fn constraints_section(store: &dyn Store, entity: &EntityRecord) -> Result<String, QdevError> {
    let mut lines: Vec<String> = Vec::new();
    let own = store.get_constraints_for_owner(&entity.id)?;
    for c in &own {
        lines.push(format!("{} ({}): {}", c.id, c.kind, c.text));
    }
    if let Some(epic_id) = entity.epic_id.as_deref().filter(|e| !e.is_empty()) {
        let inherited = store.get_constraints_for_owner(epic_id)?;
        for c in &inherited {
            lines.push(format!(
                "{} ({}): {} [inherited from {}]",
                c.id, c.kind, c.text, epic_id
            ));
        }
    }
    Ok(render_lines(&lines))
}

fn render_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        NONE_MARKER.to_string()
    } else {
        lines.join("\n")
    }
}

/// Resolved module paths: each story target module against the configured registry,
/// id-sorted. An unregistered module is named as such rather than dropped.
fn modules_section(config: &Config, entity: &EntityRecord) -> Result<String, QdevError> {
    let raw: Vec<String> = entity
        .target_modules
        .as_deref()
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default();
    let mut ids = raw;
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(NONE_MARKER.to_string());
    }
    let registry = ModuleRegistry::from_config(config);
    let mut lines = Vec::new();
    for id in &ids {
        match registry.get(id) {
            Some(module) => lines.push(format!("{} -> {}", id, module.paths.join(", "))),
            None => lines.push(format!("{} (not registered in config)", id)),
        }
    }
    Ok(lines.join("\n"))
}

/// One ADR excerpt: the ADR's Rule and Prevents. The `decision` frontmatter field is the
/// Rule and `prevents` is the Prevents; body `## Rule`/`## Decision`/`## Prevents` sections
/// win over frontmatter when present. A dangling target keeps its id, omits the text, and
/// is marked `(unresolved)` — it never fails the payload.
fn adr_sections(
    workspace_root: &Path,
    store: &dyn Store,
    relation_rows: &[RelationRecord],
    summary_mode: bool,
) -> Result<String, QdevError> {
    let mut ids: Vec<String> = relation_rows
        .iter()
        .filter(|r| r.relation == "governed_by")
        .map(|r| r.target_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(NONE_MARKER.to_string());
    }

    let mut lines = Vec::new();
    for id in &ids {
        // Reporting path: a retained stale ADR row still names the file to read.
        #[allow(clippy::disallowed_methods)]
        let adr = store.get_entity(id)?;
        match adr {
            None => lines.push(format!("{} (unresolved)", id)),
            Some(adr) => {
                let title = adr.title.clone().unwrap_or_else(|| id.clone());
                let (rule, prevents) = adr_rule_prevents(workspace_root, &adr);
                if summary_mode {
                    // specify: a one-line summary — the Rule (`decision`) alone.
                    lines.push(match &rule {
                        Some(rule) => {
                            let single_line_rule = rule.replace("\r\n", " ").replace('\n', " ");
                            format!("{} \"{}\": {}", id, title, single_line_rule)
                        }
                        None => format!("{} \"{}\" (no decision recorded)", id, title),
                    });
                } else {
                    lines.push(format!("{} \"{}\"", id, title));
                    if let Some(rule) = &rule {
                        lines.push(format!("  Rule: {}", rule));
                    }
                    if let Some(prevents) = &prevents {
                        lines.push(format!("  Prevents: {}", prevents));
                    }
                    if rule.is_none() && prevents.is_none() {
                        lines.push("  (no rule or prevents recorded)".to_string());
                    }
                }
            }
        }
    }
    Ok(lines.join("\n"))
}

/// The (Rule, Prevents) pair for a resolved ADR: body sections win over frontmatter.
fn adr_rule_prevents(workspace_root: &Path, adr: &EntityRecord) -> (Option<String>, Option<String>) {
    let (rule, prevents) = match read_entity_content(workspace_root, &adr.source_path) {
        Some((frontmatter, body)) => {
            let body_rule = extract_markdown_section(&body, &["Rule", "Decision"]);
            let rule = body_rule
                .clone()
                .or_else(|| frontmatter.get("decision").and_then(|v| v.as_str()).map(str::to_string));
            let body_prevents = extract_markdown_section(&body, &["Prevents"]);
            let prevents = body_prevents.or_else(|| {
                frontmatter
                    .get("prevents")
                    .map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Array(items) => items
                            .iter()
                            .filter_map(|i| i.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                        _ => String::new(),
                    })
                    .filter(|s| !s.is_empty())
            });
            (
                rule.filter(|s| !s.trim().is_empty()),
                prevents.filter(|s| !s.trim().is_empty()),
            )
        }
        None => (None, None),
    };
    (rule, prevents)
}

/// Linked requirements: the story's `traces_to` targets with their titles, id-sorted.
/// A dangling target keeps its id and is marked `(unresolved)`.
fn requirements_section(
    store: &dyn Store,
    relation_rows: &[RelationRecord],
) -> Result<String, QdevError> {
    let mut ids: Vec<String> = relation_rows
        .iter()
        .filter(|r| r.relation == "traces_to")
        .map(|r| r.target_id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Ok(NONE_MARKER.to_string());
    }
    let mut lines = Vec::new();
    for id in &ids {
        // Reporting path: a retained stale requirement row still carries a last-known title.
        #[allow(clippy::disallowed_methods)]
        let requirement = store.get_entity(id)?;
        lines.push(match requirement {
            Some(r) => match r.title.filter(|t| !t.trim().is_empty()) {
                Some(title) => format!("{} \"{}\"", id, title),
                None => id.clone(),
            },
            None => format!("{} (unresolved)", id),
        });
    }
    Ok(lines.join("\n"))
}

/// The scratchpad summary: key decisions, transitions, and the last five entries, via the
/// documented `summarize_scratch_entries` — never the full ledger.
fn scratchpad_section(
    store: &dyn Store,
    story_id: &str,
) -> Result<String, QdevError> {
    let records = store.get_scratchpad_entries(story_id)?;
    let entries: Vec<ScratchpadEntry> = records.iter().map(ScratchpadEntry::from_record).collect();
    if entries.is_empty() {
        return Ok(NONE_MARKER.to_string());
    }
    let summary = summarize_scratch_entries(&entries, 5, None);
    let lines: Vec<String> = summary
        .iter()
        .map(|e| format!("#{} ({}): {}", e.seq, e.kind, e.text))
        .collect();
    Ok(render_lines(&lines))
}

/// Bound gates for the develop phase: the three built-ins in execution order first, then
/// the configured `[[gates]]` whose `on_transition` includes `review` (the transition
/// engine's develop bound-gate set), id-sorted. Each entry carries id, kind, and transitions.
fn gates_section(config: &Config) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("{} (builtin) [review]", BUILTIN_GATE_SCOPE));
    lines.push(format!("{} (builtin) [review]", BUILTIN_GATE_DEPS));
    lines.push(format!("{} (builtin) [review]", BUILTIN_GATE_HYGIENE));

    let mut configured: Vec<&crate::config::GateConfig> = config
        .gates
        .iter()
        .filter(|g| g.on_transition.iter().any(|t| t == "review"))
        .collect();
    configured.sort_by(|a, b| a.id.cmp(&b.id));
    let builtin_ids = [
        BUILTIN_GATE_SCOPE.to_string(),
        BUILTIN_GATE_DEPS.to_string(),
        BUILTIN_GATE_HYGIENE.to_string(),
    ];
    for gate in configured {
        if builtin_ids.contains(&gate.id) {
            continue; // a configured gate shadowing a built-in is listed once, above
        }
        let kind = gate
            .kind
            .clone()
            .unwrap_or_else(|| "command".to_string());
        let transitions = if gate.on_transition.is_empty() {
            "(none)".to_string()
        } else {
            format!("[{}]", gate.on_transition.join(", "))
        };
        let skipped = if gate.skip.unwrap_or(false) {
            " (skipped locally)"
        } else {
            ""
        };
        lines.push(format!("{} ({} {}){}", gate.id, kind, transitions, skipped));
    }
    lines.join("\n")
}

/// Sibling story titles for the specify phase: every story in the owning epic (the target
/// marked `(current)`), seq-then-id-sorted. Reporting path: a retained stale row still
/// carries its last-known title.
fn sibling_stories_section(
    store: &dyn Store,
    entity: &EntityRecord,
) -> Result<String, QdevError> {
    let Some(epic_id) = entity.epic_id.as_deref().filter(|e| !e.is_empty()) else {
        return Ok(NONE_MARKER.to_string());
    };
    #[allow(clippy::disallowed_methods)]
    let stories = store.list_entities(&EntityFilter {
        kind: Some(EntityKind::Story),
        epic_id: Some(epic_id.to_string()),
        ..Default::default()
    })?;
    let mut stories = stories;
    stories.sort_by(|a, b| {
        a.seq.unwrap_or(u32::MAX).cmp(&b.seq.unwrap_or(u32::MAX)).then(a.id.cmp(&b.id))
    });
    let lines: Vec<String> = stories
        .iter()
        .map(|s| {
            let title = s.title.clone().unwrap_or_else(|| "(untitled)".to_string());
            let current = if s.id == entity.id { " (current)" } else { "" };
            let status = s.status.clone().unwrap_or_else(|| "unknown".to_string());
            format!("{} \"{}\" ({}){}", s.id, title, status, current)
        })
        .collect();
    Ok(render_lines(&lines))
}

/// Epic goal for the specify phase: the `goal` frontmatter field, or the body's `## Goal`
/// section, under the epic's title. An absent or unreadable epic degrades to `(none)`.
fn epic_goal_section(
    workspace_root: &Path,
    store: &dyn Store,
    epic_id: Option<&str>,
) -> Result<String, QdevError> {
    let Some(epic_id) = epic_id.filter(|e| !e.is_empty()) else {
        return Ok(NONE_MARKER.to_string());
    };
    // Reporting path: a retained stale epic row still names the file to read.
    #[allow(clippy::disallowed_methods)]
    let epic = store.get_entity(epic_id)?;
    let Some(epic) = epic else {
        return Ok(NONE_MARKER.to_string());
    };
    let title = epic.title.clone().unwrap_or_else(|| epic_id.to_string());
    let mut lines = vec![format!("{} \"{}\"", epic_id, title)];
    if let Some((frontmatter, body)) = read_entity_content(workspace_root, &epic.source_path) {
        let goal = frontmatter
            .get("goal")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| extract_markdown_section(&body, &["Goal"]))
            .filter(|g| !g.trim().is_empty());
        if let Some(goal) = goal {
            lines.push(goal);
        }
    }
    Ok(lines.join("\n"))
}

/// Epic constraints for the specify phase, id-ordered: full ids with kind and text.
fn epic_constraints_section(
    store: &dyn Store,
    epic_id: Option<&str>,
) -> Result<String, QdevError> {
    let Some(epic_id) = epic_id.filter(|e| !e.is_empty()) else {
        return Ok(NONE_MARKER.to_string());
    };
    let constraints = store.get_constraints_for_owner(epic_id)?;
    if constraints.is_empty() {
        return Ok(NONE_MARKER.to_string());
    }
    let lines: Vec<String> = constraints
        .iter()
        .map(|c: &ConstraintRecord| format!("{} ({}): {}", c.id, c.kind, c.text))
        .collect();
    Ok(lines.join("\n"))
}

/// One gate run receipt line: gate id, status, and summary when the run carries one.
fn gate_receipt_line(run: &GateRunRecord) -> String {
    let status = run.status.clone().unwrap_or_else(|| "unknown".to_string());
    match &run.summary {
        Some(summary) if !summary.trim().is_empty() => {
            format!("{} [{}] {}", run.gate_id, status, summary)
        }
        _ => format!("{} [{}]", run.gate_id, status),
    }
}

/// The latest run per gate: the store returns a story's runs newest-first, so the first
/// sight of a gate is its latest run.
fn latest_runs_per_gate(runs: Vec<GateRunRecord>) -> BTreeMap<String, GateRunRecord> {
    let mut latest: BTreeMap<String, GateRunRecord> = BTreeMap::new();
    for run in runs {
        latest.entry(run.gate_id.clone()).or_insert(run);
    }
    latest
}

/// Gate receipts for the review phase: the latest run per gate, id-sorted.
fn gate_receipts_section(store: &dyn Store, story_id: &str) -> Result<String, QdevError> {
    let runs = store.get_gate_runs_for_story(story_id)?;
    let lines: Vec<String> = latest_runs_per_gate(runs)
        .values()
        .map(gate_receipt_line)
        .collect();
    Ok(render_lines(&lines))
}

/// Evidence paths for the review phase: the evidence paths of the latest runs, path-sorted.
fn evidence_section(store: &dyn Store, story_id: &str) -> Result<String, QdevError> {
    let runs = store.get_gate_runs_for_story(story_id)?;
    let mut paths: Vec<String> = latest_runs_per_gate(runs)
        .values()
        .map(|r| r.evidence_path.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    paths.sort();
    paths.dedup();
    Ok(render_lines(&paths))
}

/// Diff summary for the review phase: files changed since the merge-base with
/// `[git] integration_branch`, per-file +/- line counts from `git diff --numstat`, plus
/// uncommitted and untracked changes — the hygiene `resolve_diff_files` pattern. An
/// unresolvable baseline yields an empty section with a reason note, never an error.
fn diff_section(workspace_root: &Path, config: &Config) -> String {
    let integration_branch = &config.git.integration_branch;

    if !is_inside_work_tree(workspace_root) {
        return "(empty: not inside a git work tree)".to_string();
    }

    let diff_target = merge_base_commit(workspace_root, integration_branch)
        .or_else(|| ref_exists(workspace_root, integration_branch).then(|| integration_branch.to_string()));

    let Some(target) = diff_target else {
        return format!(
            "(empty: cannot resolve a diff baseline against '{}')",
            integration_branch
        );
    };

    let mut changes: BTreeMap<String, (u32, u32)> = BTreeMap::new();

    let parse_numstat = |output: &std::process::Output, changes: &mut BTreeMap<String, (u32, u32)>| {
        if !output.status.success() {
            return; // a failed diff contributes nothing; the baseline itself resolved fine
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let mut parts = line.splitn(3, '\t');
            let (added, deleted, path) = match (parts.next(), parts.next(), parts.next()) {
                (Some(a), Some(d), Some(p)) => (a, d, p),
                _ => continue,
            };
            let path = path.trim();
            if path.is_empty() {
                continue;
            }
            // Binary files report `-` for both counts.
            let added: u32 = added.trim().parse().unwrap_or(0);
            let deleted: u32 = deleted.trim().parse().unwrap_or(0);
            let entry = changes.entry(path.to_string()).or_default();
            entry.0 = entry.0.saturating_add(added);
            entry.1 = entry.1.saturating_add(deleted);
        }
    };

    // The three diffs below are disjoint legs of one chain — baseline to HEAD (committed),
    // HEAD to index (staged), index to worktree (unstaged) — so summing per path cannot
    // double-count a file.
    if let Ok(output) = run_git_unquoted(
        workspace_root,
        &[
            "diff",
            "--numstat",
            "--no-renames",
            "--relative",
            target.as_str(),
            "HEAD",
        ],
    ) {
        parse_numstat(&output, &mut changes);
    }
    if let Ok(output) = run_git_unquoted(
        workspace_root,
        &["diff", "--cached", "--numstat", "--no-renames", "--relative"],
    ) {
        parse_numstat(&output, &mut changes);
    }
    if let Ok(output) = run_git_unquoted(
        workspace_root,
        &["diff", "--numstat", "--no-renames", "--relative"],
    ) {
        parse_numstat(&output, &mut changes);
    }

    // Untracked files ("??" entries) carry no numstat row on any leg: count their lines as
    // additions. A path is never both untracked and on a diff leg, so this cannot double-count.
    if let Ok(output) = status_porcelain_untracked_all(workspace_root) {
        if output.status.success() {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                if !line.starts_with("??") || line.len() < 4 {
                    continue;
                }
                let raw_path = line[2..].trim();
                let raw_path = if let Some(idx) = raw_path.find(" -> ") {
                    &raw_path[idx + 4..]
                } else {
                    raw_path
                };
                let path = raw_path.trim_matches('"');
                if path.is_empty() {
                    continue;
                }
                let entry = changes.entry(path.to_string()).or_default();
                let file = workspace_root.join(path);
                if let Ok(content) = std::fs::read_to_string(&file) {
                    entry.0 = entry.0.max(content.lines().count() as u32);
                }
            }
        }
    }

    if changes.is_empty() {
        return "(none)".to_string();
    }

    let lines: Vec<String> = changes
        .iter()
        .map(|(path, (added, deleted))| format!("{} +{} -{}", path, added, deleted))
        .collect();
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// Budget walk
// ---------------------------------------------------------------------------

/// Applies the token budget to the priority-ordered sections, in place.
///
/// The walk goes highest to lowest priority. Each section keeps complete lines that fit the
/// remaining budget — deterministic line trimming only, never mid-line splits. A section
/// that cannot fit at all is dropped from the payload and recorded in `truncated` (drop
/// order, name + full token count); a partially fitting section keeps its kept lines, is
/// flagged `truncated`, and is likewise recorded. The first section is exempt: when it
/// alone exceeds the budget it is kept whole (the payload may then overrun; `--stats`
/// surfaces it).
fn apply_budget(
    sections: &mut Vec<ContextSection>,
    budget: u32,
) -> Vec<TruncatedSection> {
    let mut truncated: Vec<TruncatedSection> = Vec::new();
    let mut used: u32 = 0;

    for (index, section) in sections.iter_mut().enumerate() {
        let full_tokens = estimate_tokens(&section.content) as u32;
        let remaining = budget.saturating_sub(used);

        if index == 0 || full_tokens <= remaining {
            // The first section is exempt from trimming; every other section included here
            // fits whole.
            section.tokens = full_tokens;
            section.truncated = false;
            used = used.saturating_add(full_tokens);
            continue;
        }

        // Trim: keep the longest prefix of complete lines that fits the remaining budget.
        let mut kept = String::new();
        for line in section.content.lines() {
            let candidate = if kept.is_empty() {
                line.to_string()
            } else {
                format!("{}\n{}", kept, line)
            };
            if estimate_tokens(&candidate) as u32 <= remaining {
                kept = candidate;
            } else {
                break;
            }
        }

        if kept.is_empty() {
            // Does not fit at all: drop the section and record it in drop order.
            truncated.push(TruncatedSection {
                name: section.name.clone(),
                tokens: full_tokens,
            });
            section.content.clear();
            section.tokens = 0;
            section.truncated = true;
        } else {
            truncated.push(TruncatedSection {
                name: section.name.clone(),
                tokens: full_tokens,
            });
            section.content = kept;
            section.tokens = estimate_tokens(&section.content) as u32;
            section.truncated = true;
        }

        used = used.saturating_add(section.tokens);
    }

    // A dropped section is removed from the payload entirely; the truncation record names it
    // in drop order.
    *sections = sections
        .drain(..)
        .filter(|s| !s.truncated || !s.content.is_empty())
        .collect();

    truncated
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

/// Builds the `qdev context` projection for a story.
///
/// Pure core: reads the cache store, the entity files, the config, and (review phase only)
/// git diff/status. It writes nothing and never prompts, so it is safe in non-interactive
/// mode. The result is deterministic: repeated builds over the same workspace are
/// byte-identical.
pub fn build_context(
    workspace_root: &Path,
    store: &dyn Store,
    config: &Config,
    options: &ContextOptions,
    id: &str,
) -> Result<ContextPayload, QdevError> {
    let entity = resolve_story(store, id)?;

    let budget = match options.budget {
        Some(0) => {
            return Err(QdevError::usage_error(
                "--budget must be a positive token count",
            ))
        }
        Some(b) => b,
        None => options.phase.default_budget(),
    };

    let relation_rows = store.get_relations_for_source(&entity.id)?;

    // Phase section sets, in fixed priority order (1 = highest). The set depends only on the
    // phase, never on the data shape: an empty section is still present, `(none)`-marked.
    let drafts: Vec<SectionDraft> = match options.phase {
        ContextPhase::Specify => vec![
            SectionDraft::text(
                "epic_goal",
                &epic_goal_section(workspace_root, store, entity.epic_id.as_deref())?,
            ),
            SectionDraft::text(
                "epic_constraints",
                &epic_constraints_section(store, entity.epic_id.as_deref())?,
            ),
            SectionDraft::text(
                "sibling_stories",
                &sibling_stories_section(store, &entity)?,
            ),
            SectionDraft::text(
                "adr_summaries",
                &adr_sections(workspace_root, store, &relation_rows, true)?,
            ),
            SectionDraft::text(
                "requirements",
                &requirements_section(store, &relation_rows)?,
            ),
        ],
        ContextPhase::Develop | ContextPhase::Review => {
            let drafts = vec![
                SectionDraft::text("story_spec", &story_spec_section(workspace_root, &entity)),
                SectionDraft::text("constraints", &constraints_section(store, &entity)?),
                SectionDraft::text("modules", &modules_section(config, &entity)?),
                SectionDraft::text(
                    "adr_excerpts",
                    &adr_sections(workspace_root, store, &relation_rows, false)?,
                ),
                SectionDraft::text(
                    "requirements",
                    &requirements_section(store, &relation_rows)?,
                ),
                SectionDraft::text("scratchpad", &scratchpad_section(store, &entity.id)?),
                SectionDraft::text("gates", &gates_section(config)),
                SectionDraft::text("hygiene", HYGIENE_DIRECTIVE),
            ];
            if options.phase == ContextPhase::Review {
                let mut review = drafts;
                review.push(SectionDraft::text(
                    "diff",
                    &diff_section(workspace_root, config),
                ));
                review.push(SectionDraft::text(
                    "gate_receipts",
                    &gate_receipts_section(store, &entity.id)?,
                ));
                review.push(SectionDraft::text(
                    "evidence",
                    &evidence_section(store, &entity.id)?,
                ));
                review
            } else {
                drafts
            }
        }
    };

    let mut sections = drafts
        .iter()
        .enumerate()
        .map(|(index, draft)| {
            let content = section_content(draft);
            ContextSection {
                name: draft.name.to_string(),
                priority: (index + 1) as u32,
                tokens: estimate_tokens(&content) as u32,
                truncated: false,
                content,
            }
        })
        .collect::<Vec<_>>();

    let truncated = apply_budget(&mut sections, budget);

    let total_tokens: u32 = sections.iter().map(|s| s.tokens).sum();

    let stats = if options.stats {
        Some(ContextStats {
            budget,
            total_tokens,
            over_budget: total_tokens > budget,
            sections: sections
                .iter()
                .map(|s| (s.name.clone(), s.tokens))
                .collect(),
        })
    } else {
        None
    };

    Ok(ContextPayload {
        id: entity.id.clone(),
        phase: options.phase,
        budget,
        total_tokens,
        sections,
        truncated,
        stats,
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// The deterministic text rendering of a context payload (the default `qdev context`
/// output): a header, one `== name ==` block per section in priority order, the stats
/// table when `--stats` set it, and the truncation record.
pub fn render_context_text(payload: &ContextPayload) -> String {
    let mut text = String::new();
    text.push_str(&format!(
        "Context for {} (phase: {})\n",
        payload.id,
        payload.phase.as_str()
    ));
    text.push_str(&format!(
        "Budget: {} tokens, total: {} tokens ({})\n\n",
        payload.budget,
        payload.total_tokens,
        if payload.total_tokens > payload.budget {
            "over budget"
        } else {
            "within budget"
        }
    ));

    for section in &payload.sections {
        text.push_str(&format!("== {} ==\n", section.name));
        text.push_str(&section.content);
        if !section.content.ends_with('\n') {
            text.push('\n');
        }
        text.push('\n');
    }

    if let Some(stats) = &payload.stats {
        text.push_str("Stats (estimated tokens)\n");
        for (name, tokens) in &stats.sections {
            text.push_str(&format!("  {:<16} {}\n", name, tokens));
        }
        text.push_str(&format!(
            "  {:<16} {}\n",
            "budget",
            stats.budget
        ));
        text.push_str(&format!(
            "  {:<16} {} ({})\n",
            "total",
            stats.total_tokens,
            if stats.over_budget {
                "over budget"
            } else {
                "within budget"
            }
        ));
        text.push('\n');
    }

    if !payload.truncated.is_empty() {
        text.push_str("Truncated (drop order)\n");
        for entry in &payload.truncated {
            text.push_str(&format!("  {} ({} tokens)\n", entry.name, entry.tokens));
        }
    }

    text
}

/// The deterministic Markdown rendering of the same payload (`--format md`): a title, one
/// `## section` per section in priority order, the stats table when `--stats` set it, and
/// the truncation list.
pub fn render_context_markdown(payload: &ContextPayload) -> String {
    let mut md = String::new();
    md.push_str(&format!(
        "# Context: {} (phase: {})\n\n",
        payload.id,
        payload.phase.as_str()
    ));
    md.push_str(&format!(
        "Budget: {} tokens; total: {} tokens ({}).\n\n",
        payload.budget,
        payload.total_tokens,
        if payload.total_tokens > payload.budget {
            "over budget"
        } else {
            "within budget"
        }
    ));

    for section in &payload.sections {
        md.push_str(&format!("## {}\n\n{}", section.name, section.content));
        if !section.content.ends_with('\n') {
            md.push('\n');
        }
        md.push('\n');
    }

    if let Some(stats) = &payload.stats {
        md.push_str("## stats\n\n");
        md.push_str("| section | tokens |\n| --- | --- |\n");
        for (name, tokens) in &stats.sections {
            md.push_str(&format!("| {} | {} |\n", name, tokens));
        }
        md.push_str(&format!("| budget | {} |\n", stats.budget));
        md.push_str(&format!(
            "| total | {} ({}) |\n",
            stats.total_tokens,
            if stats.over_budget {
                "over budget"
            } else {
                "within budget"
            }
        ));
        md.push('\n');
    }

    if !payload.truncated.is_empty() {
        md.push_str("## truncated\n\n");
        md.push_str("Trimmed or dropped by the budget walk, in drop order:\n\n");
        for entry in &payload.truncated {
            md.push_str(&format!("- {} ({} tokens)\n", entry.name, entry.tokens));
        }
    }

    md
}
