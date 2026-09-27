//! The built-in `qdev-scope` gate: verifies every changed file is inside the
//! story's `target_modules` or is covered by a matching no-go constraint.

use std::path::Path;
use std::time::Instant;

use crate::config::{Config, GateConfig};
use crate::errors::QdevError;
use crate::gate::git::{
    diff_name_only_relative, is_inside_work_tree, merge_base_commit, name_only_paths, ref_exists,
    status_porcelain_untracked_all,
};
use crate::gate::runner::{
    record_gate_evidence, resolve_commit_sha, resolve_story_id, GateRunOptions,
};
use crate::gate::{GateFailure, GateRunOutcome, GateStatus};
use crate::modules::ModuleRegistry;
use crate::schema::EntityKind;
use crate::write::resolve_entity_file;

use super::BUILTIN_GATE_SCOPE;

fn matches_word_boundary(text: &str, kw: &str) -> bool {
    let kw_len = kw.len();
    for (start_idx, _) in text.match_indices(kw) {
        let before_ok = if start_idx == 0 {
            true
        } else {
            let prev_char = text[..start_idx].chars().next_back().unwrap();
            !prev_char.is_alphanumeric() && prev_char != '_'
        };
        let end_idx = start_idx + kw_len;
        let after_ok = if end_idx >= text.len() {
            true
        } else {
            let next_char = text[end_idx..].chars().next().unwrap();
            !next_char.is_alphanumeric() && next_char != '_'
        };
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

fn find_matching_no_go<'a>(
    path: &str,
    registry: &ModuleRegistry,
    no_gos: &'a [(String, String)],
) -> Option<(&'a String, &'a String)> {
    let path_obj = Path::new(path);
    let file_name = path_obj.file_name().and_then(|f| f.to_str()).unwrap_or("");
    let file_stem = path_obj.file_stem().and_then(|f| f.to_str()).unwrap_or("");
    let matching_modules = registry.resolve_path(path);

    let mut keywords = Vec::new();
    for m in &matching_modules {
        keywords.push(m.to_lowercase());
    }
    if !file_name.is_empty() {
        keywords.push(file_name.to_lowercase());
    }
    if !file_stem.is_empty() && file_stem != file_name {
        keywords.push(file_stem.to_lowercase());
    }
    keywords.push(path.to_lowercase());
    for comp in path_obj.components() {
        let c_str = comp.as_os_str().to_string_lossy().to_lowercase();
        if c_str != "crates"
            && c_str != "src"
            && c_str != "tests"
            && c_str != "lib"
            && c_str != "pkg"
            && c_str != "packages"
        {
            keywords.push(c_str);
        }
    }

    for (cid, text) in no_gos {
        let lower_text = text.to_lowercase();
        for kw in &keywords {
            if kw.len() >= 3 && matches_word_boundary(&lower_text, kw) {
                return Some((cid, text));
            }
        }
    }
    None
}

/// Returns the changed-file set, or the reason the diff baseline could not be established.
///
/// Fails closed on purpose: when the baseline is unresolvable, returning an empty set would
/// let the scope gate pass while committed out-of-scope work stays invisible to the check.
pub(crate) fn get_git_diff_files(
    workspace_root: &Path,
    integration_branch: &str,
) -> Result<std::collections::HashSet<String>, String> {
    let is_git = is_inside_work_tree(workspace_root);

    if !is_git {
        return Err(format!(
            "workspace is not a git work tree; the diff baseline against '{integration_branch}' is unavailable"
        ));
    }

    let mut changed = std::collections::HashSet::new();

    // 1. Try merge-base with integration_branch
    let mut diff_target = merge_base_commit(workspace_root, integration_branch);

    // Fallback: check if integration_branch ref exists directly
    if diff_target.is_none() && ref_exists(workspace_root, integration_branch) {
        diff_target = Some(integration_branch.to_string());
    }

    let Some(target) = diff_target else {
        return Err(format!(
            "integration branch '{integration_branch}' is unresolvable: no merge-base with HEAD and the ref is not found locally"
        ));
    };

    match diff_name_only_relative(workspace_root, &target) {
        Ok(diff_out) if diff_out.status.success() => {
            let diff_stdout = String::from_utf8_lossy(&diff_out.stdout);
            changed.extend(name_only_paths(&diff_stdout));
        }
        Ok(diff_out) => {
            return Err(format!(
                "git diff against '{target}' failed: {}",
                String::from_utf8_lossy(&diff_out.stderr).trim()
            ))
        }
        Err(err) => {
            return Err(format!(
                "git diff against '{target}' could not be executed: {err}"
            ));
        }
    }

    // Include uncommitted changes (staged and unstaged + untracked)
    match status_porcelain_untracked_all(workspace_root) {
        Ok(status_out) if status_out.status.success() => {
            for line in String::from_utf8_lossy(&status_out.stdout).lines() {
                if line.len() >= 3 {
                    let path = line[3..].trim();
                    let path = if let Some(idx) = path.find(" -> ") {
                        &path[idx + 4..]
                    } else {
                        path
                    };
                    if !path.is_empty() {
                        changed.insert(path.to_string());
                    }
                }
            }
        }
        Ok(status_out) => {
            return Err(format!(
                "git status failed: {}",
                String::from_utf8_lossy(&status_out.stderr).trim()
            ))
        }
        Err(err) => return Err(format!("git status could not be executed: {err}")),
    }

    Ok(changed)
}

/// Executes the built-in `qdev-scope` verification gate.
pub fn execute_scope_gate(
    workspace_root: &Path,
    config: &Config,
    options: &GateRunOptions,
) -> Result<GateRunOutcome, QdevError> {
    let start_time = Instant::now();
    let commit_sha = resolve_commit_sha(workspace_root);
    let story_id = resolve_story_id(workspace_root, config, options);

    // Check if qdev-scope is configured with skip = true
    let configured_gate = config.gates.iter().find(|g| g.id == BUILTIN_GATE_SCOPE);
    if configured_gate.and_then(|g| g.skip) == Some(true) {
        let gate_cfg = configured_gate.cloned().unwrap_or_else(|| GateConfig {
            id: BUILTIN_GATE_SCOPE.to_string(),
            command: None,
            timeout_ms: None,
            depends_on: Vec::new(),
            output_adapter: None,
            on_transition: vec!["review".to_string()],
            verifies: Vec::new(),
            kind: Some("builtin".to_string()),
            metric: None,
            direction: None,
            skip: Some(true),
        });
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            &gate_cfg,
            story_id.as_deref(),
            commit_sha.as_deref(),
            "pass",
            0,
            0,
            None,
            "skipped_locally",
            None,
            None,
            true,
        )?;
        return Ok(GateRunOutcome {
            gate_id: BUILTIN_GATE_SCOPE.to_string(),
            status: GateStatus::Skip,
            exit_code: 0,
            duration_ms: 0,
            summary: "skipped_locally".to_string(),
            skipped_locally: true,
            commit_sha,
            story_id,
            stdout: None,
            stderr: None,
            agent_instruction: None,
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
            evidence_path: Some(evidence_path),
        });
    }

    let mut target_modules = Vec::new();
    let mut own_no_gos: Vec<(String, String)> = Vec::new();
    let mut inherited_no_gos: Vec<(String, String)> = Vec::new();

    if let Some(ref sid) = story_id {
        if let Ok((_, _, story_file)) = resolve_entity_file(
            workspace_root,
            Some(EntityKind::Story),
            sid,
            Some(&config.storage),
        ) {
            if let Ok(content) = std::fs::read_to_string(&story_file) {
                if let Ok(frontmatter) = crate::schema::extract_frontmatter(&content) {
                    if let Some(arr) = frontmatter.get("target_modules").and_then(|v| v.as_array())
                    {
                        for item in arr {
                            if let Some(s) = item.as_str() {
                                let trimmed = s.trim();
                                if !trimmed.is_empty()
                                    && !target_modules.contains(&trimmed.to_string())
                                {
                                    target_modules.push(trimmed.to_string());
                                }
                            }
                        }
                    }

                    let parse_no_gos =
                        |val: &serde_json::Value, prefix: &str, out: &mut Vec<(String, String)>| {
                            if let Some(arr) = val.as_array() {
                                for c in arr {
                                    if c.get("kind").and_then(|v| v.as_str()) == Some("no_go") {
                                        let cid =
                                            c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                        let text = c
                                            .get("text")
                                            .and_then(|v| v.as_str())
                                            .or_else(|| c.get("statement").and_then(|v| v.as_str()))
                                            .unwrap_or("");
                                        let full_id = if cid.contains('/') {
                                            cid.to_string()
                                        } else {
                                            format!("{}/{}", prefix, cid)
                                        };
                                        out.push((full_id, text.to_string()));
                                    }
                                }
                            } else if let Some(obj) = val.as_object() {
                                if let Some(arr) = obj.get("no_go").and_then(|v| v.as_array()) {
                                    for c in arr {
                                        let cid =
                                            c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                        let text = c
                                            .get("text")
                                            .and_then(|v| v.as_str())
                                            .or_else(|| c.get("statement").and_then(|v| v.as_str()))
                                            .unwrap_or("");
                                        let full_id = if cid.contains('/') {
                                            cid.to_string()
                                        } else {
                                            format!("{}/{}", prefix, cid)
                                        };
                                        out.push((full_id, text.to_string()));
                                    }
                                }
                            }
                        };

                    if let Some(c_val) = frontmatter.get("constraints") {
                        parse_no_gos(c_val, sid, &mut own_no_gos);
                    }

                    if let Some(epic_id) = frontmatter
                        .get("epic_id")
                        .and_then(|v| v.as_str())
                        .filter(|e| !e.is_empty())
                    {
                        if let Ok((_, _, epic_file)) = resolve_entity_file(
                            workspace_root,
                            Some(EntityKind::Epic),
                            epic_id,
                            Some(&config.storage),
                        ) {
                            if let Ok(epic_content) = std::fs::read_to_string(&epic_file) {
                                if let Ok(epic_fm) =
                                    crate::schema::extract_frontmatter(&epic_content)
                                {
                                    if let Some(c_val) = epic_fm.get("constraints") {
                                        parse_no_gos(c_val, epic_id, &mut inherited_no_gos);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let diff_files = match get_git_diff_files(workspace_root, &config.git.integration_branch) {
        Ok(files) => files,
        Err(reason) => {
            // Fail closed: an unresolvable baseline cannot be treated as "nothing changed",
            // or committed out-of-scope work would sail through the gate.
            let scope_gate_config = GateConfig {
                id: BUILTIN_GATE_SCOPE.to_string(),
                command: None,
                timeout_ms: None,
                depends_on: Vec::new(),
                output_adapter: None,
                on_transition: vec!["review".to_string()],
                verifies: Vec::new(),
                kind: Some("builtin".to_string()),
                metric: None,
                direction: None,
                skip: None,
            };
            let duration_ms = start_time.elapsed().as_millis() as u64;
            let summary = format!("scope baseline unresolvable: {reason}");
            let evidence_path = record_gate_evidence(
                workspace_root,
                config,
                &scope_gate_config,
                story_id.as_deref(),
                commit_sha.as_deref(),
                "infra",
                4,
                duration_ms,
                None,
                &summary,
                None,
                None,
                false,
            )?;
            return Ok(GateRunOutcome {
                gate_id: BUILTIN_GATE_SCOPE.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms,
                summary: summary.clone(),
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: vec![GateFailure {
                    location: "git".to_string(),
                    message: summary,
                }],
                metric: None,
                constraint_ids: Vec::new(),
                evidence_path: Some(evidence_path),
            });
        }
    };
    let registry = ModuleRegistry::from_config(config);
    let mut failures = Vec::new();
    let mut constraint_ids = Vec::new();

    let specs_prefix = format!("{}/", config.storage.specs_dir.trim_end_matches('/'));
    let state_prefix = format!("{}/", config.storage.state_dir.trim_end_matches('/'));
    let cache_prefix = format!("{}/", config.storage.cache_dir.trim_end_matches('/'));

    let mut sorted_files: Vec<String> = diff_files.into_iter().collect();
    sorted_files.sort();

    for path_str in sorted_files {
        let norm = path_str.replace('\\', "/");
        let norm = norm.strip_prefix("./").unwrap_or(&norm);
        let norm = norm.strip_prefix('/').unwrap_or(norm);

        if norm.starts_with(".qdev/")
            || norm.starts_with(".git/")
            || norm == ".gitignore"
            || norm == "qdev.toml"
            || norm == ".qdev.local.toml"
            || norm.starts_with("docs/")
            || norm.starts_with(&specs_prefix)
            || norm.starts_with(&state_prefix)
            || norm.starts_with(&cache_prefix)
        {
            continue;
        }

        let mut in_scope = false;
        for tm in &target_modules {
            if let Some(m_cfg) = registry.get(tm) {
                if m_cfg
                    .paths
                    .iter()
                    .any(|pattern| crate::validate::glob_match(pattern, norm))
                {
                    in_scope = true;
                    break;
                }
            }
        }

        if !in_scope {
            let matching_no_go = find_matching_no_go(norm, &registry, &own_no_gos)
                .or_else(|| find_matching_no_go(norm, &registry, &inherited_no_gos));

            if let Some((cid, text)) = matching_no_go {
                if !constraint_ids.contains(cid) {
                    constraint_ids.push(cid.clone());
                }
                failures.push(GateFailure {
                    location: norm.to_string(),
                    message: format!("Path outside target_modules; violates {}: {}", cid, text),
                });
            } else {
                failures.push(GateFailure {
                    location: norm.to_string(),
                    message: "Path outside target_modules; policy: target_modules".to_string(),
                });
            }
        }
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let (status, exit_code, summary, agent_instruction) = if failures.is_empty() {
        (
            GateStatus::Pass,
            0,
            "scope check passed: all changed files within target_modules".to_string(),
            Some("continue".to_string()),
        )
    } else {
        (
            GateStatus::Fail,
            1,
            format!(
                "scope check failed: {} path(s) outside target_modules",
                failures.len()
            ),
            Some("fix_cited_failures".to_string()),
        )
    };

    let scope_gate_config = GateConfig {
        id: BUILTIN_GATE_SCOPE.to_string(),
        command: None,
        timeout_ms: None,
        depends_on: Vec::new(),
        output_adapter: None,
        on_transition: vec!["review".to_string()],
        verifies: Vec::new(),
        kind: Some("builtin".to_string()),
        metric: None,
        direction: None,
        skip: None,
    };
    let ev_status = match status {
        GateStatus::Pass => "pass",
        GateStatus::Fail => "fail",
        GateStatus::Infra => "infra",
        GateStatus::Skip => "pass",
    };
    let output_text = if failures.is_empty() {
        summary.clone()
    } else {
        let failure_lines: Vec<String> = failures
            .iter()
            .map(|f| format!("{}: {}", f.location, f.message))
            .collect();
        format!("{}\n{}", summary, failure_lines.join("\n"))
    };
    let evidence_path = record_gate_evidence(
        workspace_root,
        config,
        &scope_gate_config,
        story_id.as_deref(),
        commit_sha.as_deref(),
        ev_status,
        exit_code,
        duration_ms,
        None,
        &summary,
        Some(&output_text),
        None,
        false,
    )?;

    Ok(GateRunOutcome {
        gate_id: BUILTIN_GATE_SCOPE.to_string(),
        status,
        exit_code,
        duration_ms,
        summary,
        skipped_locally: false,
        commit_sha,
        story_id,
        stdout: None,
        stderr: None,
        agent_instruction,
        failures,
        metric: None,
        constraint_ids,
        evidence_path: Some(evidence_path),
    })
}
