use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::Digest;

use crate::config::{Config, GateConfig, ModuleConfig};
use crate::errors::QdevError;
use crate::gate::adapter::{parse_with_adapter, validate_adapter_name};
use crate::gate::process::ProcessGroupIsolation;
use crate::gate::result::GateResultDocument;
use crate::gate::ring_buffer::HeadTailBuffer;
use crate::gate::{
    EvidenceBundle, GateFailure, GateListItem, GateRunOutcome, GateRunSetOutcome, GateStatus,
};
use crate::lease::find_active_lease_with_storage;
use crate::modules::ModuleRegistry;
use crate::schema::EntityKind;
use crate::store::Store;
use crate::write::resolve_entity_file;

pub const BUILTIN_GATE_SCOPE: &str = "qdev-scope";
pub const BUILTIN_GATE_DEPS: &str = "qdev-deps";

/// Execution options for running a gate.
#[derive(Debug, Clone, Default)]
pub struct GateRunOptions {
    pub story: Option<String>,
    pub timeout_ms: Option<u64>,
}

/// RAII guard ensuring temporary result files are reliably deleted upon return.
struct TempFileGuard(PathBuf);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Extracts up to `max_lines` trailing non-empty lines from `text`.
pub fn extract_last_lines(text: &str, max_lines: usize) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let take_count = lines.len().min(max_lines);
    let start_idx = lines.len() - take_count;
    Some(lines[start_idx..].join("\n"))
}

/// Parses a command line string into separate arguments, respecting single/double quotes and escapes.
/// Preserves empty string arguments e.g. `""` or `''`.
pub fn parse_command_args(cmd: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut escape = false;
    let mut has_token = false;

    for ch in cmd.chars() {
        if escape {
            current.push(ch);
            has_token = true;
            escape = false;
        } else if ch == '\\' && !in_single_quote {
            escape = true;
        } else if ch == '\'' && !in_double_quote {
            in_single_quote = !in_single_quote;
            has_token = true;
        } else if ch == '"' && !in_single_quote {
            in_double_quote = !in_double_quote;
            has_token = true;
        } else if ch.is_whitespace() && !in_single_quote && !in_double_quote {
            if has_token {
                args.push(current);
                current = String::new();
                has_token = false;
            }
        } else {
            current.push(ch);
            has_token = true;
        }
    }
    if has_token {
        args.push(current);
    }
    args
}

/// Probes whether `candidate` is a file, checking Windows `PATHEXT` extensions when appropriate.
fn check_file_with_extensions(candidate: &Path) -> Option<PathBuf> {
    if candidate.is_file() {
        return Some(candidate.to_path_buf());
    }

    #[cfg(windows)]
    let is_windows = true;
    #[cfg(not(windows))]
    let is_windows = false;

    if is_windows && candidate.extension().is_none() {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        for ext in pathext.split(';') {
            let ext = ext.trim();
            if !ext.is_empty() {
                let with_ext = candidate.with_extension(ext.trim_start_matches('.'));
                if with_ext.is_file() {
                    return Some(with_ext);
                }
            }
        }
    }
    None
}

/// Resolves an executable binary to a path and checks whether it exists on disk or in PATH.
pub fn resolve_binary(workspace_root: &Path, binary_str: &str) -> (String, bool) {
    if binary_str.contains('/') || binary_str.contains('\\') {
        let p = if Path::new(binary_str).is_absolute() {
            PathBuf::from(binary_str)
        } else {
            workspace_root.join(binary_str)
        };
        if let Some(found) = check_file_with_extensions(&p) {
            (found.display().to_string(), true)
        } else {
            (p.display().to_string(), false)
        }
    } else {
        if let Some(path_os) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_os) {
                let candidate = dir.join(binary_str);
                if let Some(found) = check_file_with_extensions(&candidate) {
                    return (found.display().to_string(), true);
                }
            }
        }
        let root_candidate = workspace_root.join(binary_str);
        if let Some(found) = check_file_with_extensions(&root_candidate) {
            return (found.display().to_string(), true);
        }
        (binary_str.to_string(), false)
    }
}

/// Resolves the current git commit SHA using `git rev-parse HEAD`.
pub fn resolve_commit_sha(workspace_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .current_dir(workspace_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !sha.is_empty() {
            return Some(sha);
        }
    }
    None
}

/// Resolves the active or specified story ID.
fn resolve_story_id(
    workspace_root: &Path,
    config: &Config,
    options: &GateRunOptions,
) -> Option<String> {
    if let Some(ref s) = options.story {
        if !s.is_empty() {
            return Some(s.clone());
        }
    }
    find_active_lease_with_storage(workspace_root, Some(&config.storage))
        .ok()
        .map(|l| l.story_id)
}

