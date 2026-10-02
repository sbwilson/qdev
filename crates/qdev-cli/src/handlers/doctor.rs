//! Diagnostic inspection handler: evaluates cache, git, leases, skills, MCP, hooks, and gates.

use serde::Serialize;

use crate::cli::{self, Cli};
use crate::open_query_store;
use crate::output::OutputEmitter;
use crate::prompt_input;
use qdev_core::{ExitCode, Interactivity, JsonEnvelope, QdevError};

#[derive(Serialize)]
pub struct DoctorPayload {
    pub sections: Vec<qdev_core::DoctorSectionReport>,
}

pub fn handle_doctor(
    doctor_args: &cli::DoctorArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    if doctor_args.fix {
        // 1. Rewrite git hook shims if in a git repository
        if qdev_core::resolve_hooks_dir(&root).is_ok() {
            if let Err(e) = qdev_core::install_hooks(&root) {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        }

        // 2. Regenerate outdated skills
        if let Ok(status) = qdev_core::inspect_skills(&root) {
            if status.outdated_count > 0 {
                let mut claude = false;
                let mut cursor = false;
                let mut agents = false;
                for p in &status.outdated_skills {
                    if p.starts_with(".claude/") {
                        claude = true;
                    } else if p.starts_with(".cursor/") {
                        cursor = true;
                    } else if p.starts_with(".agents/") {
                        agents = true;
                    }
                }
                if claude || cursor || agents {
                    let options = qdev_core::SkillInstallOptions {
                        claude,
                        cursor,
                        agents,
                    };
                    if let Err(e) = qdev_core::install_skills_configured(
                        &root,
                        &options,
                        Some(&annotated_config.config.models),
                        Some(&annotated_config.config.synthesis),
                    ) {
                        let _ = output.emit_error(&e);
                        return e.exit_code();
                    }
                }
            }
        }

        // 3. Rebuild corrupt cache if needed (missing cache is initialized without confirmation)
        let cache_db_path = root
            .join(&annotated_config.config.storage.cache_dir)
            .join("cache.sqlite");
        if !cache_db_path.exists() {
            let storage = &annotated_config.config.storage;
            let lock_path = root.join(&storage.cache_dir).join("write.lock");
            let _lock = match qdev_core::acquire_write_lock(
                &lock_path,
                std::time::Duration::from_millis(5000),
            ) {
                Ok(guard) => guard,
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
            };

            let store = match qdev_core::SqliteStore::open(&cache_db_path) {
                Ok(s) => s,
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
            };
            if let Err(e) = store.reset_and_rebuild(&root, storage) {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        } else {
            let is_corrupt = match qdev_core::inspect_cache_schema(&cache_db_path) {
                Ok(qdev_core::CacheSchemaStatus::Valid) => false,
                Ok(qdev_core::CacheSchemaStatus::Mismatch) => true,
                Ok(qdev_core::CacheSchemaStatus::NewerThanSupported { .. }) => false,
                Err(_) => true,
            };

            if is_corrupt {
                if !interactivity.is_interactive() && !doctor_args.yes {
                    let err = QdevError::policy_refusal(
                        "needs_confirmation",
                        "Rebuilding corrupt cache requires interactive confirmation or '--yes'",
                    )
                    .with_details(serde_json::json!({ "flag": "--yes" }))
                    .with_attribution(
                        qdev_core::RejectionAttribution::new(
                            "Rebuilding corrupt cache requires confirmation in non-interactive mode",
                        )
                        .with_policy("cache_rebuild_confirmation"),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::PolicyRefusal;
                }

                let should_rebuild = if doctor_args.yes {
                    true
                } else {
                    let prompt = "Cache is corrupt or mismatched. Rebuild cache from markdown files? [y/N]: ";
                    match prompt_input(prompt) {
                        Ok(ans) => ans.eq_ignore_ascii_case("y") || ans.eq_ignore_ascii_case("yes"),
                        Err(_) => false,
                    }
                };

                if should_rebuild {
                    let storage = &annotated_config.config.storage;
                    let lock_path = root.join(&storage.cache_dir).join("write.lock");
                    let _lock = match qdev_core::acquire_write_lock(
                        &lock_path,
                        std::time::Duration::from_millis(5000),
                    ) {
                        Ok(guard) => guard,
                        Err(e) => {
                            let _ = output.emit_error(&e);
                            return e.exit_code();
                        }
                    };

                    let cache_dir = root.join(&storage.cache_dir);
                    let _ = std::fs::remove_file(&cache_db_path);
                    let _ = std::fs::remove_file(cache_dir.join("cache.sqlite-wal"));
                    let _ = std::fs::remove_file(cache_dir.join("cache.sqlite-shm"));

                    let store = match qdev_core::SqliteStore::open(&cache_db_path) {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = output.emit_error(&e);
                            return e.exit_code();
                        }
                    };
                    if let Err(e) = store.reset_and_rebuild(&root, storage) {
                        let _ = output.emit_error(&e);
                        return e.exit_code();
                    }
                }
            }
        }
    }

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut sections = Vec::new();
    // `default_doctor_sections` needs the workspace root and config for the sections that
    // inspect the workspace itself rather than just the cache (the `validation` section runs
    // `qdev validate`'s own checks); `handle_doctor` already holds both.
    for section in qdev_core::default_doctor_sections(&root, &annotated_config.config) {
        match section.run(&store) {
            Ok(report) => sections.push(report),
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(DoctorPayload { sections });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit doctor envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_doctor_text(&sections, Some(&annotated_config.config.storage));
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit doctor output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

fn field_value<'a>(
    section: &'a qdev_core::DoctorSectionReport,
    key: &str,
) -> Option<&'a serde_json::Value> {
    section
        .fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
}

fn field_str<'a>(section: &'a qdev_core::DoctorSectionReport, key: &str) -> Option<&'a str> {
    field_value(section, key).and_then(|v| v.as_str())
}

fn field_u64(section: &qdev_core::DoctorSectionReport, key: &str) -> Option<u64> {
    field_value(section, key).and_then(|v| v.as_u64())
}

/// Renders doctor section reports as a summary block matching CLI reference §7 followed by
/// detailed section reports.
pub fn render_doctor_text(
    sections: &[qdev_core::DoctorSectionReport],
    storage: Option<&qdev_core::config::StorageConfig>,
) -> String {
    let mut out = String::new();

    // 1. Git summary
    let git_sec = sections.iter().find(|s| s.name == "git");
    let git_line = if let Some(git) = git_sec {
        let status = field_str(git, "status").unwrap_or("unavailable");
        match status {
            "ok" => {
                let branch = field_str(git, "branch").unwrap_or("unknown");
                let remote = field_str(git, "remote").unwrap_or("origin");
                let int_branch = field_str(git, "integration_branch").unwrap_or("develop");
                let int_state = field_str(git, "integration_state").unwrap_or("");
                if branch == int_branch && int_state == "up_to_date" {
                    format!(
                        "[✓] Git: {} tracks {}/{}; clean tree",
                        branch, remote, int_branch
                    )
                } else {
                    format!("[✓] Git: {}; clean tree", branch)
                }
            }
            "mismatch" => {
                let branch = field_str(git, "branch").unwrap_or("unknown");
                let remote = field_str(git, "remote").unwrap_or("origin");
                let int_branch = field_str(git, "integration_branch").unwrap_or("develop");
                let int_state = field_str(git, "integration_state").unwrap_or("");
                let clean = field_value(git, "clean")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let dirty_files = field_u64(git, "dirty_files").unwrap_or(0);
                let is_tracks = branch == int_branch;
                if clean {
                    if int_state == "diverged" {
                        format!(
                            "[!] Git: {}; clean tree, diverged from {}/{}",
                            branch, remote, int_branch
                        )
                    } else if int_state == "refs_missing"
                        || int_state == "integration_branch_missing"
                        || int_state == "remote_ref_missing"
                    {
                        format!(
                            "[!] Git: {}; clean tree, integration ref missing ({})",
                            branch, int_state
                        )
                    } else {
                        format!("[!] Git: {}; clean tree", branch)
                    }
                } else {
                    let tree_desc = if dirty_files > 0 {
                        format!("dirty tree ({} files)", dirty_files)
                    } else {
                        "dirty tree".to_string()
                    };
                    if is_tracks && int_state == "up_to_date" {
                        format!(
                            "[!] Git: {} tracks {}/{}; {}",
                            branch, remote, int_branch, tree_desc
                        )
                    } else if int_state == "diverged" {
                        format!(
                            "[!] Git: {}; {}, diverged from {}/{}",
                            branch, tree_desc, remote, int_branch
                        )
                    } else {
                        format!("[!] Git: {}; {}", branch, tree_desc)
                    }
                }
            }
            _ => {
                let reason = field_str(git, "unavailable_reason").unwrap_or("unavailable");
                if reason == "not_a_git_repository" {
                    "[x] Git: not a git repository".to_string()
                } else {
                    format!("[x] Git: {}", reason)
                }
            }
        }
    } else {
        "[x] Git: section missing".to_string()
    };
    out.push_str(&git_line);
    out.push('\n');

    // 2. Cache summary
    let cache_sec = sections.iter().find(|s| s.name == "cache");
    let val_sec = sections.iter().find(|s| s.name == "validation");
    let cache_line = if let Some(cache) = cache_sec {
        let status = field_str(cache, "status").unwrap_or("mismatch");
        let version = field_u64(cache, "cache_schema_version").unwrap_or(3);
        let entity_count = field_u64(cache, "entity_count").unwrap_or(0);
        let val_findings = val_sec
            .and_then(|v| field_u64(v, "finding_count"))
            .unwrap_or(0);
        let cache_path = storage
            .map(|s| format!("{}/cache.sqlite", s.cache_dir.trim_end_matches('/')))
            .unwrap_or_else(|| ".qdev/cache/cache.sqlite".to_string());
        if status == "ok" {
            let glyph = if val_findings > 0 { "[!]" } else { "[✓]" };
            format!(
                "{} Cache: {} schema v{}, {} entities, {} validation findings",
                glyph, cache_path, version, entity_count, val_findings
            )
        } else {
            let missing_tables = field_value(cache, "missing_tables")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            let expected = field_u64(cache, "expected_cache_schema_version").unwrap_or(3);
            if !missing_tables.is_empty() {
                format!(
                    "[!] Cache: {} missing tables ({})",
                    cache_path,
                    missing_tables.join(", ")
                )
            } else if status == "unavailable" {
                format!("[x] Cache: {} unreadable", cache_path)
            } else if version != expected {
                format!(
                    "[!] Cache: {} schema mismatch (expected v{}, found v{})",
                    cache_path, expected, version
                )
            } else {
                format!("[!] Cache: {} corrupted or invalid", cache_path)
            }
        }
    } else {
        "[x] Cache: section missing".to_string()
    };
    out.push_str(&cache_line);
    out.push('\n');

    // 3. Modules summary
    let modules_sec = sections.iter().find(|s| s.name == "modules");
    let modules_line = if let Some(modules) = modules_sec {
        let status = field_str(modules, "status").unwrap_or("ok");
        let declared = field_u64(modules, "declared_count").unwrap_or(0);
        let unmatched = field_u64(modules, "unmatched_count").unwrap_or(0);
        if status == "ok" {
            format!(
                "[✓] Modules: {} declared, all path globs match at least one file",
                declared
            )
        } else if status == "mismatch" {
            format!(
                "[!] Modules: {} declared, {} path glob(s) match zero files",
                declared, unmatched
            )
        } else {
            "[x] Modules: unavailable".to_string()
        }
    } else {
        "[x] Modules: section missing".to_string()
    };
    out.push_str(&modules_line);
    out.push('\n');

    // 4. Gates summary
    let gates_sec = sections.iter().find(|s| s.name == "gates");
    let gates_line = if let Some(gates) = gates_sec {
        let status = field_str(gates, "status").unwrap_or("ok");
        let configured = field_u64(gates, "configured_count").unwrap_or(0);
        let missing = field_u64(gates, "missing_count").unwrap_or(0);
        let skipped = field_u64(gates, "skipped_locally_count").unwrap_or(0);
        let skip_clause = if skipped > 0 {
            format!(", {} skipped locally", skipped)
        } else {
            String::new()
        };
        if status == "ok" {
            format!(
                "[✓] Gates: {} configured, all executables found{}",
                configured, skip_clause
            )
        } else if status == "mismatch" {
            format!(
                "[!] Gates: {} configured, {} missing executable(s){}",
                configured, missing, skip_clause
            )
        } else {
            "[x] Gates: unavailable".to_string()
        }
    } else {
        "[x] Gates: section missing".to_string()
    };
    out.push_str(&gates_line);
    out.push('\n');

    // 5. Hooks summary
    let hooks_sec = sections.iter().find(|s| s.name == "hooks");
    let hooks_line = if let Some(hooks) = hooks_sec {
        let status = field_str(hooks, "status").unwrap_or("ok");
        if status == "ok" {
            "[✓] Hooks: 3 shims installed and current".to_string()
        } else if status == "mismatch" {
            let missing = field_value(hooks, "missing_hooks")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            let outdated = field_value(hooks, "outdated_hooks")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            let mut details = Vec::new();
            if !missing.is_empty() {
                details.push(format!("missing: {}", missing.join(", ")));
            }
            if !outdated.is_empty() {
                details.push(format!("outdated: {}", outdated.join(", ")));
            }
            if details.is_empty() {
                "[!] Hooks: shims missing or outdated".to_string()
            } else {
                format!("[!] Hooks: {}", details.join("; "))
            }
        } else {
            let reason = field_str(hooks, "unavailable_reason").unwrap_or("unavailable");
            if reason == "not_a_git_repository" {
                "[x] Hooks: not a git repository".to_string()
            } else {
                format!("[x] Hooks: {}", reason)
            }
        }
    } else {
        "[x] Hooks: section missing".to_string()
    };
    out.push_str(&hooks_line);
    out.push('\n');

    // 6. Skills summary
    let skills_sec = sections.iter().find(|s| s.name == "skills");
    let skills_line = if let Some(skills) = skills_sec {
        let status = field_str(skills, "status").unwrap_or("ok");
        let installed = field_u64(skills, "installed_count").unwrap_or(0);
        let outdated = field_u64(skills, "outdated_count").unwrap_or(0);
        let binary_ver = field_str(skills, "binary_version").unwrap_or(env!("CARGO_PKG_VERSION"));
        if status == "ok" {
            format!(
                "[✓] Skills: {} installed, up to date with binary {}",
                installed, binary_ver
            )
        } else if status == "mismatch" {
            format!(
                "[!] Skills: {} installed, {} outdated with binary {}",
                installed, outdated, binary_ver
            )
        } else {
            "[x] Skills: unavailable".to_string()
        }
    } else {
        "[x] Skills: section missing".to_string()
    };
    out.push_str(&skills_line);
    out.push('\n');

    // 7. MCP summary
    let mcp_sec = sections.iter().find(|s| s.name == "mcp");
    let mcp_line = if let Some(mcp) = mcp_sec {
        let status = field_str(mcp, "status").unwrap_or("ok");
        if status == "ok" {
            let targets = field_value(mcp, "registered_targets")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            if targets.is_empty() {
                "[✓] MCP: registered".to_string()
            } else {
                let formatted = targets
                    .iter()
                    .map(|t| format!(".{}", t))
                    .collect::<Vec<_>>()
                    .join(" and ");
                format!("[✓] MCP: registered in {} settings", formatted)
            }
        } else if status == "unregistered" {
            "[!] MCP: unregistered in editor settings".to_string()
        } else {
            let reason = field_str(mcp, "unavailable_reason").unwrap_or("unavailable");
            format!("[x] MCP: {}", reason)
        }
    } else {
        "[x] MCP: section missing".to_string()
    };
    out.push_str(&mcp_line);
    out.push('\n');

    // 8. Leases summary
    let leases_sec = sections.iter().find(|s| s.name == "leases");
    let leases_line = if let Some(leases) = leases_sec {
        let status = field_str(leases, "status").unwrap_or("ok");
        let active = field_u64(leases, "active_count").unwrap_or(0);
        let stale = field_u64(leases, "stale_count").unwrap_or(0);
        if status == "ok" {
            if stale == 0 {
                format!("[✓] Leases: {} active leases, 0 stale", active)
            } else {
                let stale_list = leases
                    .fields
                    .iter()
                    .find(|(k, _)| k == "stale_leases")
                    .and_then(|(_, v)| v.as_array());
                if let Some(list) = stale_list {
                    if let Some(first) = list.first() {
                        let story_id = first
                            .get("story_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let holder = first
                            .get("holder")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let worktree = first
                            .get("worktree_path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let age = first.get("age_days").and_then(|v| v.as_u64()).unwrap_or(0);
                        format!(
                            "[!] Leases: {} held by {} in {}, {} days old",
                            story_id, holder, worktree, age
                        )
                    } else {
                        format!("[!] Leases: {} stale lease(s)", stale)
                    }
                } else {
                    format!("[!] Leases: {} stale lease(s)", stale)
                }
            }
        } else {
            "[x] Leases: unavailable".to_string()
        }
    } else {
        "[x] Leases: section missing".to_string()
    };
    out.push_str(&leases_line);
    out.push('\n');
    out.push('\n');

    // Section detailed blocks
    for section in sections {
        out.push_str(&format!("[{}]\n", section.name));
        for (key, value) in &section.fields {
            // An object is a breakdown (e.g. `findings_by_code`), and this is the output a
            // human reads when something is already wrong — so it gets one indented line per
            // entry rather than a JSON blob on the value line.
            if let serde_json::Value::Object(map) = value {
                if map.is_empty() {
                    out.push_str(&format!("  {} = none\n", key));
                } else {
                    out.push_str(&format!("  {}:\n", key));
                    for (entry_key, entry_value) in map {
                        let rendered = match entry_value {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        out.push_str(&format!("    {} = {}\n", entry_key, rendered));
                    }
                }
                continue;
            }
            if let serde_json::Value::Array(items) = value {
                if items.is_empty() {
                    out.push_str(&format!("  {} = none\n", key));
                } else {
                    let rendered_items: Vec<String> = items
                        .iter()
                        .map(|item| match item {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    out.push_str(&format!("  {} = {}\n", key, rendered_items.join(", ")));
                }
                continue;
            }
            let rendered = match value {
                serde_json::Value::Null => "null".to_string(),
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.push_str(&format!("  {} = {}\n", key, rendered));
        }
    }
    out
}
