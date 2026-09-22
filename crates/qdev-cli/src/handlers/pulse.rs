use crate::cli::Cli;
use crate::output::OutputEmitter;
use qdev_core::{resolve_author, ExitCode, JsonEnvelope, QdevError};

/// The workspace pulse (Story 2.12), shared by the default command and its `status`
/// alias, emitted as the §5 human layout or a `--json` envelope.
///
/// Genuinely read-only: no boot-time `ensure_cache`, no sweep, no rebuild, no advisory
/// lock, no cache row, entity file, lease file, or decision record written. The cache is
/// opened only when it already exists *and* this binary can read it; a missing cache, a
/// mismatched stamp, missing tables, or an unparseable stamp leave the store `None`, so
/// the payload reports the cache as degraded (`schema_status: "mismatch"`, null counts,
/// `sprints` empty, `next: null`) and the command still exits 0. The one refusal is a
/// cache stamped *newer* than this binary supports — the same `schema_version_mismatch`
/// (exit 5) every other command gets, so the story-1.6 refusal contract holds here too.
/// Outside a workspace it short-circuits to the one-line init hint (D-1) — the store is
/// never opened, so no cache file is created.
pub fn handle_pulse(
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);
    let workspace = root.join("qdev.toml").is_file();

    // The store is opened only inside an initialized workspace, only when the cache file
    // already exists, and only when its stamp reads. `SqliteStore::open` creates the cache
    // file (and its parent directory), so opening it here would repair the workspace
    // instead of reporting it — and outside a workspace the D-1 contract is a hint with
    // every field null.
    let cache_db_path = root
        .join(&annotated_config.config.storage.cache_dir)
        .join("cache.sqlite");
    let store = if !workspace {
        None
    } else {
        match qdev_core::inspect_cache_schema(&cache_db_path) {
            // The parent directory necessarily exists, since the file does.
            Ok(qdev_core::CacheSchemaStatus::Valid) => {
                match qdev_core::SqliteStore::open(&cache_db_path) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        let _ = output.emit_error(&e);
                        return e.exit_code();
                    }
                }
            }
            // Refused, not rebuilt: a cache from a newer binary must never be read with the
            // wrong schema, and `qdev sync --rebuild` is the documented recovery.
            Ok(qdev_core::CacheSchemaStatus::NewerThanSupported { found, supported }) => {
                let e = qdev_core::newer_cache_conflict(found, supported);
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
            // `Mismatch` — missing cache, stale stamp, missing or extra tables, an
            // unparseable stamp — is reported, not repaired: no store, and nothing created.
            _ => None,
        }
    };

    let author = match resolve_author(None, None, annotated_config, &root) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::PulseOptions {
        workspace,
        workspace_root: &root,
        store: store.as_ref().map(|s| s as &dyn qdev_core::Store),
        config: &annotated_config.config,
        author,
        now: std::time::SystemTime::now(),
    };

    let payload = match qdev_core::build_pulse(&options) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(&payload);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit pulse envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_pulse_text(&payload);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit pulse output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

/// The pulse's Next section: the `render_next_text` shape, plus the selection's
/// `Run:` line (phase mapping `draft→specify`, `ready`/`in-progress→develop` —
/// decided D-5). `next: null` renders the summary plus the blocker lines.
fn render_pulse_next_text(selection: &qdev_core::NextSelection) -> String {
    let mut text = String::from("Next\n");
    if let Some(record) = &selection.next {
        text.push_str(&format!("  {}\n", selection.reason.summary));
        for note in &selection.reason.notes {
            text.push_str(&format!("  {note}\n"));
        }
        let phase = if record.status == "draft" {
            "specify"
        } else {
            "develop"
        };
        text.push_str(&format!(
            "  Run: /qdev-{} {}   (or: qdev context {} --phase {})\n",
            phase, record.id, record.id, phase
        ));
    } else {
        text.push_str(&format!(
            "  Nothing eligible: {}\n",
            selection.reason.summary
        ));
        for note in &selection.reason.notes {
            text.push_str(&format!("  {note}\n"));
        }
        if !selection.blockers.is_empty() {
            text.push_str("  Blockers\n");
            for blocker in &selection.blockers {
                match &blocker.story_id {
                    Some(story_id) => text.push_str(&format!(
                        "    {story_id} ({}): {}\n",
                        blocker.kind, blocker.detail
                    )),
                    None => text.push_str(&format!("    ({}): {}\n", blocker.kind, blocker.detail)),
                }
            }
        }
    }
    text
}