fn extract_metric_from_text(text: &str, metric_name: &str) -> Option<f64> {
    let lower_metric = metric_name.to_lowercase();
    let patterns = [
        format!("{}:", lower_metric),
        format!("{} =", lower_metric),
        format!("{}=", lower_metric),
    ];
    for line in text.lines().rev() {
        let trimmed = line.trim();
        let lower_line = trimmed.to_lowercase();
        for pat in &patterns {
            if let Some(idx) = lower_line.find(pat) {
                let remainder = trimmed[idx + pat.len()..].trim();
                let token = remainder
                    .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                    .find(|s| !s.is_empty());
                if let Some(t) = token {
                    if let Ok(val) = t.parse::<f64>() {
                        if val.is_finite() {
                            return Some(val);
                        }
                    }
                }
            }
        }
    }

    if let Ok(val) = text.trim().parse::<f64>() {
        if val.is_finite() {
            return Some(val);
        }
    }

    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| l.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
}

/// Helper function to write an immutable evidence bundle and hydrate into SQLite cache.
#[allow(clippy::too_many_arguments)]
fn record_gate_evidence(
    workspace_root: &Path,
    config: &Config,
    gate_config: &GateConfig,
    story_id: Option<&str>,
    commit_sha: Option<&str>,
    status: &str,
    exit_code: i32,
    duration_ms: u64,
    metric: Option<f64>,
    summary: &str,
    stdout: Option<&str>,
    stderr: Option<&str>,
    skipped_locally: bool,
) -> Result<String, QdevError> {
    let short_sha = commit_sha
        .map(|s| {
            let char_count = s.chars().count();
            if char_count >= 7 {
                s.chars().take(7).collect::<String>()
            } else if !s.is_empty() {
                s.to_string()
            } else {
                "unknown".to_string()
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let output_sha256 = {
        let mut hasher = sha2::Sha256::new();
        if let Some(out) = stdout {
            if !out.is_empty() {
                sha2::Digest::update(&mut hasher, out.as_bytes());
            }
        }
        if let Some(err) = stderr {
            if !err.is_empty() {
                sha2::Digest::update(&mut hasher, err.as_bytes());
            }
        }
        format!("{:x}", sha2::Digest::finalize(hasher))
    };

    let annotated_config =
        crate::config::AnnotatedConfig::new(config.clone(), std::collections::BTreeMap::new());
    let run_by = crate::write::resolve_author(None, None, &annotated_config, workspace_root)
        .unwrap_or_else(|_| crate::write::Author::new("human", "developer"));

    let bundle = EvidenceBundle {
        schema_version: "1".to_string(),
        gate: gate_config.id.clone(),
        story: story_id.map(str::to_string),
        commit: short_sha,
        status: status.to_string(),
        exit_code,
        duration_ms,
        metric,
        summary: summary.to_string(),
        output_sha256,
        run_by,
        ran_at: crate::write::current_iso8601(),
        verifies: gate_config.verifies.clone(),
        skipped_locally,
    };

    let (evidence_path, rel_evidence_path, stem) =
        crate::gate::write_evidence_bundle(workspace_root, &config.storage, &bundle)?;

    // Hydrate into SQLite cache if database exists and is valid
    let cache_db_path = workspace_root
        .join(&config.storage.cache_dir)
        .join("cache.sqlite");
    if cache_db_path.is_file() {
        if let Ok(crate::store::CacheSchemaStatus::Valid) =
            crate::store::inspect_cache_schema(&cache_db_path)
        {
            if let Ok(store) = crate::store::SqliteStore::open(&cache_db_path) {
                let target_dir = bundle.story.as_deref().unwrap_or("_workspace");
                let run_id = format!("{}:{}", target_dir, stem);
                let gate_run_record = crate::store::GateRunRecord {
                    id: run_id,
                    story_id: bundle.story.clone(),
                    gate_id: bundle.gate.clone(),
                    commit_sha: bundle.commit.clone(),
                    status: Some(bundle.status.clone()),
                    exit_code: Some(bundle.exit_code),
                    duration_ms: Some(bundle.duration_ms),
                    metric_value: bundle.metric,
                    summary: Some(bundle.summary.clone()),
                    evidence_path: rel_evidence_path.clone(),
                    output_hash: Some(bundle.output_sha256.clone()),
                    run_by_type: Some(bundle.run_by.author_type.clone()),
                    run_by_id: Some(bundle.run_by.id.clone()),
                    ran_at: Some(bundle.ran_at.clone()),
                };
                let _ = store.upsert_gate_run(&gate_run_record);
                let (mtime, size) = crate::store::sqlite::file_change_stamp(&evidence_path);
                let content_hash = crate::write::sha256_digest(
                    std::fs::read(&evidence_path).unwrap_or_default().as_slice(),
                );
                let _ = store.upsert_sync_state(
                    &rel_evidence_path,
                    mtime,
                    size,
                    Some(&content_hash),
                );
            }
        }
    }

    Ok(rel_evidence_path)
}

/// Executes a verification gate external subprocess according to Story 3.2 specification.
pub fn execute_gate(
    workspace_root: &Path,
    config: &Config,
    gate_id: &str,
    options: &GateRunOptions,
) -> Result<GateRunOutcome, QdevError> {
    if gate_id == BUILTIN_GATE_SCOPE {
        return execute_scope_gate(workspace_root, config, options);
    }
    if gate_id == BUILTIN_GATE_DEPS {
        return execute_deps_gate(workspace_root, config, options);
    }

    let gate_config = config
        .gates
        .iter()
        .find(|g| g.id == gate_id)
        .ok_or_else(|| {
            QdevError::usage_error(format!("gate '{}' not found in configuration", gate_id))
        })?;

    // Validate declared output adapter upfront
    if let Some(ref adapter) = gate_config.output_adapter {
        validate_adapter_name(adapter)?;
    }

    let commit_sha = resolve_commit_sha(workspace_root);
    let story_id = resolve_story_id(workspace_root, config, options);

    // 1. Check local gate skip configuration
    if gate_config.skip == Some(true) {
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            gate_config,
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
            gate_id: gate_id.to_string(),
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

    // 2. Resolve command
    let cmd_str = match &gate_config.command {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => {
            let summary = "missing executable: <none>".to_string();
            let evidence_path = record_gate_evidence(
                workspace_root,
                config,
                gate_config,
                story_id.as_deref(),
                commit_sha.as_deref(),
                "infra",
                4,
                0,
                None,
                &summary,
                None,
                None,
                false,
            )?;
            return Ok(GateRunOutcome {
                gate_id: gate_id.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms: 0,
                summary,
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
                evidence_path: Some(evidence_path),
            });
        }
    };

    let args = parse_command_args(cmd_str);
    if args.is_empty() {
        let summary = "missing executable: <empty>".to_string();
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            gate_config,
            story_id.as_deref(),
            commit_sha.as_deref(),
            "infra",
            4,
            0,
            None,
            &summary,
            None,
            None,
            false,
        )?;
        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms: 0,
            summary,
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: None,
            stderr: None,
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
            evidence_path: Some(evidence_path),
        });
    }

    let binary_str = &args[0];
    let binary_args = &args[1..];

    // 3. Check if executable exists
    let (resolved_path, binary_exists) = resolve_binary(workspace_root, binary_str);
    if !binary_exists {
        let summary = format!("missing executable: {}", resolved_path);
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            gate_config,
            story_id.as_deref(),
            commit_sha.as_deref(),
            "infra",
            4,
            0,
            None,
            &summary,
            None,
            None,
            false,
        )?;
        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms: 0,
            summary,
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: None,
            stderr: None,
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
            evidence_path: Some(evidence_path),
        });
    }

    // 4. Assemble environment variables
    let temp_dir = std::env::temp_dir();
    let result_file_path = temp_dir.join(format!(
        "qdev-gate-result-{}-{}-{}.json",
        gate_id,
        std::process::id(),
        rand::random::<u64>()
    ));
    let _result_guard = TempFileGuard(result_file_path.clone());

    let module_registry = ModuleRegistry::from_config(config);
    let module_paths_map = module_registry.to_module_paths_map();
    let module_paths_json =
        serde_json::to_string(&module_paths_map).unwrap_or_else(|_| "{}".to_string());

    let mut cmd = Command::new(&resolved_path);
    cmd.args(binary_args);
    cmd.current_dir(workspace_root);

    // Apply [environment] overrides
    for (k, v) in &config.environment.variables {
        cmd.env(k, v);
    }

    // Inject QDEV_* execution environment variables
    cmd.env("QDEV_STORY", story_id.as_deref().unwrap_or(""));
    cmd.env("QDEV_GATE", gate_id);
    cmd.env("QDEV_COMMIT", commit_sha.as_deref().unwrap_or(""));
    cmd.env("QDEV_RESULT_FILE", &result_file_path);
    cmd.env("QDEV_MODULE_PATHS", &module_paths_json);

    // Process group isolation
    ProcessGroupIsolation::configure_command(&mut cmd);

    // Stream capture via pipes and null stdin
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    // 5. Spawn subprocess
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let summary = format!("missing executable: {}", resolved_path);
            let evidence_path = record_gate_evidence(
                workspace_root,
                config,
                gate_config,
                story_id.as_deref(),
                commit_sha.as_deref(),
                "infra",
                4,
                0,
                None,
                &summary,
                None,
                None,
                false,
            )?;
            return Ok(GateRunOutcome {
                gate_id: gate_id.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms: 0,
                summary,
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
                evidence_path: Some(evidence_path),
            });
        }
        Err(e) => {
            let summary = format!("failed to spawn executable {}: {}", resolved_path, e);
            let evidence_path = record_gate_evidence(
                workspace_root,
                config,
                gate_config,
                story_id.as_deref(),
                commit_sha.as_deref(),
                "infra",
                4,
                0,
                None,
                &summary,
                None,
                None,
                false,
            )?;
            return Ok(GateRunOutcome {
                gate_id: gate_id.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms: 0,
                summary,
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
                evidence_path: Some(evidence_path),
            });
        }
    };

    let isolation = ProcessGroupIsolation::on_spawned(&child);

    // 6. Concurrently capture stdout and stderr into bounded HeadTailBuffers
    let mut child_stdout = child.stdout.take().expect("child stdout must be piped");
    let mut child_stderr = child.stderr.take().expect("child stderr must be piped");

    let stdout_handle = std::thread::spawn(move || {
        let mut buffer = HeadTailBuffer::new();
        let mut chunk = [0u8; 8192];
        loop {
            match child_stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buffer.write_bytes(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        buffer
    });

    let stderr_handle = std::thread::spawn(move || {
        let mut buffer = HeadTailBuffer::new();
        let mut chunk = [0u8; 8192];
        loop {
            match child_stderr.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buffer.write_bytes(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        buffer
    });

    // 7. Enforce timeout and monitor child process
    let timeout_ms = options
        .timeout_ms
        .or(gate_config.timeout_ms)
        .unwrap_or(300_000);
    let timeout_duration = Duration::from_millis(timeout_ms);
    let start_time = Instant::now();

    let mut timed_out = false;
    let mut child_status = None;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                child_status = Some(status);
                break;
            }
            Ok(None) => {
                if start_time.elapsed() >= timeout_duration {
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                break;
            }
        }
    }

    if timed_out {
        isolation.kill_tree(&mut child);
        let _ = child.wait();
        let stdout_buf = stdout_handle.join().unwrap_or_default();
        let stderr_buf = stderr_handle.join().unwrap_or_default();
        let duration_ms = start_time.elapsed().as_millis() as u64;
        let summary = format!("timeout after {}ms", timeout_ms);
        let stdout_str = stdout_buf.to_string_lossy();
        let stderr_str = stderr_buf.to_string_lossy();
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            gate_config,
            story_id.as_deref(),
            commit_sha.as_deref(),
            "infra",
            4,
            duration_ms,
            None,
            &summary,
            Some(&stdout_str),
            Some(&stderr_str),
            false,
        )?;

        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms,
            summary,
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: Some(stdout_str),
            stderr: Some(stderr_str),
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
            evidence_path: Some(evidence_path),
        });
    }

    let status = match child_status {
        Some(s) => s,
        None => child.wait().map_err(|e| {
            QdevError::infrastructure_failure(
                "wait_failed",
                format!("Failed to wait on child process: {}", e),
            )
        })?,
    };

    // Terminate any remaining descendant processes in the tree
    isolation.kill_tree(&mut child);

    let stdout_buf = stdout_handle.join().unwrap_or_default();
    let stderr_buf = stderr_handle.join().unwrap_or_default();
    let duration_ms = start_time.elapsed().as_millis() as u64;

    let stdout_str = stdout_buf.to_string_lossy();
    let stderr_str = stderr_buf.to_string_lossy();

    // Check if process was killed by a signal
    #[cfg(unix)]
    let killed_by_signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let killed_by_signal: Option<i32> = None;

    if let Some(sig) = killed_by_signal {
        let summary = format!("process terminated by signal {}", sig);
        let evidence_path = record_gate_evidence(
            workspace_root,
            config,
            gate_config,
            story_id.as_deref(),
            commit_sha.as_deref(),
            "infra",
            4,
            duration_ms,
            None,
            &summary,
            Some(&stdout_str),
            Some(&stderr_str),
            false,
        )?;
        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms,
            summary,
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: Some(stdout_str),
            stderr: Some(stderr_str),
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
            evidence_path: Some(evidence_path),
        });
    }

    let raw_exit_code = status.code().unwrap_or(-1);

    // 8. Output classification based on output_adapter configuration
    let (
        mut status,
        mut exit_code,
        mut summary,
        mut agent_instruction,
        mut failures,
        mut metric,
        constraint_ids,
    ) = match gate_config.output_adapter.as_deref() {
            Some("json") => {
                // If output_adapter = "json" is declared, require a valid result document.
                let mut parsed_doc = None;
                let mut schema_err = None;

                if result_file_path.exists() {
                    let content = std::fs::read_to_string(&result_file_path).unwrap_or_default();
                    if !content.trim().is_empty() {
                        match GateResultDocument::from_json_str(&content) {
                            Ok(doc) => parsed_doc = Some(doc),
                            Err(errs) => schema_err = Some(errs),
                        }
                    }
                }

                if parsed_doc.is_none() && !stdout_str.trim().is_empty() {
                    match GateResultDocument::from_json_str(stdout_str.trim()) {
                        Ok(doc) => {
                            parsed_doc = Some(doc);
                            schema_err = None;
                        }
                        Err(errs) => {
                            if schema_err.is_none() {
                                schema_err = Some(errs);
                            }
                        }
                    }
                }

                if let Some(doc) = parsed_doc {
                    let (ec, instr) = match doc.status {
                        GateStatus::Pass => (0, Some("continue".to_string())),
                        GateStatus::Fail => {
                            let code = if raw_exit_code != 0 {
                                raw_exit_code
                            } else {
                                1
                            };
                            (code, Some("fix_cited_failures".to_string()))
                        }
                        GateStatus::Infra => (4, Some("halt_and_alert".to_string())),
                        GateStatus::Skip => (0, None),
                    };
                    (
                        doc.status,
                        ec,
                        doc.summary,
                        instr,
                        doc.failures,
                        doc.metric,
                        doc.constraint_ids,
                    )
                } else if let Some(errs) = schema_err {
                    (
                        GateStatus::Infra,
                        4,
                        format!(
                            "declared json adapter schema validation failed: {}",
                            errs.join("; ")
                        ),
                        Some("halt_and_alert".to_string()),
                        Vec::new(),
                        None,
                        Vec::new(),
                    )
                } else {
                    (
                        GateStatus::Infra,
                        4,
                        "declared json adapter produced no result document on $QDEV_RESULT_FILE or stdout"
                            .to_string(),
                        Some("halt_and_alert".to_string()),
                        Vec::new(),
                        None,
                        Vec::new(),
                    )
                }
            }
            Some("cargo") => {
                let parsed = parse_with_adapter(
                    "cargo",
                    &stdout_str,
                    &stderr_str,
                    raw_exit_code,
                    Some(workspace_root),
                )?;
                let ec = match parsed.status {
                    GateStatus::Pass => 0,
                    GateStatus::Fail => {
                        if raw_exit_code != 0 {
                            raw_exit_code
                        } else {
                            1
                        }
                    }
                    GateStatus::Infra => 4,
                    GateStatus::Skip => 0,
                };
                let instr = match parsed.status {
                    GateStatus::Pass => Some("continue".to_string()),
                    GateStatus::Fail => Some("fix_cited_failures".to_string()),
                    GateStatus::Infra => Some("halt_and_alert".to_string()),
                    GateStatus::Skip => None,
                };
                (
                    parsed.status,
                    ec,
                    parsed.summary,
                    instr,
                    parsed.failures,
                    None,
                    Vec::new(),
                )
            }
            Some("xcodebuild") => {
                let parsed = parse_with_adapter(
                    "xcodebuild",
                    &stdout_str,
                    &stderr_str,
                    raw_exit_code,
                    Some(workspace_root),
                )?;
                let ec = match parsed.status {
                    GateStatus::Pass => 0,
                    GateStatus::Fail => {
                        if raw_exit_code != 0 {
                            raw_exit_code
                        } else {
                            1
                        }
                    }
                    GateStatus::Infra => 4,
                    GateStatus::Skip => 0,
                };
                let instr = match parsed.status {
                    GateStatus::Pass => Some("continue".to_string()),
                    GateStatus::Fail => Some("fix_cited_failures".to_string()),
                    GateStatus::Infra => Some("halt_and_alert".to_string()),
                    GateStatus::Skip => None,
                };
                (
                    parsed.status,
                    ec,
                    parsed.summary,
                    instr,
                    parsed.failures,
                    None,
                    Vec::new(),
                )
            }
            _ => {
                // No adapter declared: check if valid result document exists on $QDEV_RESULT_FILE or stdout
                let mut maybe_doc = None;
                if result_file_path.exists() {
                    let content = std::fs::read_to_string(&result_file_path).unwrap_or_default();
                    if !content.trim().is_empty() {
                        maybe_doc = GateResultDocument::from_json_str(&content).ok();
                    }
                }
                if maybe_doc.is_none() && !stdout_str.trim().is_empty() {
                    maybe_doc = GateResultDocument::from_json_str(stdout_str.trim()).ok();
                }

                if let Some(doc) = maybe_doc {
                    let (ec, instr) = match doc.status {
                        GateStatus::Pass => (0, Some("continue".to_string())),
                        GateStatus::Fail => {
                            let code = if raw_exit_code != 0 {
                                raw_exit_code
                            } else {
                                1
                            };
                            (code, Some("fix_cited_failures".to_string()))
                        }
                        GateStatus::Infra => (4, Some("halt_and_alert".to_string())),
                        GateStatus::Skip => (0, None),
                    };
                    (
                        doc.status,
                        ec,
                        doc.summary,
                        instr,
                        doc.failures,
                        doc.metric,
                        doc.constraint_ids,
                    )
                } else if raw_exit_code == 0 {
                    let summary = if let Some(last_line) =
                        stdout_str.lines().rev().find(|l| !l.trim().is_empty())
                    {
                        last_line.trim().to_string()
                    } else {
                        "gate passed".to_string()
                    };
                    (
                        GateStatus::Pass,
                        0,
                        summary,
                        Some("continue".to_string()),
                        Vec::new(),
                        None,
                        Vec::new(),
                    )
                } else {
                    let summary = extract_last_lines(&stderr_str, 40)
                        .or_else(|| extract_last_lines(&stdout_str, 40))
                        .unwrap_or_else(|| format!("gate exited with code {}", raw_exit_code));
                    (
                        GateStatus::Fail,
                        raw_exit_code,
                        summary,
                        Some("fix_cited_failures".to_string()),
                        Vec::new(),
                        None,
                        Vec::new(),
                    )
                }
            }
        };

    // If gate is a ratchet gate, evaluate against branch baseline
    if gate_config.kind.as_deref() == Some("ratchet") {
        let direction_str = gate_config.direction.as_deref().unwrap_or("");
        if direction_str != "must_not_increase" && direction_str != "must_not_decrease" {
            return Err(QdevError::usage_error(format!(
                "ratchet gate '{}' requires direction 'must_not_increase' or 'must_not_decrease'",
                gate_id
            )));
        }

        if status != GateStatus::Infra {
            let metric_name = gate_config.metric.as_deref().unwrap_or("metric");
            if metric.is_none() {
                metric = extract_metric_from_text(&stdout_str, metric_name)
                    .or_else(|| extract_metric_from_text(&stderr_str, metric_name));
            }

            if let Some(current_metric) = metric {
                let baseline = crate::gate::read_baseline(
                    workspace_root,
                    &config.storage.state_dir,
                    &config.git.integration_branch,
                    gate_id,
                )?;

                let evaluation = crate::gate::evaluate_ratchet(
                    current_metric,
                    baseline.as_ref(),
                    direction_str,
                    metric_name,
                    &config.git.integration_branch,
                )?;

                if evaluation.is_regression {
                    status = evaluation.status;
                    exit_code = evaluation.exit_code;
                    summary = evaluation.summary;
                    agent_instruction = Some("fix_cited_failures".to_string());
                    failures.push(GateFailure {
                        location: gate_id.to_string(),
                        message: summary.clone(),
                    });
                } else if status == GateStatus::Pass {
                    summary = evaluation.summary;
                }
            } else {
                status = GateStatus::Fail;
                exit_code = 1;
                summary = format!("ratchet gate '{}' did not produce a numeric metric", gate_id);
                agent_instruction = Some("fix_cited_failures".to_string());
                failures.push(GateFailure {
                    location: gate_id.to_string(),
                    message: summary.clone(),
                });
            }
        }
    }

    let ev_status = match status {
        GateStatus::Pass => "pass",
        GateStatus::Fail => "fail",
        GateStatus::Infra => "infra",
        GateStatus::Skip => "pass",
    };
    let evidence_path = record_gate_evidence(
        workspace_root,
        config,
        gate_config,
        story_id.as_deref(),
        commit_sha.as_deref(),
        ev_status,
        exit_code,
        duration_ms,
        metric,
        &summary,
        Some(&stdout_str),
        Some(&stderr_str),
        false,
    )?;

    Ok(GateRunOutcome {
        gate_id: gate_id.to_string(),
        status,
        exit_code,
        duration_ms,
        summary,
        skipped_locally: false,
        commit_sha,
        story_id,
        stdout: Some(stdout_str),
        stderr: Some(stderr_str),
        agent_instruction,
        failures,
        metric,
        constraint_ids,
        evidence_path: Some(evidence_path),
    })
}

