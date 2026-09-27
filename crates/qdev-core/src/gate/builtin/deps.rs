//! The built-in `qdev-deps` gate: scans Rust and Swift sources for undeclared
//! module imports and layer inversions.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{Config, GateConfig, ModuleConfig};
use crate::errors::QdevError;
use crate::gate::runner::{
    record_gate_evidence, resolve_commit_sha, resolve_story_id, GateRunOptions,
};
use crate::gate::{GateFailure, GateRunOutcome, GateStatus};
use crate::modules::ModuleRegistry;

use super::BUILTIN_GATE_DEPS;

/// Scans source content for Rust `use` statements, returning (line_number, root_imported_crate).
pub fn scan_rust_imports(content: &str) -> Vec<(usize, String)> {
    let mut results = Vec::new();
    let mut in_block_comment = false;

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let mut line = raw_line.trim().to_string();

        // Handle multi-line block comment continuation
        if in_block_comment {
            if let Some(pos) = line.find("*/") {
                line = line[pos + 2..].trim().to_string();
                in_block_comment = false;
            } else {
                continue;
            }
        }

        // If line comment // appears before /*, strip it first so // ... /* does not trigger block comment
        if !in_block_comment {
            if let Some(slash_slash) = line.find("//") {
                if let Some(slash_star) = line.find("/*") {
                    if slash_slash < slash_star {
                        line = line[..slash_slash].trim().to_string();
                    }
                } else {
                    line = line[..slash_slash].trim().to_string();
                }
            }
        }

        // Strip single-line block comments within the line
        while let Some(start_pos) = line.find("/*") {
            if let Some(end_pos) = line[start_pos + 2..].find("*/") {
                let actual_end = start_pos + 2 + end_pos + 2;
                line = format!("{}{}", &line[..start_pos], &line[actual_end..]);
            } else {
                line = line[..start_pos].trim().to_string();
                in_block_comment = true;
                break;
            }
        }

        // Strip trailing line comment
        if let Some(pos) = line.find("//") {
            line = line[..pos].trim().to_string();
        }

        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        // Strip visibility modifiers: pub, pub(crate), pub(...)
        let mut rest = trimmed_line;
        if rest.starts_with("pub") {
            rest = rest[3..].trim_start();
            if rest.starts_with('(') {
                if let Some(close_paren) = rest.find(')') {
                    rest = rest[close_paren + 1..].trim_start();
                }
            }
        }

        if let Some(stripped) = rest.strip_prefix("use ") {
            let after_use = stripped.trim_start().trim_start_matches(':');
            let token: String = after_use
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !token.is_empty()
                && token != "crate"
                && token != "super"
                && token != "self"
                && token != "std"
                && token != "core"
                && token != "alloc"
            {
                results.push((line_no, token));
            }
        }
    }
    results
}

/// Scans source content for Swift `import` statements, returning (line_number, root_imported_module).
pub fn scan_swift_imports(content: &str) -> Vec<(usize, String)> {
    let mut results = Vec::new();
    let mut in_block_comment = false;

    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = idx + 1;
        let mut line = raw_line.trim().to_string();

        if in_block_comment {
            if let Some(pos) = line.find("*/") {
                line = line[pos + 2..].trim().to_string();
                in_block_comment = false;
            } else {
                continue;
            }
        }

        // If line comment // appears before /*, strip it first so // ... /* does not trigger block comment
        if !in_block_comment {
            if let Some(slash_slash) = line.find("//") {
                if let Some(slash_star) = line.find("/*") {
                    if slash_slash < slash_star {
                        line = line[..slash_slash].trim().to_string();
                    }
                } else {
                    line = line[..slash_slash].trim().to_string();
                }
            }
        }

        while let Some(start_pos) = line.find("/*") {
            if let Some(end_pos) = line[start_pos + 2..].find("*/") {
                let actual_end = start_pos + 2 + end_pos + 2;
                line = format!("{}{}", &line[..start_pos], &line[actual_end..]);
            } else {
                line = line[..start_pos].trim().to_string();
                in_block_comment = true;
                break;
            }
        }

        if let Some(pos) = line.find("//") {
            line = line[..pos].trim().to_string();
        }

        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        let mut rest = trimmed_line;
        // Strip attributes (e.g. @_exported, @testable, @preconcurrency) and access modifiers
        loop {
            if rest.starts_with('@') {
                if let Some(space_pos) = rest.find(char::is_whitespace) {
                    rest = rest[space_pos..].trim_start();
                    continue;
                } else {
                    break;
                }
            }
            if let Some(stripped) = rest
                .strip_prefix("public ")
                .or_else(|| rest.strip_prefix("internal "))
                .or_else(|| rest.strip_prefix("fileprivate "))
                .or_else(|| rest.strip_prefix("private "))
                .or_else(|| rest.strip_prefix("open "))
            {
                rest = stripped.trim_start();
                continue;
            }
            break;
        }

        if let Some(stripped) = rest.strip_prefix("import ") {
            let after_import = stripped.trim_start();
            let mut parts = after_import.split_whitespace();
            if let Some(first) = parts.next() {
                let module_cand = match first {
                    "class" | "struct" | "enum" | "protocol" | "typealias" | "func" | "let"
                    | "var" => parts.next(),
                    other => Some(other),
                };
                if let Some(cand) = module_cand {
                    let root_mod: String = cand
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !root_mod.is_empty() {
                        results.push((line_no, root_mod));
                    }
                }
            }
        }
    }
    results
}