/// The first `HH:MM` of an ISO8601 timestamp for the Lease line (the §5 layout shows
/// "since 09:41"); anything shaped differently renders as-is.
fn lease_clock(started_at: &str) -> String {
    let chars: Vec<char> = started_at.chars().collect();
    if chars.len() >= 16 && chars[10] == 'T' && chars[13] == ':' {
        chars[11..16].iter().collect()
    } else {
        started_at.to_string()
    }
}

/// Human layout for the pulse (`docs/cli-reference.md` §5). Deterministic given the
/// payload: every line renders from stored data only.
fn render_pulse_text(payload: &qdev_core::PulsePayload) -> String {
    // D-1: outside a workspace the whole output is the one-line hint.
    if !payload.workspace {
        return "not a qdev workspace — run `qdev init` first\n".to_string();
    }

    let mut text = format!(
        "qdev {} — Development Engine & Gatekeeper\n",
        env!("CARGO_PKG_VERSION")
    );

    if let Some(env) = &payload.environment {
        text.push_str("Environment\n");
        match &env.working_tree {
            None => text.push_str("  Working tree   not a git repository\n"),
            Some(wt) => {
                let dirty = wt.dirty_files.unwrap_or(0);
                let tree = if wt.clean.unwrap_or(true) {
                    "clean".to_string()
                } else {
                    format!("dirty ({dirty} files)")
                };
                let branch = wt.branch.as_deref().unwrap_or("unknown");
                let head = wt.head.as_deref().unwrap_or("unknown");
                text.push_str(&format!("  Working tree   {tree} ({branch} @ {head})\n"));
            }
        }
        match &env.integration {
            None => text.push_str(
                "  Integration    not available — no git repository to compare against\n",
            ),
            Some(integration) => {
                let remote_ref = format!("{}/{}", integration.remote, integration.branch);
                let line = match integration.state.as_str() {
                    "up_to_date" => format!(
                        "{} is up to date with {}",
                        integration.branch, remote_ref
                    ),
                    "behind" => format!(
                        "{} is {} behind {}",
                        integration.branch,
                        integration.behind.unwrap_or(0),
                        remote_ref
                    ),
                    "ahead" => format!(
                        "{} is {} ahead of {}",
                        integration.branch,
                        integration.ahead.unwrap_or(0),
                        remote_ref
                    ),
                    "diverged" => format!(
                        "{} has diverged from {} ({} ahead, {} behind)",
                        integration.branch,
                        remote_ref,
                        integration.ahead.unwrap_or(0),
                        integration.behind.unwrap_or(0),
                    ),
                    "integration_branch_missing" => format!(
                        "integration branch '{}' not found locally (no remote update performed)",
                        integration.branch
                    ),
                    "remote_ref_missing" => format!(
                        "'{}' not found locally — run a remote update if needed (qdev never does)",
                        remote_ref
                    ),
                    "refs_missing" => format!(
                        "neither '{}' nor '{}' found locally (no remote update performed)",
                        integration.branch, remote_ref
                    ),
                    _ => format!(
                        "could not compare '{}' with '{}' — the comparison failed (no remote update attempted)",
                        integration.branch, remote_ref
                    ),
                };
                text.push_str(&format!("  Integration    {line}\n"));
            }
        }
        let cache = &env.cache;
        let cache_line = match cache.schema_status.as_str() {
            "ok" => format!(
                "healthy (synced {}, {} entities, {} findings)",
                match cache.synced_ms_ago {
                    Some(ms) => format!("{ms} ms ago"),
                    None => "never — run `qdev sync`".to_string(),
                },
                cache
                    .entity_count
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                cache
                    .finding_count
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "?".to_string()),
            ),
            _ => format!(
                "degraded (cache schema mismatch — run `qdev sync --rebuild`; {} entities, {} findings)",
                cache
                    .entity_count
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                cache
                    .finding_count
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "?".to_string()),
            ),
        };
        text.push_str(&format!("  Cache          {cache_line}\n"));
        match &env.lease {
            None => text.push_str("  Lease          none — no lease held by this worktree\n\n"),
            Some(leases) => {
                // Multiple this-worktree leases render one line each, sorted by story id.
                for (i, lease) in leases.iter().enumerate() {
                    let label = if i == 0 {
                        "  Lease          ".to_string()
                    } else {
                        "                 ".to_string()
                    };
                    text.push_str(&format!(
                        "{}{} held by {} since {} (this worktree)\n",
                        label,
                        lease.story_id,
                        lease.holder,
                        lease_clock(&lease.started_at)
                    ));
                }
                text.push('\n');
            }
        }
    }

    // The gate summary is workspace-wide (`N/M passing` over every `gate_runs` row), so it
    // renders once — in the first sprint block, or right after the "no active sprints"
    // line — never once per block, and never omitted just because no sprint is active.
    let gates_line = payload.gates.map(|gates| {
        format!(
            "  Gates          {}/{} passing\n",
            gates.passing, gates.total
        )
    });
    let sprints = payload.sprints.clone().unwrap_or_default();
    if sprints.is_empty() {
        text.push_str("Sprints\n  No active sprints — nothing to report\n");
        if let Some(gates) = &gates_line {
            text.push_str(gates);
        }
        text.push('\n');
    } else {
        for (i, sprint) in sprints.iter().enumerate() {
            let header = match (&sprint.title, &sprint.release) {
                (Some(title), Some(release)) => {
                    format!("Sprint {} — {title}  [release {release}]\n", sprint.id)
                }
                (Some(title), None) => format!("Sprint {} — {title}\n", sprint.id),
                (None, Some(release)) => {
                    format!("Sprint {}  [release {release}]\n", sprint.id)
                }
                (None, None) => format!("Sprint {}\n", sprint.id),
            };
            text.push_str(&header);
            let counters = &sprint.stories;
            text.push_str(&format!(
                "  Stories        {} done / {} in progress / {} blocked / {} backlog\n",
                counters.done, counters.in_progress, counters.blocked, counters.backlog
            ));
            text.push_str(&format!(
                "  Deferred work  {} open ({} unacceptable)\n",
                sprint.deferred_work.open, sprint.deferred_work.unacceptable
            ));
            // Only the first block carries the workspace-wide count (decided D-4: the
            // line exists only while `gate_runs` evidence does, and one count covers
            // every sprint).
            if i == 0 {
                if let Some(gates) = &gates_line {
                    text.push_str(gates);
                }
            }
            text.push('\n');
        }
    }

    if let Some(selection) = &payload.next {
        text.push_str(&render_pulse_next_text(selection));
    } else if payload.workspace
        && payload
            .environment
            .as_ref()
            .is_some_and(|env| env.cache.schema_status != "ok")
    {
        // With no readable cache `select_next` cannot run, so there is no selection to
        // embed. The section still renders — saying that nothing can be recommended —
        // instead of vanishing, and names `qdev sync` as the fix the user has to run.
        text.push_str(
            "Next\n  Nothing to recommend — there is no readable cache to select from \
             (run `qdev sync`)\n",
        );
    }

    text
}
