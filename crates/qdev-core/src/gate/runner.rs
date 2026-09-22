use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::errors::QdevError;
use crate::gate::adapter::{parse_with_adapter, validate_adapter_name};
use crate::gate::process::ProcessGroupIsolation;
use crate::gate::result::GateResultDocument;
use crate::gate::ring_buffer::HeadTailBuffer;
use crate::gate::{GateRunOutcome, GateStatus};
use crate::lease::find_active_lease_with_storage;
use crate::modules::ModuleRegistry;

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
fn resolve_commit_sha(workspace_root: &Path) -> Option<String> {
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

/// Executes a verification gate external subprocess according to Story 3.2 specification.
pub fn execute_gate(
    workspace_root: &Path,
    config: &Config,
    gate_id: &str,
    options: &GateRunOptions,
) -> Result<GateRunOutcome, QdevError> {
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
        });
    }

    // 2. Resolve command
    let cmd_str = match &gate_config.command {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => {
            return Ok(GateRunOutcome {
                gate_id: gate_id.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms: 0,
                summary: "missing executable: <none>".to_string(),
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
            });
        }
    };

    let args = parse_command_args(cmd_str);
    if args.is_empty() {
        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms: 0,
            summary: "missing executable: <empty>".to_string(),
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: None,
            stderr: None,
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
        });
    }

    let binary_str = &args[0];
    let binary_args = &args[1..];

    // 3. Check if executable exists
    let (resolved_path, binary_exists) = resolve_binary(workspace_root, binary_str);
    if !binary_exists {
        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms: 0,
            summary: format!("missing executable: {}", resolved_path),
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: None,
            stderr: None,
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
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
            return Ok(GateRunOutcome {
                gate_id: gate_id.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms: 0,
                summary: format!("missing executable: {}", resolved_path),
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
            });
        }
        Err(e) => {
            return Ok(GateRunOutcome {
                gate_id: gate_id.to_string(),
                status: GateStatus::Infra,
                exit_code: 4,
                duration_ms: 0,
                summary: format!("failed to spawn executable {}: {}", resolved_path, e),
                skipped_locally: false,
                commit_sha,
                story_id,
                stdout: None,
                stderr: None,
                agent_instruction: Some("halt_and_alert".to_string()),
                failures: Vec::new(),
                metric: None,
                constraint_ids: Vec::new(),
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

        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms,
            summary: format!("timeout after {}ms", timeout_ms),
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: Some(stdout_buf.to_string_lossy()),
            stderr: Some(stderr_buf.to_string_lossy()),
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
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
        return Ok(GateRunOutcome {
            gate_id: gate_id.to_string(),
            status: GateStatus::Infra,
            exit_code: 4,
            duration_ms,
            summary: format!("process terminated by signal {}", sig),
            skipped_locally: false,
            commit_sha,
            story_id,
            stdout: Some(stdout_str),
            stderr: Some(stderr_str),
            agent_instruction: Some("halt_and_alert".to_string()),
            failures: Vec::new(),
            metric: None,
            constraint_ids: Vec::new(),
        });
    }

    let raw_exit_code = status.code().unwrap_or(-1);

    // 8. Output classification based on output_adapter configuration
    let (status, exit_code, summary, agent_instruction, failures, metric, constraint_ids) =
        match gate_config.output_adapter.as_deref() {
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
    })
}
