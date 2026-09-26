//! Core Impact Analysis engine per Story 3.10.
//!
//! Computes change blast radius across active concurrent stories, requirements,
//! hazards, ADRs, reverse dependents with depth tracking, overlapping verification gates,
//! and inline code citations.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::config::{Config, DEFAULT_CITATION_PATTERN};
use crate::errors::QdevError;
use crate::lease::find_active_lease_with_storage;
use crate::modules::ModuleRegistry;
use crate::preflight::resolve_story_target_modules;
use crate::schema::{extract_frontmatter, EntityKind};
use crate::store::{EntityFilter, Store};
use crate::write::resolve_entity_file;

/// Options configuring the impact analysis execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImpactOptions {
    /// Optional target story ID.
    pub story: Option<String>,
    /// Optional explicit workspace paths to inspect.
    pub paths: Vec<String>,
}

/// An active story discovered touching the affected modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactStory {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_modules: Vec<String>,
}

/// A reverse dependent story reached via `depends_on` or `extends`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactDependent {
    pub id: String,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relation: Option<String>,
}

/// Full outcome of an impact analysis evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactOutcome {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_story: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_paths: Vec<String>,
    pub modules: Vec<String>,
    pub stories: Vec<ImpactStory>,
    pub dependents: Vec<ImpactDependent>,
    pub requirements: Vec<String>,
    pub hazards: Vec<String>,
    pub adrs: Vec<String>,
    pub gates: Vec<String>,
    pub cited_entities: Vec<String>,
}

/// JSON payload structure for `qdev impact --json` conforming to `payload-impact.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_story: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_paths: Vec<String>,
    pub modules: Vec<String>,
    pub stories: Vec<ImpactStory>,
    pub dependents: Vec<ImpactDependent>,
    pub requirements: Vec<String>,
    pub hazards: Vec<String>,
    pub adrs: Vec<String>,
    pub gates: Vec<String>,
    pub cited_entities: Vec<String>,
}

impl From<ImpactOutcome> for ImpactPayload {
    fn from(outcome: ImpactOutcome) -> Self {
        Self {
            target_story: outcome.target_story,
            target_paths: outcome.target_paths,
            modules: outcome.modules,
            stories: outcome.stories,
            dependents: outcome.dependents,
            requirements: outcome.requirements,
            hazards: outcome.hazards,
            adrs: outcome.adrs,
            gates: outcome.gates,
            cited_entities: outcome.cited_entities,
        }
    }
}