/// Validates gate dependencies: checks for unknown gate dependencies and cycles.
pub fn validate_gate_dependencies(gates: &[GateConfig]) -> Result<(), QdevError> {
    let mut seen = std::collections::HashSet::new();
    for gate in gates {
        if !seen.insert(&gate.id) {
            return Err(QdevError::usage_error(format!(
                "duplicate gate id '{}'",
                gate.id
            )));
        }
    }

    let known_ids: std::collections::HashSet<&str> = gates.iter().map(|g| g.id.as_str()).collect();

    // Check for dangling or unknown dependencies
    for gate in gates {
        for dep in &gate.depends_on {
            if !known_ids.contains(dep.as_str()) {
                return Err(QdevError::usage_error(format!(
                    "gate '{}' depends on unknown gate '{}'",
                    gate.id, dep
                )));
            }
        }
    }

    // Build (gate_id, dep_id) directed edges for cycle detection
    let mut edges = Vec::new();
    for gate in gates {
        for dep in &gate.depends_on {
            edges.push((gate.id.clone(), dep.clone()));
        }
    }

    if let Some(cycle) = crate::dag::find_dependency_cycle(&edges) {
        return Err(QdevError::usage_error(format!(
            "circular dependency detected in gates: {}",
            cycle.join(" -> ")
        )));
    }

    Ok(())
}