fn find_source_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if p.is_dir() && !p.is_symlink() {
            if name_str.starts_with('.')
                || name_str == "target"
                || name_str == "build"
                || name_str == ".build"
                || name_str == "node_modules"
                || name_str == "docs"
            {
                continue;
            }
            find_source_files(&p, files);
        } else if p.is_file() {
            if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
                if ext == "rs" || ext == "swift" {
                    files.push(p);
                }
            }
        }
    }
}

/// Executes the built-in `qdev-deps` verification gate.
pub fn execute_deps_gate(
    workspace_root: &Path,
    config: &Config,
    options: &GateRunOptions,
) -> Result<GateRunOutcome, QdevError> {
    let start_time = Instant::now();
    let commit_sha = resolve_commit_sha(workspace_root);
    let story_id = resolve_story_id(workspace_root, config, options);

    let configured_gate = config.gates.iter().find(|g| g.id == BUILTIN_GATE_DEPS);
    if configured_gate.and_then(|g| g.skip) == Some(true) {
        let gate_cfg = configured_gate.cloned().unwrap_or_else(|| GateConfig {
            id: BUILTIN_GATE_DEPS.to_string(),
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
            gate_id: BUILTIN_GATE_DEPS.to_string(),
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

    let registry = ModuleRegistry::from_config(config);
    let mut module_map: std::collections::HashMap<String, &ModuleConfig> =
        std::collections::HashMap::new();
    for m in &config.modules {
        module_map.insert(m.id.clone(), m);
    }

    let mut source_files = Vec::new();
    find_source_files(workspace_root, &mut source_files);
    source_files.sort();

    let mut failures = Vec::new();

    for file_path in source_files {
        let Ok(rel_path) = file_path.strip_prefix(workspace_root) else {
            continue;
        };
        let norm_rel = rel_path.to_string_lossy().replace('\\', "/");
        let matched_modules = registry.resolve_path(&norm_rel);
        if matched_modules.is_empty() {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&file_path) else {
            continue;
        };

        let is_rust = file_path.extension().and_then(|s| s.to_str()) == Some("rs");
        let imports = if is_rust {
            scan_rust_imports(&content)
        } else {
            scan_swift_imports(&content)
        };

        for (line_no, imported_name) in imports {
            let target_mod = config.modules.iter().find(|m| {
                m.id == imported_name
                    || m.id.replace('-', "_") == imported_name
                    || m.id.eq_ignore_ascii_case(&imported_name)
            });

            if let Some(target_m) = target_mod {
                for src_mod_id in &matched_modules {
                    if src_mod_id == &target_m.id {
                        continue;
                    }
                    let src_m = module_map[src_mod_id];

                    // Check 1: may_depend_on (with hyphen/underscore normalization)
                    let is_declared_dep = src_m.may_depend_on.iter().any(|dep| {
                        dep == &target_m.id
                            || dep.replace('-', "_") == target_m.id.replace('-', "_")
                            || dep.eq_ignore_ascii_case(&target_m.id)
                    });
                    if !is_declared_dep {
                        let failure = GateFailure {
                            location: format!("{}:{}", norm_rel, line_no),
                            message: format!(
                                "module '{}' imports undeclared dependency '{}' (not in may_depend_on)",
                                src_m.id, target_m.id
                            ),
                        };
                        if !failures.contains(&failure) {
                            failures.push(failure);
                        }
                    }

                    // Check 2: layer hierarchy (target_layer > source_layer)
                    if let (Some(la), Some(lb)) = (src_m.layer, target_m.layer) {
                        if lb > la {
                            let failure = GateFailure {
                                location: format!("{}:{}", norm_rel, line_no),
                                message: format!(
                                    "module '{}' (layer {}) imports higher layer module '{}' (layer {}) [layer inversion]",
                                    src_m.id, la, target_m.id, lb
                                ),
                            };
                            if !failures.contains(&failure) {
                                failures.push(failure);
                            }
                        }
                    }
                }
            }
        }
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;
    let (status, exit_code, summary, agent_instruction) = if failures.is_empty() {
        (
            GateStatus::Pass,
            0,
            "dependency check passed: zero undeclared imports or layer inversions".to_string(),
            Some("continue".to_string()),
        )
    } else {
        (
            GateStatus::Fail,
            1,
            format!(
                "dependency check failed with {} violation(s)",
                failures.len()
            ),
            Some("fix_cited_failures".to_string()),
        )
    };

    let deps_gate_config = GateConfig {
        id: BUILTIN_GATE_DEPS.to_string(),
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
        &deps_gate_config,
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
        gate_id: BUILTIN_GATE_DEPS.to_string(),
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
        constraint_ids: Vec::new(),
        evidence_path: Some(evidence_path),
    })
}