/// Executes change impact analysis.
pub fn run_impact(
    workspace_root: &Path,
    config: &Config,
    options: &ImpactOptions,
    opt_store: Option<&dyn Store>,
) -> Result<ImpactOutcome, QdevError> {
    // 1. Resolve target story or fallback to active lease
    let normalized_story = options
        .story
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let target_story = if let Some(story_id) = normalized_story {
        Some(story_id)
    } else if options.paths.is_empty() {
        match find_active_lease_with_storage(workspace_root, Some(&config.storage)) {
            Ok(lease) => Some(lease.story_id),
            Err(_) => {
                return Err(QdevError::usage_error(
                    "No target story or paths specified, and no active story lease found in this workspace. Specify a story ID ('qdev impact E12S4') or paths ('qdev impact --paths <PATHS...>').",
                ));
            }
        }
    } else {
        None
    };

    // 2. Validate target story exists if specified
    if let Some(ref story_id) = target_story {
        let exists_in_store = if let Some(store) = opt_store {
            match store.get_live_entity_for_derivation(story_id) {
                Ok(Some(entity)) => entity.kind == EntityKind::Story,
                _ => false,
            }
        } else {
            false
        };

        if !exists_in_store {
            let exists_on_disk = resolve_entity_file(
                workspace_root,
                Some(EntityKind::Story),
                story_id,
                Some(&config.storage),
            )
            .is_ok();

            if !exists_on_disk {
                return Err(QdevError::logical_failure(
                    "entity_not_found",
                    format!("Story '{}' not found", story_id),
                ));
            }
        }
    }

    // 3. Resolve affected modules via ModuleRegistry
    let module_registry = ModuleRegistry::from_config(config);
    let mut resolved_modules_set = BTreeSet::new();

    if let Some(ref story_id) = target_story {
        let story_modules = resolve_story_target_modules(workspace_root, config, opt_store, story_id)?;
        for m in story_modules {
            resolved_modules_set.insert(m);
        }
    }

    for p in &options.paths {
        let matched = module_registry.resolve_path(p);
        for m in matched {
            resolved_modules_set.insert(m);
        }
    }

    let modules: Vec<String> = resolved_modules_set.into_iter().collect();

    // 4. Discover active stories in in-progress or review whose target_modules overlap
    let mut active_stories = Vec::new();
    if !modules.is_empty() {
        if let Some(store) = opt_store {
            let filter = EntityFilter {
                kind: Some(EntityKind::Story),
                ..Default::default()
            };
            #[allow(clippy::disallowed_methods)] // Impact analysis reads entities to report active stories in modules.
            if let Ok(entities) = store.list_entities(&filter) {
                for entity in entities {
                    if !entity.exists_for_derivation() {
                        continue;
                    }
                    if target_story.as_deref() == Some(&entity.id) {
                        continue;
                    }
                    let status = entity.status.as_deref().unwrap_or("");
                    let is_active = status == "in-progress" || status == "in_progress" || status == "review";
                    if !is_active {
                        continue;
                    }

                    let mut story_modules: Vec<String> = if let Some(ref tm_json) = entity.target_modules {
                        serde_json::from_str(tm_json).unwrap_or_default()
                    } else if let Ok(Some(details)) = store.get_story_details(&entity.id) {
                        details
                            .target_modules
                            .as_ref()
                            .and_then(|tm| serde_json::from_str(tm).ok())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    };

                    if story_modules.is_empty() {
                        if let Ok(mods) = resolve_story_target_modules(workspace_root, config, Some(store), &entity.id) {
                            story_modules = mods;
                        }
                    }

                    if story_modules.iter().any(|m| modules.contains(m)) {
                        active_stories.push(ImpactStory {
                            id: entity.id,
                            title: entity.title,
                            status: status.to_string(),
                            target_modules: story_modules,
                        });
                    }
                }
            }
        } else {
            // Fallback to scanning filesystem when no store is present
            let stories_dir = workspace_root.join(&config.storage.specs_dir).join("stories");
            if stories_dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&stories_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("md") {
                            if let Ok(content) = fs::read_to_string(&path) {
                                if let Ok(fm) = extract_frontmatter(&content) {
                                    let id = fm.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    let title = fm.get("title").and_then(|v| v.as_str()).map(|s| s.to_string());
                                    let status = fm.get("status").and_then(|v| v.as_str()).unwrap_or("");
                                    let is_active = status == "in-progress" || status == "in_progress" || status == "review";
                                    if is_active && !id.is_empty() && target_story.as_deref() != Some(id) {
                                        let mut story_mods = Vec::new();
                                        if let Some(arr) = fm.get("target_modules").and_then(|v| v.as_array()) {
                                            for item in arr {
                                                if let Some(s) = item.as_str() {
                                                    story_mods.push(s.to_string());
                                                }
                                            }
                                        }
                                        if story_mods.iter().any(|m| modules.contains(m)) {
                                            active_stories.push(ImpactStory {
                                                id: id.to_string(),
                                                title,
                                                status: status.to_string(),
                                                target_modules: story_mods,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    active_stories.sort_by(|a, b| a.id.cmp(&b.id));

    // 5. Compute transitive dependents with depth via reverse `depends_on` and `extends`
    let mut dependents = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    let seed_stories: Vec<String> = if let Some(ref target_id) = target_story {
        vec![target_id.clone()]
    } else {
        active_stories.iter().map(|s| s.id.clone()).collect()
    };

    for seed in &seed_stories {
        visited.insert(seed.clone());
        queue.push_back((seed.clone(), 0usize));
    }

    while let Some((curr_id, curr_depth)) = queue.pop_front() {
        let next_depth = curr_depth + 1;
        let relations = if let Some(store) = opt_store {
            store.get_relations_for_target(&curr_id).unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut matching: Vec<_> = relations
            .into_iter()
            .filter(|r| r.relation == "depends_on" || r.relation == "extends")
            .collect();
        matching.sort_by(|a, b| a.source_id.cmp(&b.source_id));

        for rel in matching {
            if visited.contains(&rel.source_id) {
                continue;
            }

            let (title, status) = if let Some(store) = opt_store {
                match store.get_live_entity_for_derivation(&rel.source_id) {
                    Ok(Some(ent)) if ent.kind == EntityKind::Story && ent.exists_for_derivation() => {
                        (ent.title, ent.status)
                    }
                    _ => continue,
                }
            } else {
                (None, None)
            };

            visited.insert(rel.source_id.clone());
            dependents.push(ImpactDependent {
                id: rel.source_id.clone(),
                depth: next_depth,
                title,
                status,
                relation: Some(rel.relation.clone()),
            });
            queue.push_back((rel.source_id, next_depth));
        }
    }
    dependents.sort_by(|a, b| (a.depth, &a.id).cmp(&(b.depth, &b.id)));

    // 6. Trace linked requirements, hazards, ADRs, and story declared gates
    let stories_to_trace: Vec<String> = if let Some(ref target_id) = target_story {
        vec![target_id.clone()]
    } else {
        active_stories.iter().map(|s| s.id.clone()).collect()
    };

    let mut requirements_set = BTreeSet::new();
    let mut hazards_set = BTreeSet::new();
    let mut adrs_set = BTreeSet::new();
    let mut story_declared_gates = BTreeSet::new();

    for sid in &stories_to_trace {
        // Trace relations from store
        if let Some(store) = opt_store {
            if let Ok(relations) = store.get_relations_for_source(sid) {
                for r in relations {
                    match r.relation.as_str() {
                        "traces_to" => {
                            requirements_set.insert(r.target_id);
                        }
                        "mitigates" => {
                            hazards_set.insert(r.target_id);
                        }
                        "governed_by" => {
                            adrs_set.insert(r.target_id);
                        }
                        _ => {}
                    }
                }
            }
        }

        // Trace from story file frontmatter (for declared gates and frontmatter relations fallback)
        if let Ok((_, _, story_file)) = resolve_entity_file(
            workspace_root,
            Some(EntityKind::Story),
            sid,
            Some(&config.storage),
        ) {
            if let Ok(content) = fs::read_to_string(&story_file) {
                if let Ok(fm) = extract_frontmatter(&content) {
                    if let Some(gates_arr) = fm.get("gates").and_then(|v| v.as_array()) {
                        for g in gates_arr {
                            if let Some(gate_id) = g.as_str() {
                                story_declared_gates.insert(gate_id.to_string());
                            }
                        }
                    }

                    if let Some(rel_obj) = fm.get("relations").and_then(|v| v.as_object()) {
                        if let Some(traces_arr) = rel_obj.get("traces_to").and_then(|v| v.as_array()) {
                            for t in traces_arr {
                                if let Some(req_id) = t.as_str() {
                                    requirements_set.insert(req_id.to_string());
                                }
                            }
                        }
                        if let Some(mit_arr) = rel_obj.get("mitigates").and_then(|v| v.as_array()) {
                            for h in mit_arr {
                                if let Some(haz_id) = h.as_str() {
                                    hazards_set.insert(haz_id.to_string());
                                }
                            }
                        }
                        if let Some(gov_arr) = rel_obj.get("governed_by").and_then(|v| v.as_array()) {
                            for a in gov_arr {
                                if let Some(adr_id) = a.as_str() {
                                    adrs_set.insert(adr_id.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Story declared gates also reach their verified requirements
    for gate_cfg in &config.gates {
        if story_declared_gates.contains(&gate_cfg.id) {
            for v in &gate_cfg.verifies {
                requirements_set.insert(v.clone());
            }
        }
    }

    // 7. Identify overlapping gates:
    // Any requirement in gate.verifies is present in reached requirements,
    // OR gate ID is directly declared in story's gates array.
    let mut overlapping_gates_set = BTreeSet::new();
    for gate_cfg in &config.gates {
        let matches_declared = story_declared_gates.contains(&gate_cfg.id);
        let matches_verifies = gate_cfg.verifies.iter().any(|v| requirements_set.contains(v));
        if matches_declared || matches_verifies {
            overlapping_gates_set.insert(gate_cfg.id.clone());
        }
    }

    let requirements: Vec<String> = requirements_set.into_iter().collect();
    let hazards: Vec<String> = hazards_set.into_iter().collect();
    let adrs: Vec<String> = adrs_set.into_iter().collect();
    let gates: Vec<String> = overlapping_gates_set.into_iter().collect();

    // 8. Scan changed files for inline entity citations matching citation_pattern
    let changed_files = resolve_changed_files(
        workspace_root,
        &options.paths,
        &config.git.integration_branch,
    );

    let pattern_str = config
        .hygiene
        .citation_pattern
        .as_deref()
        .unwrap_or(DEFAULT_CITATION_PATTERN);

    let cited_entities = scan_citations_in_files(&changed_files, pattern_str);

    Ok(ImpactOutcome {
        target_story,
        target_paths: options.paths.clone(),
        modules,
        stories: active_stories,
        dependents,
        requirements,
        hazards,
        adrs,
        gates,
        cited_entities,
    })
}

/// Recursively collects all regular files within a directory, ignoring symlinks.
fn collect_files_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            if path.is_file() {
                out.push(path);
            } else if path.is_dir() {
                collect_files_recursive(&path, out);
            }
        }
    }
}

/// Resolves target changed files from explicit paths or git diff in a worktree.
fn resolve_changed_files(
    workspace_root: &Path,
    explicit_paths: &[String],
    integration_branch: &str,
) -> Vec<PathBuf> {
    if !explicit_paths.is_empty() {
        let mut files = Vec::new();
        for p in explicit_paths {
            let path = if Path::new(p).is_absolute() {
                PathBuf::from(p)
            } else {
                workspace_root.join(p)
            };
            if path.is_file() {
                files.push(path);
            } else if path.is_dir() {
                collect_files_recursive(&path, &mut files);
            }
        }
        return files;
    }

    let is_git = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace_root)
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false);

    if !is_git {
        return Vec::new();
    }

    let mut changed = HashSet::new();

    // 1. Try merge-base with integration_branch
    let mb_output = Command::new("git")
        .args(["merge-base", "HEAD", integration_branch])
        .current_dir(workspace_root)
        .output();

    let mut diff_target = None;
    if let Ok(ref out) = mb_output {
        if out.status.success() {
            let mb = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !mb.is_empty() {
                diff_target = Some(mb);
            }
        }
    }

    // Try merge-base with origin/<integration_branch> if local branch ref not found
    if diff_target.is_none() {
        let remote_branch = format!("origin/{}", integration_branch);
        let mb_remote = Command::new("git")
            .args(["merge-base", "HEAD", &remote_branch])
            .current_dir(workspace_root)
            .output();
        if let Ok(ref out) = mb_remote {
            if out.status.success() {
                let mb = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !mb.is_empty() {
                    diff_target = Some(mb);
                }
            }
        }
    }

    // Fallback: integration_branch ref directly
    if diff_target.is_none() {
        let verify = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", integration_branch])
            .current_dir(workspace_root)
            .output();
        if let Ok(ref out) = verify {
            if out.status.success() {
                diff_target = Some(integration_branch.to_string());
            }
        }
    }

    // Fallback: origin/<integration_branch> ref directly
    if diff_target.is_none() {
        let remote_branch = format!("origin/{}", integration_branch);
        let verify_remote = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet", &remote_branch])
            .current_dir(workspace_root)
            .output();
        if let Ok(ref out) = verify_remote {
            if out.status.success() {
                diff_target = Some(remote_branch);
            }
        }
    }

    if let Some(target) = diff_target {
        if let Ok(diff_out) = Command::new("git")
            .args(["-c", "core.quotePath=false", "diff", "--name-only", "--relative", &target])
            .current_dir(workspace_root)
            .output()
        {
            if diff_out.status.success() {
                for line in String::from_utf8_lossy(&diff_out.stdout).lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        changed.insert(trimmed.to_string());
                    }
                }
            }
        }
    }

    // Include uncommitted changes (staged, unstaged, untracked)
    if let Ok(status_out) = Command::new("git")
        .args(["-c", "core.quotePath=false", "status", "--porcelain=v1", "-uall"])
        .current_dir(workspace_root)
        .output()
    {
        if status_out.status.success() {
            for line in String::from_utf8_lossy(&status_out.stdout).lines() {
                if line.len() >= 3 {
                    let path = line[3..].trim();
                    let path = if let Some(idx) = path.find(" -> ") {
                        &path[idx + 4..]
                    } else {
                        path
                    };
                    let cleaned = path.trim_matches('"');
                    if !cleaned.is_empty() {
                        changed.insert(cleaned.to_string());
                    }
                }
            }
        }
    }

    changed
        .into_iter()
        .map(|p| workspace_root.join(p))
        .filter(|p| p.is_file())
        .collect()
}

/// Scans target files for inline entity citations matching the given regex pattern.
fn scan_citations_in_files(files: &[PathBuf], pattern: &str) -> Vec<String> {
    let re = match regex::Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    let mut cited = BTreeSet::new();

    for file_path in files {
        if fs::metadata(file_path).map(|m| m.len()).unwrap_or(0) > 10 * 1024 * 1024 {
            continue;
        }

        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => continue, // Gracefully skip unreadable or binary files
        };

        for caps in re.captures_iter(&content) {
            let id = if let Some(m) = caps.get(1) {
                m.as_str().trim()
            } else if let Some(m) = caps.get(0) {
                m.as_str().trim().trim_start_matches('[').trim_end_matches(']')
            } else {
                continue;
            };
            if !id.is_empty() {
                cited.insert(id.to_string());
            }
        }
    }

    cited.into_iter().collect()
}

/// Formats the human-readable text report for impact analysis.
pub fn format_impact_text(outcome: &ImpactOutcome) -> String {
    let mut out = String::new();

    // Header
    if let Some(ref story_id) = outcome.target_story {
        if !outcome.target_paths.is_empty() {
            out.push_str(&format!(
                "Impact Analysis for Story: {} and Paths: {}\n",
                story_id,
                outcome.target_paths.join(", ")
            ));
        } else {
            out.push_str(&format!("Impact Analysis for Story: {}\n", story_id));
        }
    } else if !outcome.target_paths.is_empty() {
        out.push_str(&format!(
            "Impact Analysis for Paths: {}\n",
            outcome.target_paths.join(", ")
        ));
    } else {
        out.push_str("Impact Analysis\n");
    }

    // Modules
    if outcome.modules.is_empty() {
        out.push_str("  Modules: (none)\n");
    } else {
        out.push_str(&format!("  Modules: {}\n", outcome.modules.join(", ")));
    }

    // Active stories in modules
    if outcome.stories.is_empty() {
        out.push_str("  Active Stories in Modules: (none)\n");
    } else {
        out.push_str(&format!(
            "  Active Stories in Modules ({}):\n",
            outcome.stories.len()
        ));
        for s in &outcome.stories {
            let title = s.title.as_deref().unwrap_or("(untitled)");
            out.push_str(&format!("    - {} [{}]: {}\n", s.id, s.status, title));
        }
    }

    // Transitive Dependents
    if outcome.dependents.is_empty() {
        out.push_str("  Transitive Dependents: (none)\n");
    } else {
        out.push_str(&format!(
            "  Transitive Dependents ({}):\n",
            outcome.dependents.len()
        ));
        for d in &outcome.dependents {
            let rel = d.relation.as_deref().unwrap_or("depends_on");
            let title_part = if let Some(ref t) = d.title {
                format!(": {}", t)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "    - {} (depth {}, {}){}\n",
                d.id, d.depth, rel, title_part
            ));
        }
    }

    // Requirements
    if outcome.requirements.is_empty() {
        out.push_str("  Linked Requirements: (none)\n");
    } else {
        out.push_str(&format!(
            "  Linked Requirements: {}\n",
            outcome.requirements.join(", ")
        ));
    }

    // Hazards
    if outcome.hazards.is_empty() {
        out.push_str("  Linked Hazards: (none)\n");
    } else {
        out.push_str(&format!("  Linked Hazards: {}\n", outcome.hazards.join(", ")));
    }

    // ADRs
    if outcome.adrs.is_empty() {
        out.push_str("  Linked ADRs: (none)\n");
    } else {
        out.push_str(&format!("  Linked ADRs: {}\n", outcome.adrs.join(", ")));
    }

    // Overlapping Gates
    if outcome.gates.is_empty() {
        out.push_str("  Overlapping Gates: (none)\n");
    } else {
        out.push_str(&format!("  Overlapping Gates: {}\n", outcome.gates.join(", ")));
    }

    // Cited Entities
    if outcome.cited_entities.is_empty() {
        out.push_str("  Cited Entities in Changed Files: (none)\n");
    } else {
        out.push_str(&format!(
            "  Cited Entities in Changed Files: {}\n",
            outcome.cited_entities.join(", ")
        ));
    }

    out.trim_end().to_string()
}