/// Resolves topological execution order using Kahn's algorithm with ties broken by declaration order in `gates`.
/// If `target_gate_ids` is provided, resolves execution order for those targets and their transitive dependencies.
/// If `target_gate_ids` is None, resolves execution order for all gates.
pub fn resolve_gate_execution_order(
    gates: &[GateConfig],
    target_gate_ids: Option<&[String]>,
) -> Result<Vec<String>, QdevError> {
    validate_gate_dependencies(gates)?;

    let gate_map: std::collections::HashMap<&str, &GateConfig> =
        gates.iter().map(|g| (g.id.as_str(), g)).collect();

    // 1. Determine the subset of gates to execute
    let selected_ids: std::collections::HashSet<String> = match target_gate_ids {
        Some(targets) => {
            let mut visited = std::collections::HashSet::new();
            let mut queue = std::collections::VecDeque::new();
            for target in targets {
                if !gate_map.contains_key(target.as_str()) {
                    return Err(QdevError::usage_error(format!(
                        "gate '{}' not found in configuration",
                        target
                    )));
                }
                if visited.insert(target.clone()) {
                    queue.push_back(target.clone());
                }
            }

            while let Some(current) = queue.pop_front() {
                if let Some(cfg) = gate_map.get(current.as_str()) {
                    for dep in &cfg.depends_on {
                        if visited.insert(dep.clone()) {
                            queue.push_back(dep.clone());
                        }
                    }
                }
            }
            visited
        }
        None => gates.iter().map(|g| g.id.clone()).collect(),
    };

    if selected_ids.is_empty() {
        return Ok(Vec::new());
    }

    // 2. Compute in-degrees: count of unsatisfied prerequisites in selected_ids.
    // If gate G depends on dep D, D must execute before G.
    // So in-degree(G) = number of dep in G.depends_on where dep is in selected_ids.
    let mut in_degree: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut dependents: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();

    for id in &selected_ids {
        let cfg = gate_map[id.as_str()];
        let mut deg = 0;
        for dep in &cfg.depends_on {
            if selected_ids.contains(dep) {
                deg += 1;
                dependents.entry(dep.as_str()).or_default().push(id.as_str());
            }
        }
        in_degree.insert(id.as_str(), deg);
    }

    // Declaration order index for tie breaking
    let order_index: std::collections::HashMap<&str, usize> = gates
        .iter()
        .enumerate()
        .map(|(idx, g)| (g.id.as_str(), idx))
        .collect();

    // 3. Kahn's algorithm
    let mut remaining: std::collections::HashSet<&str> =
        selected_ids.iter().map(|s| s.as_str()).collect();
    let mut execution_order = Vec::new();

    while !remaining.is_empty() {
        let mut ready: Vec<&str> = remaining
            .iter()
            .copied()
            .filter(|&id| in_degree.get(id).copied().unwrap_or(0) == 0)
            .collect();

        if ready.is_empty() {
            return Err(QdevError::usage_error("cycle detected in gate dependencies"));
        }

        ready.sort_by_key(|id| order_index.get(id).copied().unwrap_or(usize::MAX));

        let next = ready[0];
        execution_order.push(next.to_string());
        remaining.remove(next);

        if let Some(deps) = dependents.get(next) {
            for &dep_gate in deps {
                if let Some(deg) = in_degree.get_mut(dep_gate) {
                    if *deg > 0 {
                        *deg -= 1;
                    }
                }
            }
        }
    }

    Ok(execution_order)
}

/// Executes a set of gates in the given execution order, propagating skip cascades for dependency failures.
pub fn execute_gate_set(
    workspace_root: &Path,
    config: &Config,
    gate_ids: &[String],
    options: &GateRunOptions,
) -> Result<GateRunSetOutcome, QdevError> {
    let mut outcomes = Vec::new();
    let mut gate_status_map: std::collections::HashMap<String, GateStatus> =
        std::collections::HashMap::new();
    let mut skipped_locally_map: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();

    let commit_sha = resolve_commit_sha(workspace_root);
    let story_id = resolve_story_id(workspace_root, config, options);

    let gate_configs: std::collections::HashMap<&str, &crate::config::GateConfig> = config
        .gates
        .iter()
        .map(|g| (g.id.as_str(), g))
        .collect();

    for gate_id in gate_ids {
        let gate_config = gate_configs.get(gate_id.as_str()).ok_or_else(|| {
            QdevError::usage_error(format!("gate '{}' not found in configuration", gate_id))
        })?;

        // Check if any direct dependency failed (Fail, Infra, or non-local Skip)
        let mut failed_dep = None;
        for dep in &gate_config.depends_on {
            if let Some(&status) = gate_status_map.get(dep) {
                let is_local_skip = skipped_locally_map.get(dep).copied().unwrap_or(false);
                let is_failed = status == GateStatus::Fail
                    || status == GateStatus::Infra
                    || (status == GateStatus::Skip && !is_local_skip);
                if is_failed {
                    failed_dep = Some(dep.clone());
                    break;
                }
            } else {
                // Prerequisite dependency was not evaluated prior to this gate
                failed_dep = Some(dep.clone());
                break;
            }
        }

        if let Some(failed_dep_id) = failed_dep {
            let outcome = GateRunOutcome {
                gate_id: gate_id.clone(),
                status: GateStatus::Skip,
                exit_code: 0,
                duration_ms: 0,
                summary: format!("dependency failed: {}", failed_dep_id),
                skipped_locally: false,
                commit_sha: commit_sha.clone(),
                story_id: story_id.clone(),
                stdout: None,
                stderr: None,
                agent_instruction: None,
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
                evidence_path: None,
            };
            gate_status_map.insert(gate_id.clone(), GateStatus::Skip);
            skipped_locally_map.insert(gate_id.clone(), false);
            outcomes.push(outcome);
        } else {
            let outcome = execute_gate(workspace_root, config, gate_id, options)?;
            gate_status_map.insert(gate_id.clone(), outcome.status);
            skipped_locally_map.insert(gate_id.clone(), outcome.skipped_locally);
            outcomes.push(outcome);
        }
    }

    Ok(GateRunSetOutcome::new(outcomes))
}

fn scan_evidence_dir(
    dir: &Path,
    gate_latest: &mut std::collections::HashMap<String, (String, String)>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && !path.is_symlink() {
            scan_evidence_dir(&path, gate_latest);
        } else if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    let gate_id = v
                        .get("gate_id")
                        .or_else(|| v.get("gate"))
                        .and_then(|g| g.as_str());
                    let status = v.get("status").and_then(|s| s.as_str());
                    if let (Some(gid), Some(st)) = (gate_id, status) {
                        let timestamp = v
                            .get("ran_at")
                            .and_then(|t| t.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                entry
                                    .metadata()
                                    .ok()
                                    .and_then(|m| m.modified().ok())
                                    .map(|mtime| {
                                        let secs = mtime
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .map(|d| d.as_secs() as i64)
                                            .unwrap_or(0);
                                        crate::write::iso8601_from_timestamp(secs)
                                    })
                                    .unwrap_or_default()
                            });
                        let current = gate_latest
                            .entry(gid.to_string())
                            .or_insert_with(|| (String::new(), String::new()));
                        if timestamp >= current.0 {
                            *current = (timestamp, st.to_string());
                        }
                    }
                }
            }
        }
    }
}

/// Gathers gate list items with resolved `last_status` from SQLite store and `docs/state/evidence/`.
pub fn get_gate_list(
    workspace_root: &Path,
    config: &Config,
) -> Result<Vec<GateListItem>, QdevError> {
    let mut gate_latest: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();

    let cache_db_path = workspace_root
        .join(&config.storage.cache_dir)
        .join("cache.sqlite");

    if cache_db_path.is_file() {
        if let Ok(crate::store::CacheSchemaStatus::Valid) =
            crate::store::inspect_cache_schema(&cache_db_path)
        {
            if let Ok(store) = crate::store::SqliteStore::open(&cache_db_path) {
                if let Ok(runs) = store.list_gate_runs() {
                    for run in runs {
                        if let Some(status) = run.status {
                            let timestamp = run.ran_at.unwrap_or_default();
                            let entry = gate_latest
                                .entry(run.gate_id)
                                .or_insert_with(|| (String::new(), String::new()));
                            if timestamp >= entry.0 {
                                *entry = (timestamp, status);
                            }
                        }
                    }
                }
            }
        }
    }

    let evidence_dir = workspace_root
        .join(&config.storage.state_dir)
        .join("evidence");
    if evidence_dir.is_dir() {
        scan_evidence_dir(&evidence_dir, &mut gate_latest);
    }

    let mut items = Vec::new();
    for gate_config in &config.gates {
        let last_status = gate_latest
            .get(&gate_config.id)
            .map(|(_, status)| status.clone());

        items.push(GateListItem {
            id: gate_config.id.clone(),
            kind: gate_config
                .kind
                .clone()
                .unwrap_or_else(|| "command".to_string()),
            transitions: gate_config.on_transition.clone(),
            dependencies: gate_config.depends_on.clone(),
            last_status,
        });
    }

    Ok(items)
}

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
            if let Some(stripped) = rest.strip_prefix("public ")
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
                    "class" | "struct" | "enum" | "protocol" | "typealias" | "func" | "let" | "var" => {
                        parts.next()
                    }
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
        if c_str != "crates" && c_str != "src" && c_str != "tests" && c_str != "lib" && c_str != "pkg" && c_str != "packages" {
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

fn get_git_diff_files(
    workspace_root: &Path,
    integration_branch: &str,
) -> std::collections::HashSet<String> {
    let is_git = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !is_git {
        return std::collections::HashSet::new();
    }

    let mut changed = std::collections::HashSet::new();

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

    // Fallback: check if integration_branch ref exists directly
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

    // Include uncommitted changes (staged and unstaged + untracked)
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
                    if !path.is_empty() {
                        changed.insert(path.to_string());
                    }
                }
            }
        }
    }

    changed
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
                    if let Some(arr) = frontmatter.get("target_modules").and_then(|v| v.as_array()) {
                        for item in arr {
                            if let Some(s) = item.as_str() {
                                let trimmed = s.trim();
                                if !trimmed.is_empty() && !target_modules.contains(&trimmed.to_string()) {
                                    target_modules.push(trimmed.to_string());
                                }
                            }
                        }
                    }

                    let parse_no_gos = |val: &serde_json::Value, prefix: &str, out: &mut Vec<(String, String)>| {
                        if let Some(arr) = val.as_array() {
                            for c in arr {
                                if c.get("kind").and_then(|v| v.as_str()) == Some("no_go") {
                                    let cid = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    let text = c.get("text").and_then(|v| v.as_str())
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
                                    let cid = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                    let text = c.get("text").and_then(|v| v.as_str())
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
                                if let Ok(epic_fm) = crate::schema::extract_frontmatter(&epic_content) {
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

    let diff_files = get_git_diff_files(workspace_root, &config.git.integration_branch);
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
                if m_cfg.paths.iter().any(|pattern| crate::validate::glob_match(pattern, norm)) {
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
            format!("scope check failed: {} path(s) outside target_modules", failures.len()),
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
        let failure_lines: Vec<String> = failures.iter().map(|f| format!("{}: {}", f.location, f.message)).collect();
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
            format!("dependency check failed with {} violation(s)", failures.len()),
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
        let failure_lines: Vec<String> = failures.iter().map(|f| format!("{}: {}", f.location, f.message)).collect();
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

