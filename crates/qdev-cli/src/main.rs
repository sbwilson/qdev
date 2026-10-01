mod cli;
mod handlers;
mod output;

use std::collections::BTreeMap;
use std::fs;
use std::io::IsTerminal;
use std::panic;
use std::process::ExitCode as StdExitCode;

use clap::Parser;
use qdev_core::{
    serde_yaml, ExitCode, Interactivity, JsonEnvelope, JsonErrorEnvelope, QdevError, Store,
    CACHE_SCHEMA_VERSION,
};
use serde::Serialize;

use cli::{is_json_requested, Cli, Commands, ConfigCommands, CreateCommands};
use output::OutputEmitter;

#[derive(Serialize)]
struct VersionPayload {
    version: String,
}

#[derive(Serialize)]
struct CreateStoryPayload {
    id: String,
    path: String,
}

fn main() -> StdExitCode {
    let raw_args: Vec<String> = std::env::args().collect();
    let json_mode = is_json_requested(&raw_args);

    if json_mode {
        // Set a panic hook that emits an AD-13 error envelope on stdout and avoids unformatted panic traces
        panic::set_hook(Box::new(|panic_info| {
            let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
                (*s).to_string()
            } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
                s.clone()
            } else {
                "Internal panic occurred".to_string()
            };

            let location_details = panic_info.location().map(|loc| {
                serde_json::json!({
                    "file": loc.file(),
                    "line": loc.line(),
                    "column": loc.column(),
                })
            });

            let envelope = JsonErrorEnvelope::new(
                "infrastructure_failure",
                format!("Internal error: {}", message),
                location_details,
            );

            let _ = OutputEmitter::emit_error_envelope(&envelope);
        }));
    }

    let result = panic::catch_unwind(|| run(&raw_args));

    let exit_code = match result {
        Ok(code) => code,
        Err(_) => ExitCode::InfrastructureFailure.as_i32(),
    };

    StdExitCode::from(exit_code as u8)
}

fn normalize_raw_args(raw_args: &[String]) -> Vec<String> {
    let is_scratch_append = raw_args
        .windows(2)
        .any(|w| w[0] == "scratch" && w[1] == "append");
    if !is_scratch_append {
        return raw_args.to_vec();
    }

    let dash_dash_pos = raw_args.iter().position(|a| a == "--");
    let dash_dash_idx = match dash_dash_pos {
        Some(idx) => idx,
        None => return raw_args.to_vec(),
    };

    let mut before = raw_args[..dash_dash_idx].to_vec();
    let after = &raw_args[dash_dash_idx + 1..];

    if after.is_empty() {
        return raw_args.to_vec();
    }

    // The first token immediately following `--` is the entry text operand.
    // Preserve it as positional text and only scan subsequent tokens for trailing flags.
    let positional_text = &after[0];
    let mut hoisted_flags = Vec::new();
    let mut remaining_positional = vec![positional_text.clone()];

    let mut i = 1;
    while i < after.len() {
        let arg = &after[i];
        if arg == "--override" || arg == "--json" || arg == "--non-interactive" {
            hoisted_flags.push(arg.clone());
            i += 1;
        } else if arg == "--justification"
            || arg == "--kind"
            || arg == "--author-type"
            || arg == "--author-id"
        {
            hoisted_flags.push(arg.clone());
            if i + 1 < after.len() && !after[i + 1].starts_with("--") {
                hoisted_flags.push(after[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        } else if arg.starts_with("--justification=")
            || arg.starts_with("--kind=")
            || arg.starts_with("--author-type=")
            || arg.starts_with("--author-id=")
        {
            hoisted_flags.push(arg.clone());
            i += 1;
        } else {
            remaining_positional.push(arg.clone());
            i += 1;
        }
    }

    if hoisted_flags.is_empty() {
        return raw_args.to_vec();
    }

    before.extend(hoisted_flags);
    before.push("--".to_string());
    before.extend(remaining_positional);
    before
}

fn run(raw_args: &[String]) -> i32 {
    let raw_args = normalize_raw_args(raw_args);
    let json_mode = is_json_requested(&raw_args);
    let output = OutputEmitter::new(json_mode);

    let cli = match Cli::try_parse_from(&raw_args) {
        Ok(cli) => cli,
        Err(clap_err) => {
            if clap_err.kind() == clap::error::ErrorKind::DisplayHelp {
                print!("{}", clap_err);
                return ExitCode::Success.as_i32();
            }

            let err_msg = clap_err.to_string();
            let qdev_err = QdevError::usage_error(err_msg.trim());
            let _ = output.emit_error(&qdev_err);
            return ExitCode::UsageError.as_i32();
        }
    };

    // Handle --version flag
    if cli.version {
        if cli.json {
            let envelope = JsonEnvelope::new(VersionPayload {
                version: env!("CARGO_PKG_VERSION").to_string(),
            });
            if let Err(e) = output.emit_envelope(&envelope) {
                let err = QdevError::infrastructure_failure(
                    "io_error",
                    format!("Failed to emit version envelope: {}", e),
                );
                let _ = output.emit_error(&err);
                return ExitCode::InfrastructureFailure.as_i32();
            }
        } else {
            println!("qdev {}", env!("CARGO_PKG_VERSION"));
        }
        return ExitCode::Success.as_i32();
    }

    // Resolve interactivity per AD-12
    let env_var = std::env::var("QDEV_NONINTERACTIVE").ok();
    let is_stdin_tty = std::io::stdin().is_terminal()
        || std::env::var("_QDEV_MOCK_TTY")
            .map(|v| v == "1")
            .unwrap_or(false);
    let interactivity =
        Interactivity::resolve(cli.non_interactive, env_var.as_deref(), is_stdin_tty);

    // Boot-time configuration loading per spec-1-2
    let current_dir = match std::env::current_dir() {
        Ok(dir) => dir,
        Err(e) => {
            let qdev_err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to determine current working directory: {}", e),
            );
            let _ = output.emit_error(&qdev_err);
            return ExitCode::InfrastructureFailure.as_i32();
        }
    };

    // Dispatch schema command before loading config so it works without an initialized workspace
    if let Some(Commands::Schema(ref schema_args)) = cli.command {
        return handle_schema(schema_args, &cli, &output).as_i32();
    }

    // `init` is dispatched *after* this, not before: it resolves its layout through the same
    // loader as every other command, so an unparseable or invalid configuration file is the same
    // exit-2 refusal from the tool's own remedy as from everything else, and what `init`
    // scaffolds is what the next command will read. Bootstrapping still works from nothing —
    // both files absent is not an error, it is the default configuration.
    let annotated_config = match qdev_core::load_config(&current_dir) {
        Ok(cfg) => cfg,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code().as_i32();
        }
    };

    let root = qdev_core::find_workspace_root(&current_dir);

    if let Some(Commands::Init(ref init_args)) = cli.command {
        return handle_init(
            init_args,
            interactivity,
            &cli,
            &output,
            &root,
            &annotated_config,
        )
        .as_i32();
    }

    // Every command that touches the cache is refused outside an initialized workspace, checked
    // once here rather than per handler. Opening the store directly creates an empty,
    // schema-less `cache.sqlite`, so a command that forgot the check would litter the directory
    // and fail with a raw `sqlite_error: no such table` instead of a clean usage error.
    if requires_workspace(cli.command.as_ref()) {
        if let Err(e) = ensure_query_workspace(&root) {
            let _ = output.emit_error(&e);
            return e.exit_code().as_i32();
        }
    }

    // Boot-time cache verification and initialization per spec-1-6 (in initialized workspaces).
    //
    // The pulse is deliberately excluded: `ensure_cache` either rebuilds the cache from the
    // Markdown files or runs `sweep_workspace`, and the sweep stamps
    // `sync_meta (id, last_synced_at)` with the current time — a cache mutation. Story 2.12
    // requires the default command and its `status` alias to be genuinely read-only: an
    // unsynced workspace is *reported* as stale cache, never repaired, so neither command may
    // trigger the boot rebuild or sweep. `context` (Story 4.1) is excluded for the same
    // reason: the projection is read-only by contract — no cache writes, no file writes, no
    // git mutations — so it reports on the last-synced cache and never repairs it.
    // Everything else keeps its boot behaviour untouched.
    let mut boot_summary: Option<qdev_core::SweepSummary> = None;
    let skip_boot_ensure = match cli.command {
        None | Some(Commands::Status) | Some(Commands::Context(_)) => true,
        Some(Commands::Doctor(ref args)) if args.fix => {
            let cache_db_path = root
                .join(&annotated_config.config.storage.cache_dir)
                .join("cache.sqlite");
            if !cache_db_path.exists() {
                false
            } else {
                !matches!(
                    qdev_core::inspect_cache_schema(&cache_db_path),
                    Ok(qdev_core::CacheSchemaStatus::Valid)
                )
            }
        }
        _ => false,
    };

    if root.join("qdev.toml").is_file() && !skip_boot_ensure {
        match qdev_core::ensure_cache_with_summary(&root, &annotated_config.config.storage) {
            Ok((_, summary)) => boot_summary = Some(summary),
            Err(e) => {
                // A cache stamped by a newer binary is refused on boot for every command — except
                // `qdev sync --rebuild`, the documented recovery path the refusal itself names.
                // That command drops and repopulates every table from the Markdown files, so it is
                // the one caller that does not need to read the newer cache first. Every other
                // command, `qdev doctor` included, still exits 5 here — the pulse included too,
                // which performs this check itself in `handle_pulse`.
                let recoverable_by_this_command = e.code() == "schema_version_mismatch"
                    && matches!(cli.command, Some(Commands::Sync(ref args)) if args.rebuild);
                if !recoverable_by_this_command {
                    let _ = output.emit_error(&e);
                    return e.exit_code().as_i32();
                }
            }
        }
    }

    // Dispatch commands
    match cli.command {
        None | Some(Commands::Status) => {
            handlers::pulse::handle_pulse(&annotated_config, &cli, &output, &current_dir).as_i32()
        }
        Some(Commands::Config(config_args)) => match config_args.command {
            ConfigCommands::Show => {
                if cli.json {
                    let envelope = JsonEnvelope::new(annotated_config);
                    if let Err(e) = output.emit_envelope(&envelope) {
                        let err = QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to emit config envelope: {}", e),
                        );
                        let _ = output.emit_error(&err);
                        return ExitCode::InfrastructureFailure.as_i32();
                    }
                } else {
                    let report = annotated_config.to_text_report();
                    if let Err(e) = output.emit_text(&report) {
                        let err = QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to emit config report: {}", e),
                        );
                        let _ = output.emit_error(&err);
                        return ExitCode::InfrastructureFailure.as_i32();
                    }
                }
                ExitCode::Success.as_i32()
            }
        },
        Some(Commands::Create(ref create_args)) => match create_args.command {
            CreateCommands::Story(ref story_args) => {
                handle_create_story(story_args, &annotated_config, &cli, &output, &current_dir)
                    .as_i32()
            }
        },
        Some(Commands::Update(ref update_args)) => handle_update(
            update_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Transition(ref transition_args)) => handlers::transition::handle_transition(
            transition_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Get(ref get_args)) => {
            handle_get(get_args, &annotated_config, &cli, &output, &current_dir).as_i32()
        }
        Some(Commands::List(ref list_args)) => {
            handle_list(list_args, &annotated_config, &cli, &output, &current_dir).as_i32()
        }
        Some(Commands::Relate(ref relate_args)) => handle_relate(
            relate_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Unrelate(ref unrelate_args)) => handle_unrelate(
            unrelate_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Constraint(ref constraint_args)) => handlers::constraint::handle_constraint(
            constraint_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Graph(ref graph_args)) => {
            handle_graph(graph_args, &annotated_config, &cli, &output, &current_dir).as_i32()
        }
        Some(Commands::Validate(ref validate_args)) => handle_validate(
            validate_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Sync(ref sync_args)) => handle_sync(
            sync_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            boot_summary.as_ref(),
        )
        .as_i32(),
        Some(Commands::Doctor(ref doctor_args)) => handle_doctor(
            doctor_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Claim(ref claim_args)) => handlers::claim::handle_claim(
            claim_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Release(ref release_args)) => handlers::claim::handle_release(
            release_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Scratch(ref scratch_args)) => handlers::scratch::handle_scratch(
            scratch_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Decision(ref decision_args)) => handlers::decision::handle_decision(
            decision_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Dw(ref dw_args)) => handlers::dw::handle_dw(
            dw_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        )
        .as_i32(),
        Some(Commands::Sprint(ref sprint_args)) => handlers::sprint::handle_sprint(
            sprint_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Review(ref review_args)) => handlers::review::handle_review(
            review_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Chore(ref chore_args)) => handlers::chore::handle_chore(
            chore_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Next(ref next_args)) => {
            handlers::next::handle_next(next_args, &annotated_config, &cli, &output, &current_dir)
                .as_i32()
        }
        Some(Commands::Context(ref context_args)) => handlers::context::handle_context(
            context_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Gate(ref gate_args)) => {
            handlers::gate::handle_gate(gate_args, &annotated_config, &cli, &output, &root).as_i32()
        }
        Some(Commands::Soup(ref soup_args)) => {
            handlers::soup::handle_soup(soup_args, &annotated_config, &cli, &output, &root).as_i32()
        }
        Some(Commands::Preflight(ref preflight_args)) => handlers::preflight::handle_preflight(
            preflight_args,
            &annotated_config,
            &cli,
            &output,
            &root,
        )
        .as_i32(),
        Some(Commands::Install(ref install_args)) => handlers::install::handle_install(
            install_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Hook(ref hook_args)) => {
            handlers::hook::handle_hook(hook_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::Hygiene(ref hygiene_args)) => handlers::hygiene::handle_hygiene(
            hygiene_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        )
        .as_i32(),
        Some(Commands::Impact(ref impact_args)) => {
            handlers::impact::handle_impact(impact_args, &annotated_config, &cli, &output, &root)
                .as_i32()
        }
        Some(Commands::Mcp(ref mcp_args)) => {
            handlers::mcp::handle_mcp(mcp_args, &annotated_config, &cli, &current_dir).as_i32()
        }
        Some(Commands::Init(_)) => unreachable!(),
        Some(Commands::Schema(_)) => unreachable!(),
    }
}

/// Resolves a `<target> [<id>]` argument pair into `(kind_hint, entity_id)`, the shape shared
/// by every command that addresses an entity: `<kind> <id>` gives both, and a bare `<id>` gives
/// the id alone (the cache's `entities.id` is globally unique, so no kind is needed). A bare
/// argument that names a *kind* is a missing id, not an entity.
///
/// One implementation rather than a copy per handler: `qdev get E12S4` and `qdev update E12S4`
/// must not be able to drift on how they read the same argument.
pub(crate) fn resolve_kind_and_id(
    target: &str,
    id: Option<&str>,
) -> Result<(Option<qdev_core::EntityKind>, String), QdevError> {
    match id {
        Some(id_str) => {
            let kind = qdev_core::EntityKind::from_str_loose(target)?;
            Ok((Some(kind), id_str.to_string()))
        }
        None if qdev_core::EntityKind::from_str_loose(target).is_ok() => Err(
            QdevError::usage_error(format!("Missing entity ID for kind '{}'", target)),
        ),
        None => Ok((None, target.to_string())),
    }
}

/// Rejects a filter flag given an empty value. An empty value is almost always an unset shell
/// variable rather than a request for "everything with a blank owner": matching it literally
/// returns nothing and is indistinguishable from a legitimately empty result.
pub(crate) fn reject_empty_filter_values(flags: &[(&str, Option<&str>)]) -> Result<(), QdevError> {
    for (flag, value) in flags {
        if value.is_some_and(|v| v.trim().is_empty()) {
            return Err(QdevError::usage_error(format!(
                "'{}' was given an empty value",
                flag
            )));
        }
    }
    Ok(())
}

/// Whether `command` needs an initialized qdev workspace (a `qdev.toml`) to run.
///
/// The exceptions are deliberate: the default command and its `status` alias render
/// the workspace pulse (Story 2.12) — outside a workspace they short-circuit to the
/// one-line init hint (D-1) and never open the cache, inside one they only read it.
/// `config show` reports on whatever it finds, `init` and `schema` are dispatched
/// before this point, and `create story` bootstraps — it is specified and tested to
/// work in a clean directory, allocating the first id and writing the story file.
/// Outside a workspace it deliberately leaves no `.qdev/` behind: no cache (an
/// unstamped one is a v0 cache the next `qdev init` would drop and rebuild) and no
/// advisory lock (there is no other writer to serialize against). Everything else
/// reads or writes an existing cache.
///
/// Deciding this from the command itself, rather than from a call inside each handler, is what
/// stops the next new command from silently shipping without the guard — the omission that let
/// `qdev update` keep creating a stray, schema-less `cache.sqlite` outside a workspace and fail
/// with a raw `sqlite_error` after `get`/`list` were fixed.
fn requires_workspace(command: Option<&Commands>) -> bool {
    match command {
        None
        | Some(Commands::Status)
        | Some(Commands::Init(_))
        | Some(Commands::Schema(_))
        | Some(Commands::Preflight(_))
        | Some(Commands::Install(_))
        | Some(Commands::Hook(_))
        | Some(Commands::Hygiene(_)) => false,
        // Matched at subcommand granularity, not by whole variant: a future `create epic` or
        // `config set` must be classified deliberately rather than inheriting the exemption.
        Some(Commands::Create(args)) => match args.command {
            CreateCommands::Story(_) => false,
        },
        Some(Commands::Config(args)) => match args.command {
            ConfigCommands::Show => false,
        },
        Some(Commands::Update(_))
        | Some(Commands::Transition(_))
        | Some(Commands::Get(_))
        | Some(Commands::List(_))
        | Some(Commands::Relate(_))
        | Some(Commands::Unrelate(_))
        | Some(Commands::Constraint(_))
        | Some(Commands::Graph(_))
        | Some(Commands::Validate(_))
        | Some(Commands::Sync(_))
        | Some(Commands::Doctor(_))
        | Some(Commands::Claim(_))
        | Some(Commands::Release(_))
        | Some(Commands::Scratch(_))
        | Some(Commands::Decision(_))
        | Some(Commands::Dw(_))
        | Some(Commands::Sprint(_))
        | Some(Commands::Review(_))
        | Some(Commands::Chore(_))
        | Some(Commands::Next(_))
        | Some(Commands::Context(_))
        | Some(Commands::Gate(_))
        | Some(Commands::Soup(_))
        | Some(Commands::Impact(_))
        | Some(Commands::Mcp(cli::McpArgs {
            command: cli::McpCommands::Serve,
        })) => true,
    }
}

fn handle_schema(schema_args: &cli::SchemaArgs, cli: &Cli, output: &OutputEmitter) -> ExitCode {
    if schema_args.kind.trim().eq_ignore_ascii_case("payload") {
        return handle_schema_payload(schema_args, cli, output);
    }

    if let Some(extra) = &schema_args.name {
        let e = QdevError::usage_error(format!(
            "Unexpected extra argument '{}'. Usage: qdev schema <entity-kind>",
            extra
        ));
        let _ = output.emit_error(&e);
        return ExitCode::UsageError;
    }

    let kind = match qdev_core::EntityKind::from_str_loose(&schema_args.kind) {
        Ok(k) => k,
        Err(e) => {
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(kind.schema_json());
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit schema envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let schema_text = kind.pretty_schema_str();
        if let Err(e) = output.emit_text(&schema_text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit schema: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

/// Handles `qdev schema payload <name>`: prints a hand-authored JSON Schema for a CLI output
/// payload shape, resolved via `PayloadKind` — entirely separate from the `EntityKind` path above.
fn handle_schema_payload(
    schema_args: &cli::SchemaArgs,
    cli: &Cli,
    output: &OutputEmitter,
) -> ExitCode {
    let name = match &schema_args.name {
        Some(n) => n,
        None => {
            let e = QdevError::usage_error(format!(
                "Missing payload name. Usage: qdev schema payload <name>. Valid payload names: {}",
                qdev_core::PayloadKind::valid_names()
            ));
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    };

    let payload = match qdev_core::PayloadKind::from_str_loose(name) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return ExitCode::UsageError;
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(payload.schema_json());
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit payload schema envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let schema_text = payload.pretty_schema_str();
        if let Err(e) = output.emit_text(&schema_text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit payload schema: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

fn handle_init(
    init_args: &cli::InitArgs,
    interactivity: Interactivity,
    cli: &Cli,
    output: &OutputEmitter,
    root: &std::path::Path,
    annotated_config: &qdev_core::AnnotatedConfig,
) -> ExitCode {
    let root = root.to_path_buf();

    // One resolver. The effective layout is the loader's merged answer — the same value
    // `ensure_cache` is handed on every other command — and the project layout is what
    // `qdev.toml` alone says, needed because `.gitignore` is committed.
    let project_storage = match qdev_core::load_project_storage(&root) {
        Ok(storage) => storage,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };
    let layout =
        qdev_core::InitLayout::new(annotated_config.config.storage.clone(), project_storage);

    let (name, developer, teams) = if interactivity.is_non_interactive() {
        // Non-interactive mode: strict flag requirements
        let name = match init_args
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(n) => n.to_string(),
            None => {
                let err = QdevError::policy_refusal(
                    "needs_confirmation",
                    "Missing required flag '--name' in non-interactive mode",
                )
                .with_details(serde_json::json!({
                    "flag": "--name"
                }))
                .with_attribution(
                    qdev_core::RejectionAttribution::new(
                        "Required initialization flags must be provided in non-interactive mode",
                    )
                    .with_policy("init_parameters"),
                );
                let _ = output.emit_error(&err);
                return ExitCode::PolicyRefusal;
            }
        };

        let developer = match init_args
            .developer
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(d) => d.to_string(),
            None => {
                let err = QdevError::policy_refusal(
                    "needs_confirmation",
                    "Missing required flag '--developer' in non-interactive mode",
                )
                .with_details(serde_json::json!({
                    "flag": "--developer"
                }))
                .with_attribution(
                    qdev_core::RejectionAttribution::new(
                        "Required initialization flags must be provided in non-interactive mode",
                    )
                    .with_policy("init_parameters"),
                );
                let _ = output.emit_error(&err);
                return ExitCode::PolicyRefusal;
            }
        };

        let teams: Vec<String> = init_args
            .team
            .iter()
            .flat_map(|t| t.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        if teams.is_empty() {
            let err = QdevError::policy_refusal(
                "needs_confirmation",
                "Missing required flag '--team' in non-interactive mode",
            )
            .with_details(serde_json::json!({
                "flag": "--team"
            }))
            .with_attribution(
                qdev_core::RejectionAttribution::new(
                    "Required initialization flags must be provided in non-interactive mode",
                )
                .with_policy("init_parameters"),
            );
            let _ = output.emit_error(&err);
            return ExitCode::PolicyRefusal;
        }

        // No cache check here: an older cache needs no confirmation, and `qdev_core::init`
        // inspects the cache upfront — before it touches the filesystem — so a cache stamped by
        // a newer binary is still the exit-5 refusal, raised in one place rather than two.
        (name, developer, teams)
    } else {
        // Interactive mode: TTY prompt wizard
        let name = if let Some(n) = init_args
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            n.to_string()
        } else {
            match prompt_input("Project name: ") {
                Ok(n) if !n.is_empty() => n,
                Ok(_) => {
                    let err = QdevError::policy_refusal(
                        "needs_confirmation",
                        "Project name cannot be empty",
                    )
                    .with_details(serde_json::json!({ "flag": "--name" }));
                    let _ = output.emit_error(&err);
                    return ExitCode::PolicyRefusal;
                }
                Err(e) => {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to read project name: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            }
        };

        let developer = if let Some(d) = init_args
            .developer
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            d.to_string()
        } else {
            let git_email = qdev_core::resolve_git_email(Some(&root));
            let prompt = if let Some(ref email) = git_email {
                format!("Developer ID [{}]: ", email)
            } else {
                "Developer ID: ".to_string()
            };
            match prompt_input(&prompt) {
                Ok(d) if !d.is_empty() => d,
                Ok(_) if git_email.is_some() => git_email.unwrap(),
                Ok(_) => {
                    let err = QdevError::policy_refusal(
                        "needs_confirmation",
                        "Developer ID cannot be empty",
                    )
                    .with_details(serde_json::json!({ "flag": "--developer" }));
                    let _ = output.emit_error(&err);
                    return ExitCode::PolicyRefusal;
                }
                Err(e) => {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to read developer ID: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            }
        };

        let mut teams: Vec<String> = init_args
            .team
            .iter()
            .flat_map(|t| t.split(','))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        if teams.is_empty() {
            match prompt_input("Team(s) (comma-separated): ") {
                Ok(t) => {
                    teams = t
                        .split(',')
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect();
                    if teams.is_empty() {
                        let err = QdevError::policy_refusal(
                            "needs_confirmation",
                            "At least one team must be specified",
                        )
                        .with_details(serde_json::json!({ "flag": "--team" }));
                        let _ = output.emit_error(&err);
                        return ExitCode::PolicyRefusal;
                    }
                }
                Err(e) => {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to read teams: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            }
        }

        // No migration prompt: the wizard asks about the project, not about the cache. An older
        // cache is migrated, exactly as it is on every other command's boot.
        (name, developer, teams)
    };

    let options = qdev_core::InitOptions {
        root,
        name,
        developer,
        teams,
        layout,
    };

    let result = match qdev_core::init(&options) {
        Ok(res) => res,
        Err(err) => {
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(result);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit init envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        // Rendered from the result, so every path named here is a path this run resolved and
        // built. Hardcoded default paths sent a reader — human or wrapper — to a directory that
        // does not exist whenever the workspace was configured for another layout.
        // The two config files are reported only when this run actually wrote them: `init` never
        // overwrites an existing one, so an idempotent re-run claiming to have created them
        // breaks the same rule the hardcoded paths broke.
        for name in [
            qdev_core::PROJECT_CONFIG_FILENAME,
            qdev_core::LOCAL_CONFIG_FILENAME,
        ] {
            if result.created_files.iter().any(|f| f == name) {
                if name == qdev_core::LOCAL_CONFIG_FILENAME {
                    println!("✔ {} (gitignored)", name);
                } else {
                    println!("✔ {}", name);
                }
            }
        }
        println!(
            "✔ {}/ (gitignored), {}/gates/",
            result.storage.cache_dir, result.qdev_dir
        );
        println!(
            "✔ {}/{{{}}}",
            result.storage.specs_dir,
            qdev_core::SPEC_SUBDIRECTORIES.join(",")
        );
        println!(
            "✔ {}/{{{}}}",
            result.storage.state_dir,
            qdev_core::STATE_SUBDIRECTORIES.join(",")
        );
        if result.cache_migrated {
            // The rehydration count is the half that says whether the migration worked: a
            // migration drops every table, so `0 files` on a populated workspace means the
            // repopulation found nothing — a wrong `[storage]` layout, say.
            println!(
                "✔ cache schema migrated to v{} ({} files rehydrated)",
                CACHE_SCHEMA_VERSION,
                result.cache_files_rehydrated.unwrap_or(0)
            );
        } else if result.already_initialized {
            println!("✔ cache schema v{} up to date", CACHE_SCHEMA_VERSION);
        } else {
            println!("✔ cache schema v{}", CACHE_SCHEMA_VERSION);
        }
    }

    ExitCode::Success
}

pub(crate) fn prompt_input(prompt: &str) -> std::io::Result<String> {
    use std::io::{self, Write};
    eprint!("{}", prompt);
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

pub(crate) enum GovernanceOutcome {
    Proceed {
        justification: Option<String>,
        override_decision: Option<(String, String)>,
    },
    Aborted,
}

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)] // Interactive governance prompt displays retained entity title to user.
pub(crate) fn check_governance_gate(
    root: &std::path::Path,
    target_id: &str,
    target_kind: Option<qdev_core::EntityKind>,
    author: &qdev_core::Author,
    annotated_config: &qdev_core::AnnotatedConfig,
    interactivity: Interactivity,
    cli: &Cli,
    output: &OutputEmitter,
    if_version: &mut Option<u64>,
) -> Result<GovernanceOutcome, ExitCode> {
    let opt_store = open_query_store(root, annotated_config).ok();
    let classification = match qdev_core::classify_mutation(
        root,
        target_id,
        target_kind,
        author,
        &annotated_config.config,
        opt_store.as_ref().map(|s| s as &dyn qdev_core::Store),
    ) {
        Ok(c) => c,
        Err(e) => {
            let _ = output.emit_error(&e);
            return Err(e.exit_code());
        }
    };

    if !classification.is_out_of_lease && !classification.is_cross_team {
        return Ok(GovernanceOutcome::Proceed {
            justification: None,
            override_decision: None,
        });
    }

    if cli.r#override {
        let just = cli.justification.as_deref().unwrap_or("").trim();
        if just.is_empty() {
            let attribution = qdev_core::RejectionAttribution::new(
                "Governance override requires non-empty justification",
            )
            .with_policy("justification_required");

            let err = qdev_core::QdevError::policy_refusal(
                "needs_justification",
                "--override requires a non-empty --justification",
            )
            .with_details(serde_json::json!({
                "target_id": target_id,
            }))
            .with_attribution(attribution);
            let _ = output.emit_error(&err);
            return Err(ExitCode::PolicyRefusal);
        }

        let decision_type = if classification.is_cross_team {
            "cross_team_override"
        } else {
            "lease_override"
        };

        return Ok(GovernanceOutcome::Proceed {
            justification: Some(just.to_string()),
            override_decision: Some((decision_type.to_string(), just.to_string())),
        });
    }

    if interactivity.is_non_interactive() {
        let (policy, rule, msg) = if classification.is_cross_team && classification.is_out_of_lease
        {
            (
                "cross_team_governance",
                "Non-interactive mutations crossing team ownership boundaries and lease scope require override and justification",
                format!(
                    "Mutation on entity '{}' is out-of-lease and cross-team; re-run with --override --justification \"<rationale>\"",
                    target_id
                ),
            )
        } else if classification.is_cross_team {
            (
                "cross_team_governance",
                "Non-interactive mutations crossing team ownership boundaries require override and justification",
                format!(
                    "Cross-team mutation on entity '{}' requires confirmation; re-run with --override --justification \"<rationale>\"",
                    target_id
                ),
            )
        } else {
            (
                "lease_scope",
                "Non-interactive mutations outside the active lease require override and justification",
                format!(
                    "Out-of-lease mutation on entity '{}' requires confirmation; re-run with --override --justification \"<rationale>\"",
                    target_id
                ),
            )
        };

        let mut attribution = qdev_core::RejectionAttribution::new(rule).with_policy(policy);
        if let Some(ref l) = classification.active_lease {
            attribution = attribution.with_holder(&l.holder);
        }

        let mut details = serde_json::json!({
            "target_id": target_id,
            "is_cross_team": classification.is_cross_team,
            "is_out_of_lease": classification.is_out_of_lease,
        });
        if let Some(ref l) = classification.active_lease {
            details["active_lease"] = serde_json::json!({
                "story_id": l.story_id,
                "holder": l.holder,
                "worktree_path": l.worktree_path,
            });
        }

        let err = qdev_core::QdevError::policy_refusal("needs_confirmation", msg)
            .with_details(details)
            .with_attribution(attribution);
        let _ = output.emit_error(&err);
        return Err(ExitCode::PolicyRefusal);
    }

    // Interactive TTY flow
    let entity_title = opt_store
        .as_ref()
        .and_then(|s| s.get_entity(target_id).ok().flatten())
        .and_then(|e| e.title)
        .unwrap_or_else(|| {
            if let Ok((_, _, path)) = qdev_core::resolve_entity_file(
                root,
                None,
                target_id,
                Some(&annotated_config.config.storage),
            ) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(fm) = qdev_core::extract_frontmatter(&content) {
                        return fm
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                    }
                }
            }
            String::new()
        });

    if classification.is_cross_team {
        let owners_str = if classification.target_owners.is_empty() {
            "none".to_string()
        } else {
            classification.target_owners.join(", ")
        };
        let user_teams_str = if classification.user_teams.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                classification
                    .user_teams
                    .iter()
                    .map(|t| qdev_core::canonical_team_string(t))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        let team_to_add = if let Some(team) = classification.user_teams.first() {
            qdev_core::canonical_team_string(team)
        } else {
            author.id.clone()
        };

        eprintln!("⚠ Cross-team edit");
        eprintln!(
            "  Entity  {} \"{}\"   owners: {}",
            target_id, entity_title, owners_str
        );
        eprintln!("  You     {}{}", author.id, user_teams_str);
        eprintln!("  [1] Override with justification (logged as DEC cross_team_override)");
        eprintln!(
            "  [2] Add {} to owners (requires an existing owner's lease or override)",
            team_to_add
        );
        eprintln!("  [3] Abort");

        let choice = match prompt_input("") {
            Ok(c) => c,
            Err(_) => {
                println!("Aborted.");
                return Ok(GovernanceOutcome::Aborted);
            }
        };

        match choice.trim() {
            "1" => {
                let just = match prompt_input("Justification: ") {
                    Ok(j) => j,
                    Err(_) => {
                        let err = qdev_core::QdevError::policy_refusal(
                            "needs_justification",
                            "--override requires a non-empty --justification",
                        )
                        .with_attribution(
                            qdev_core::RejectionAttribution::new(
                                "Override requires non-empty justification",
                            )
                            .with_policy("justification_required"),
                        );
                        let _ = output.emit_error(&err);
                        return Err(ExitCode::PolicyRefusal);
                    }
                };
                let trimmed_just = just.trim();
                if trimmed_just.is_empty() {
                    let err = qdev_core::QdevError::policy_refusal(
                        "needs_justification",
                        "--override requires a non-empty --justification",
                    )
                    .with_attribution(
                        qdev_core::RejectionAttribution::new(
                            "Override requires non-empty justification",
                        )
                        .with_policy("justification_required"),
                    );
                    let _ = output.emit_error(&err);
                    return Err(ExitCode::PolicyRefusal);
                }

                Ok(GovernanceOutcome::Proceed {
                    justification: Some(trimmed_just.to_string()),
                    override_decision: Some((
                        "cross_team_override".to_string(),
                        trimmed_just.to_string(),
                    )),
                })
            }
            "2" => {
                let just = match prompt_input("Justification: ") {
                    Ok(j) => j,
                    Err(_) => {
                        let err = qdev_core::QdevError::policy_refusal(
                            "needs_justification",
                            "--override requires a non-empty --justification",
                        )
                        .with_attribution(
                            qdev_core::RejectionAttribution::new(
                                "Override requires non-empty justification",
                            )
                            .with_policy("justification_required"),
                        );
                        let _ = output.emit_error(&err);
                        return Err(ExitCode::PolicyRefusal);
                    }
                };
                let trimmed_just = just.trim();
                if trimmed_just.is_empty() {
                    let err = qdev_core::QdevError::policy_refusal(
                        "needs_justification",
                        "--override requires a non-empty --justification",
                    )
                    .with_attribution(
                        qdev_core::RejectionAttribution::new(
                            "Override requires non-empty justification",
                        )
                        .with_policy("justification_required"),
                    );
                    let _ = output.emit_error(&err);
                    return Err(ExitCode::PolicyRefusal);
                }

                if let Err(e) = qdev_core::add_team_to_entity_owners(
                    root,
                    Some(&annotated_config.config.storage),
                    target_id,
                    &team_to_add,
                    author,
                    *if_version,
                ) {
                    let _ = output.emit_error(&e);
                    return Err(e.exit_code());
                }

                if let Some(v) = *if_version {
                    *if_version = Some(v + 1);
                }

                Ok(GovernanceOutcome::Proceed {
                    justification: Some(trimmed_just.to_string()),
                    override_decision: Some((
                        "cross_team_override".to_string(),
                        trimmed_just.to_string(),
                    )),
                })
            }
            "3" => {
                println!("Aborted.");
                Ok(GovernanceOutcome::Aborted)
            }
            _ => {
                println!("Aborted.");
                Ok(GovernanceOutcome::Aborted)
            }
        }
    } else {
        eprintln!("⚠ Out-of-lease edit");
        eprintln!("  Entity  {} \"{}\"", target_id, entity_title);
        if let Some(ref l) = classification.active_lease {
            eprintln!("  Lease   {} (held in {})", l.story_id, l.worktree_path);
        }
        eprintln!("  [1] Override with justification (logged as DEC lease_override)");
        eprintln!("  [3] Abort");

        let choice = match prompt_input("") {
            Ok(c) => c,
            Err(_) => {
                println!("Aborted.");
                return Ok(GovernanceOutcome::Aborted);
            }
        };

        match choice.trim() {
            "1" => {
                let just = match prompt_input("Justification: ") {
                    Ok(j) => j,
                    Err(_) => {
                        let err = qdev_core::QdevError::policy_refusal(
                            "needs_justification",
                            "--override requires a non-empty --justification",
                        )
                        .with_attribution(
                            qdev_core::RejectionAttribution::new(
                                "Override requires non-empty justification",
                            )
                            .with_policy("justification_required"),
                        );
                        let _ = output.emit_error(&err);
                        return Err(ExitCode::PolicyRefusal);
                    }
                };
                let trimmed_just = just.trim();
                if trimmed_just.is_empty() {
                    let err = qdev_core::QdevError::policy_refusal(
                        "needs_justification",
                        "--override requires a non-empty --justification",
                    )
                    .with_attribution(
                        qdev_core::RejectionAttribution::new(
                            "Override requires non-empty justification",
                        )
                        .with_policy("justification_required"),
                    );
                    let _ = output.emit_error(&err);
                    return Err(ExitCode::PolicyRefusal);
                }

                Ok(GovernanceOutcome::Proceed {
                    justification: Some(trimmed_just.to_string()),
                    override_decision: Some((
                        "lease_override".to_string(),
                        trimmed_just.to_string(),
                    )),
                })
            }
            _ => {
                println!("Aborted.");
                Ok(GovernanceOutcome::Aborted)
            }
        }
    }
}

/// Argument parsing, id allocation, author resolution and output rendering — the write itself
/// belongs to `qdev_core::create_story`, which takes the advisory lock, validates the generated
/// frontmatter against the story schema, writes atomically and syncs the cache. Per AD-2 the CLI
/// does not assemble YAML or touch the filesystem itself.
fn handle_create_story(
    story_args: &cli::CreateStoryArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let epic_arg = story_args.epic.trim();

    let parsed = match epic_arg.parse::<qdev_core::Identifier>() {
        Ok(id) => id,
        Err(e) => {
            let err = QdevError::from(e);
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    };

    let epic_num = match parsed {
        qdev_core::Identifier::Epic { number } => number,
        _ => {
            let err = QdevError::usage_error(format!(
                "Argument must be an Epic ID (e.g. E12), got '{}'",
                epic_arg
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    };

    let root = qdev_core::find_workspace_root(current_dir);

    // Resolved before allocation so a misconfigured author type is refused without scanning or
    // writing anything. `resolve_author` is the single attribution path (AD-12): flags first,
    // then `QDEV_AUTHOR_TYPE`/`QDEV_AUTHOR_ID`, then config identity, then the git email.
    let author = match resolve_author(
        story_args.author_type.as_deref(),
        story_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Allocate against, and write into, the *configured* specs directory. Using the default
    // layout here while the sweep reads `storage.specs_dir` meant a non-default `[storage]`
    // silently wrote every created story somewhere no read command would ever look.
    let storage = &annotated_config.config.storage;

    // The cache is a union member of the in-use id set, not its source: an id belonging to a
    // hydrated entity is taken even if its file has since become unreadable. Opened only inside
    // an initialised workspace — `qdev create story` works in a bare directory, and opening the
    // store there would create a stray, schema-less `cache.sqlite`. `main` has already run
    // `ensure_cache` for every command in an initialised workspace, so a cache that reaches
    // here is a healthy one.
    // The cache is a *union member* of the in-use set, never a requirement: `create story` has
    // always worked in a bare directory, and a workspace whose cache cannot be opened is exactly
    // the workspace where refusing to create anything is least helpful. An unopenable cache
    // falls back to the filesystem half, which is the conservative answer — it can only miss
    // ids whose files are gone.
    let query_store = if ensure_query_workspace(&root).is_ok() {
        open_query_store(&root, annotated_config).ok()
    } else {
        None
    };
    let story_id = match qdev_core::allocate_next_story_id_in(
        &root,
        storage,
        epic_num,
        query_store.as_ref().map(|s| s as &dyn Store),
    ) {
        Ok(id) => id,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };
    // Released before `create_story` takes the advisory lock and writes: the allocator is a
    // reader, and holding a connection open past it serves nothing.
    drop(query_store);

    let create_opts = qdev_core::StoryCreateOptions {
        workspace_root: root,
        storage: Some(storage.clone()),
        story_id: story_id.to_string(),
        title: story_args.title.clone(),
        appetite: story_args.appetite.clone(),
        safety_class: story_args.safety_class.clone(),
        target_modules: story_args.module.clone(),
        owners: story_args.owner.clone(),
        author,
    };

    let res = match qdev_core::create_story(&create_opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(CreateStoryPayload {
            id: res.id.clone(),
            path: res.rel_path.clone(),
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit story envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        println!("Created story {} at {}", res.id, res.rel_path);
    }

    ExitCode::Success
}

#[allow(clippy::disallowed_methods)] // Relation write-gate intentionally reads the retained source row.
fn handle_update(
    update_args: &cli::UpdateArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let (entity_kind, entity_id) =
        match resolve_kind_and_id(&update_args.target, update_args.id.as_deref()) {
            Ok(resolved) => resolved,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };

    // Parse custom fields
    let mut custom_fields = Vec::new();
    for f in &update_args.field {
        if let Some((k, v)) = f.split_once('=') {
            let k_trimmed = k.trim();
            if k_trimmed.is_empty()
                || !k_trimmed
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
            {
                let err = QdevError::usage_error(format!(
                    "Invalid --field argument '{}': key must contain only alphanumeric, '_', or '-' characters",
                    f
                ));
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }
            let val: serde_yaml::Value = serde_yaml::from_str(v)
                .unwrap_or_else(|_| serde_yaml::Value::String(v.to_string()));
            custom_fields.push((k_trimmed.to_string(), val));
        } else {
            let err = QdevError::usage_error(format!(
                "Invalid --field argument '{}', expected KEY=VALUE",
                f
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    }

    // Reject modifications to managed fields via --field
    const MANAGED_FIELDS: &[&str] = &["id", "version", "updated_by", "created_by"];
    for (k, _) in &custom_fields {
        if MANAGED_FIELDS.iter().any(|m| m.eq_ignore_ascii_case(k)) {
            let err = QdevError::usage_error(format!(
                "Cannot modify managed frontmatter field '{}' via --field",
                k
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    }

    // A case-variant of a canonical key would be appended as a second, ignored frontmatter key
    // by the exact-case patcher.  It is a usage error, not a spelling we silently normalize.
    const CANONICAL_FRONTMATTER_FIELDS: &[&str] = &[
        "appetite",
        "assignments",
        "at",
        "base_version",
        "baseline_snapshot",
        "binds",
        "cause",
        "completed_at",
        "constraints",
        "commit_sha",
        "context",
        "control",
        "id",
        "created_by",
        "created_at",
        "decision",
        "decision_type",
        "dependency_version",
        "duration_ms",
        "epic_id",
        "evaluated_for_release",
        "evidence_path",
        "exit_code",
        "gate",
        "gate_id",
        "gates",
        "hazard",
        "iec62304_class",
        "introduced_by_story",
        "kind",
        "license",
        "metric_value",
        "metrics",
        "mitigations",
        "name",
        "origin_story_id",
        "output_hash",
        "owners",
        "personas",
        "phase",
        "prevents",
        "probability",
        "ran_at",
        "rationale",
        "relations",
        "release",
        "release_date",
        "release_version",
        "requirement_type",
        "resolution",
        "risk_level",
        "ruling",
        "safety_class",
        "safety_risk",
        "seq",
        "severity",
        "started_at",
        "status",
        "story_id",
        "subject_id",
        "summary",
        "target_module",
        "target_modules",
        "text",
        "title",
        "topic",
        "traces_to",
        "updated_by",
        "version",
        "vision",
    ];
    for (key, _) in &custom_fields {
        if let Some(&canonical) = CANONICAL_FRONTMATTER_FIELDS
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(key))
        {
            if key != canonical {
                let err = QdevError::usage_error(format!(
                    "Non-canonical --field key '{}'; use '{}'",
                    key, canonical
                ));
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
            }
        }
    }

    // Check conflicting arguments: flag vs --field
    if update_args.status.is_some()
        && custom_fields
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("status"))
    {
        let err = QdevError::usage_error(
            "Conflicting arguments: status specified via both --status and --field status=...",
        );
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    if update_args.title.is_some()
        && custom_fields
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("title"))
    {
        let err = QdevError::usage_error(
            "Conflicting arguments: title specified via both --title and --field title=...",
        );
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // `relations` replaces the whole map, so validate that final map before the generic writer
    // obtains its lock or can touch the file/cache/version.  Invalid YAML shapes remain the
    // generic writer's schema-validation responsibility; a well-formed relation map uses the
    // same proposed-graph gate as `relate`.
    if let Some((_, relations_value)) = custom_fields
        .iter()
        .rev()
        .find(|(key, _)| key == "relations")
    {
        if let Some(proposed_relations) = relation_map_from_yaml(relations_value) {
            let store = match open_query_store(&root, annotated_config) {
                Ok(store) => store,
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
            };
            let source = match store.get_entity(&entity_id) {
                Ok(Some(source)) => source,
                Ok(None) => {
                    let err =
                        QdevError::usage_error(format!("Source entity '{}' not found", entity_id));
                    let _ = output.emit_error(&err);
                    return ExitCode::UsageError;
                }
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
            };
            let proposed_source_kind = custom_fields
                .iter()
                .rev()
                .find(|(key, _)| key == "kind")
                .and_then(|(_, value)| value.as_str())
                .and_then(|kind| qdev_core::EntityKind::from_str_loose(kind).ok())
                .unwrap_or(source.kind);
            if let Err(e) = validate_proposed_relation_map(
                &store,
                &source,
                proposed_source_kind,
                &proposed_relations,
            ) {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        }
    }

    // Resolve active author attribution through the shared resolver — see `resolve_author`.
    let author = match resolve_author(
        update_args.author_type.as_deref(),
        update_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut if_version = update_args.if_version;
    let (_gov_just, override_decision) = match check_governance_gate(
        &root,
        &entity_id,
        entity_kind,
        &author,
        annotated_config,
        interactivity,
        cli,
        output,
        &mut if_version,
    ) {
        Ok(GovernanceOutcome::Proceed {
            justification,
            override_decision,
        }) => (justification, override_decision),
        Ok(GovernanceOutcome::Aborted) => return ExitCode::Success,
        Err(code) => return code,
    };

    // Resolve section file path if provided
    let section_file = update_args.file.as_ref().map(|f| {
        let p = std::path::Path::new(f);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            current_dir.join(p)
        }
    });

    let update_opts = qdev_core::EntityUpdateOptions {
        workspace_root: root.clone(),
        storage: Some(annotated_config.config.storage.clone()),
        entity_kind,
        entity_id: entity_id.clone(),
        status: update_args.status.clone(),
        title: update_args.title.clone(),
        custom_fields,
        section: update_args.section.clone(),
        section_file,
        if_version,
        author: author.clone(),
    };

    let res = match qdev_core::apply_entity_update(&update_opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if let Some((dec_type, dec_just)) = override_decision {
        if let Err(e) = qdev_core::create_governance_override_decision(
            &root,
            Some(&annotated_config.config.storage),
            &entity_id,
            &dec_type,
            &dec_just,
            &author,
            None,
        ) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(res.updated_frontmatter);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit update envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        println!(
            "Updated {} {} (version {}) at {}",
            res.kind, res.id, res.new_version, res.rel_path
        );
    }

    ExitCode::Success
}

/// Opens the SQLite cache read-only for `get`/`list`, which never write.
///
/// Callers must check `ensure_query_workspace` first: `SqliteStore::open` creates the cache
/// file (and its parent directory) if missing, so calling this outside an initialized
/// workspace would otherwise silently create an empty, schema-less cache file.
pub(crate) fn open_query_store(
    root: &std::path::Path,
    annotated_config: &qdev_core::AnnotatedConfig,
) -> Result<qdev_core::SqliteStore, QdevError> {
    let cache_db_path = root
        .join(&annotated_config.config.storage.cache_dir)
        .join("cache.sqlite");
    qdev_core::SqliteStore::open(&cache_db_path)
}

/// Verifies `root` is an initialized qdev workspace before `get`/`list` touch the cache.
/// Without this, opening the cache store directly (bypassing the boot-time `ensure_cache`
/// hydration path) in an uninitialized directory would create a stray empty `cache.sqlite`
/// and fail with a raw `sqlite_error: no such table` instead of a clean usage error.
fn ensure_query_workspace(root: &std::path::Path) -> Result<(), QdevError> {
    if root.join("qdev.toml").is_file() {
        Ok(())
    } else {
        Err(QdevError::usage_error(
            "Not a qdev workspace (no qdev.toml found); run `qdev init` first",
        ))
    }
}

fn handle_get(
    get_args: &cli::GetArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let (kind_hint, entity_id) = match resolve_kind_and_id(&get_args.target, get_args.id.as_deref())
    {
        Ok(resolved) => resolved,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Validate --expand values: only "scratch" and "evidence" change behavior; "relations"/"constraints" are
    // accepted as no-ops since the default projection already includes them.
    let mut expand_scratch = false;
    let mut expand_evidence = false;
    for value in &get_args.expand {
        match value.trim() {
            "scratch" => expand_scratch = true,
            "evidence" => expand_evidence = true,
            "relations" | "constraints" | "" => {}
            other => {
                let err = QdevError::usage_error(format!(
                    "Unknown --expand value '{}', expected one of: relations, constraints, scratch, evidence",
                    other
                ));
                let _ = output.emit_error(&err);
                return ExitCode::UsageError;
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

    let query_opts = qdev_core::QueryOptions {
        expand_scratch,
        expand_evidence,
    };

    let result = match qdev_core::query_entity(&store, kind_hint, &entity_id, &query_opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let emit_res = match result {
            qdev_core::GetResult::Entity(projection) => {
                let envelope = JsonEnvelope::new(*projection);
                output.emit_envelope(&envelope)
            }
            qdev_core::GetResult::Constraint(constraint) => {
                let envelope = JsonEnvelope::new(constraint);
                output.emit_envelope(&envelope)
            }
        };
        if let Err(e) = emit_res {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit get envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = match &result {
            qdev_core::GetResult::Entity(projection) => render_get_entity_text(projection),
            qdev_core::GetResult::Constraint(constraint) => render_get_constraint_text(constraint),
        };
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit get output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

#[derive(Serialize)]
struct ListPayload {
    kind: String,
    items: Vec<qdev_core::ListEntryProjection>,
}

fn handle_list(
    list_args: &cli::ListArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let kind = match qdev_core::EntityKind::from_str_loose(&list_args.kind) {
        Ok(k) => k,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if let Err(e) = reject_empty_filter_values(&[
        ("--epic", list_args.epic.as_deref()),
        ("--status", list_args.status.as_deref()),
        ("--owner", list_args.owner.as_deref()),
        ("--module", list_args.module.as_deref()),
        ("--subject", list_args.subject.as_deref()),
        ("--type", list_args.r#type.as_deref()),
        ("--risk", list_args.risk.as_deref()),
    ]) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    // Resolve "--owner me" against the active identity the same way `qdev update`'s
    // attribution does: config.identity.developer_id, falling back to resolve_git_email.
    let owner = match list_args.owner.as_deref() {
        Some("me") => {
            let identity = if !annotated_config.config.identity.developer_id.is_empty() {
                Some(annotated_config.config.identity.developer_id.clone())
            } else {
                qdev_core::resolve_git_email(Some(&root))
            };
            match identity {
                Some(id) if !id.trim().is_empty() => Some(id),
                _ => {
                    let err = QdevError::usage_error(
                        "Cannot resolve '--owner me': no active identity is configured and no git user.email fallback was found",
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::UsageError;
                }
            }
        }
        Some(other) => Some(other.to_string()),
        None => None,
    };

    let mut query_opts = qdev_core::ListQueryOptions::new(kind);
    query_opts.epic_id = list_args.epic.clone();
    query_opts.status = list_args.status.clone();
    query_opts.owner = owner;
    query_opts.module = list_args.module.clone();
    query_opts.sprint = list_args.sprint;
    query_opts.subject = list_args.subject.clone();
    query_opts.decision_type = list_args.r#type.clone();
    query_opts.safety_risk = list_args.risk.clone();

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let rows = match qdev_core::query_list(&store, &query_opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(ListPayload {
            kind: kind.as_str().to_string(),
            items: rows,
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit list envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_list_text(&rows);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit list output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

fn handle_graph(
    graph_args: &cli::GraphArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    if graph_args.dot && cli.json {
        let err = QdevError::usage_error("Cannot combine '--dot' and '--json' output flags")
            .with_details(serde_json::json!({ "flags": ["--dot", "--json"] }));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    if !graph_args.dot && !cli.json {
        let err =
            QdevError::usage_error("qdev graph requires either '--dot' or '--json' output format")
                .with_details(serde_json::json!({ "flag": "output_format" }));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    let root = qdev_core::find_workspace_root(current_dir);

    if let Err(e) = reject_empty_filter_values(&[
        ("--epic", graph_args.epic.as_deref()),
        ("--sprint", graph_args.sprint.as_deref()),
    ]) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    let normalized_sprint = match graph_args.sprint.as_deref() {
        Some(s) => match qdev_core::normalize_sprint_id(s) {
            Ok(num) => Some(num),
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        },
        None => None,
    };

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let options = qdev_core::StoryGraphOptions {
        epic: graph_args.epic.clone(),
        sprint: normalized_sprint,
        highlight_critical_path: graph_args.highlight_critical_path,
    };

    let payload = match qdev_core::build_story_graph(
        &store,
        &root,
        Some(&annotated_config.config.storage),
        &options,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(payload);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit graph JSON envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let dot = qdev_core::render_graph_dot(&payload);
        if let Err(e) = output.emit_text(&dot) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit graph output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

/// Resolves the active author attribution for every command that records one: explicit CLI
/// flags, then `QDEV_AUTHOR_TYPE`/`QDEV_AUTHOR_ID`, then config identity, then the git email.
fn resolve_author(
    author_type: Option<&str>,
    author_id: Option<&str>,
    annotated_config: &qdev_core::AnnotatedConfig,
    root: &std::path::Path,
) -> Result<qdev_core::Author, QdevError> {
    qdev_core::resolve_author(author_type, author_id, annotated_config, root)
}

#[derive(Serialize)]
struct RelatePayload {
    id: String,
    relation: String,
    target_id: String,
    version: u64,
    changed: bool,
    relations: serde_json::Value,
}

/// Refuses a relation name architecture.md §8 does not define, as a usage error naming the
/// valid ones — the same class as an unrecognised `--author-type` or entity kind. Both `relate`
/// and `unrelate` call this before every other check, so a typo can neither be reported as a
/// disallowed kind pair (`relate` could not tell the two empty pair lists apart) nor be absorbed
/// by the idempotent no-op (`unrelate` exited 0 while the real edge survived).
///
/// A *known* relation whose source and target kinds are not an allowed pair — `verifies`, which
/// has no pairs until Epic 3 models gates, included — is a different refusal and keeps its own
/// `invalid_relation_kind` (exit 1). `qdev_core::apply_relation_change` deliberately keeps
/// accepting any name, because `qdev validate --fix-ids` rewrites the names it finds in files.
fn validate_relation_name(relation: &str) -> Result<(), QdevError> {
    if qdev_core::is_known_relation(relation) {
        return Ok(());
    }
    // The list in the message comes from `relation_names()`, never from a literal: the arg help
    // strings in `cli.rs` do spell the eight out for `--help` readability, and those are the one
    // place a ninth relation would have to be added by hand — a `#[value_parser]` would fix that
    // too, at the cost of clap owning the error shape this envelope depends on.
    Err(QdevError::usage_error(format!(
        "Unknown relation '{}', must be one of: {}",
        relation,
        qdev_core::relation_names().collect::<Vec<_>>().join(", ")
    )))
}

/// Converts a syntactically usable YAML relation map for the command gate.  Invalid shapes are
/// deliberately returned as `None`: the generic update writer retains ownership of its existing
/// schema-validation error contract for arbitrary `--field` values.
fn relation_map_from_yaml(value: &serde_yaml::Value) -> Option<BTreeMap<String, Vec<String>>> {
    let serde_yaml::Value::Mapping(map) = value else {
        return None;
    };
    let mut relations = BTreeMap::new();
    for (relation, targets) in map {
        let relation = relation.as_str()?.to_string();
        let serde_yaml::Value::Sequence(targets) = targets else {
            return None;
        };
        let targets = targets
            .iter()
            .map(|target| target.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()?;
        relations.insert(relation, targets);
    }
    Some(relations)
}

/// Reads the cache into the pure core proposed-graph gate.  Keeping store access here leaves
/// the gate reusable for future callers and gives `relate` and whole-map replacement one rule.
#[allow(clippy::disallowed_methods)] // Relation write-gate must inspect the complete retained graph.
fn validate_proposed_relation_map(
    store: &qdev_core::SqliteStore,
    source: &qdev_core::EntityRecord,
    source_kind: qdev_core::EntityKind,
    proposed_relations: &BTreeMap<String, Vec<String>>,
) -> Result<(), QdevError> {
    let entities = store
        .list_entities(&qdev_core::EntityFilter::default())?
        .into_iter()
        .map(|entity| (entity.id, entity.kind))
        .collect::<Vec<_>>();
    let relations = store
        .list_relations()?
        .into_iter()
        .map(|relation| (relation.source_id, relation.relation, relation.target_id))
        .collect::<Vec<_>>();
    qdev_core::validate_proposed_relation_map(
        &source.id,
        source_kind,
        proposed_relations,
        &entities,
        &relations,
    )
}

#[allow(clippy::disallowed_methods)] // Relation write-gate intentionally reads the retained source row.
fn handle_relate(
    relate_args: &cli::RelateArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // Before any other check: an unknown relation name is a usage error, not a kind-pair one.
    if let Err(err) = validate_relation_name(&relate_args.relation) {
        let _ = output.emit_error(&err);
        return err.exit_code();
    }

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Resolve the source before building the complete proposed map for the shared relation gate.
    let source = match store.get_entity(&relate_args.source_id) {
        Ok(Some(e)) => e,
        Ok(None) => {
            let err = QdevError::usage_error(format!(
                "Source entity '{}' not found",
                relate_args.source_id
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let target_id = relate_args.target_id.clone();
    let mut proposed_relations = store.get_relations_for_source(&source.id).map(|rows| {
        let mut map = BTreeMap::<String, Vec<String>>::new();
        for row in rows {
            map.entry(row.relation).or_default().push(row.target_id);
        }
        map
    });
    let proposed_relations = match proposed_relations.as_mut() {
        Ok(relations) => {
            let targets = relations.entry(relate_args.relation.clone()).or_default();
            if !targets.iter().any(|target| target == &target_id) {
                targets.push(target_id.clone());
            }
            relations
        }
        Err(e) => {
            let _ = output.emit_error(e);
            return e.exit_code();
        }
    };
    if let Err(e) = validate_proposed_relation_map(&store, &source, source.kind, proposed_relations)
    {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    let author = match resolve_author(
        relate_args.author_type.as_deref(),
        relate_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut if_version = relate_args.if_version;
    let (_gov_just, override_decision) = match check_governance_gate(
        &root,
        &source.id,
        Some(source.kind),
        &author,
        annotated_config,
        interactivity,
        cli,
        output,
        &mut if_version,
    ) {
        Ok(GovernanceOutcome::Proceed {
            justification,
            override_decision,
        }) => (justification, override_decision),
        Ok(GovernanceOutcome::Aborted) => return ExitCode::Success,
        Err(code) => return code,
    };

    let opts = qdev_core::RelationChangeOptions {
        workspace_root: root.clone(),
        storage: Some(annotated_config.config.storage.clone()),
        entity_kind: None,
        // Both ids as the cache stores them, which is what the kind-pair and cycle pre-checks
        // above ran against. `get_entity` matches `entities.id` exactly, so these equal the
        // strings the user typed today; using the resolved values keeps the checks and the write
        // addressing the same two entities if lookup ever becomes more permissive.
        entity_id: source.id.clone(),
        relation: relate_args.relation.clone(),
        target_id: target_id.clone(),
        add: true,
        if_version,
        author: author.clone(),
    };

    let res = match qdev_core::apply_relation_change(&opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if let Some((dec_type, dec_just)) = override_decision {
        if let Err(e) = qdev_core::create_governance_override_decision(
            &root,
            Some(&annotated_config.config.storage),
            &source.id,
            &dec_type,
            &dec_just,
            &author,
            None,
        ) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(RelatePayload {
            id: res.id.clone(),
            relation: relate_args.relation.clone(),
            target_id: relate_args.target_id.clone(),
            version: res.new_version,
            changed: res.changed,
            relations: res.relations,
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit relate envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else if res.changed {
        println!(
            "Related {} --[{}]--> {} (version {}) at {}",
            res.id, relate_args.relation, relate_args.target_id, res.new_version, res.rel_path
        );
    } else {
        println!(
            "No-op: {} already has a '{}' relation to '{}'",
            res.id, relate_args.relation, relate_args.target_id
        );
    }

    ExitCode::Success
}

fn handle_unrelate(
    unrelate_args: &cli::UnrelateArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // Before any other check, and before the write path can report the idempotent no-op that
    // used to swallow this: an unknown relation name is a usage error naming the valid ones.
    if let Err(err) = validate_relation_name(&unrelate_args.relation) {
        let _ = output.emit_error(&err);
        return err.exit_code();
    }

    let author = match resolve_author(
        unrelate_args.author_type.as_deref(),
        unrelate_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut if_version = unrelate_args.if_version;
    let (_gov_just, override_decision) = match check_governance_gate(
        &root,
        &unrelate_args.source_id,
        None,
        &author,
        annotated_config,
        interactivity,
        cli,
        output,
        &mut if_version,
    ) {
        Ok(GovernanceOutcome::Proceed {
            justification,
            override_decision,
        }) => (justification, override_decision),
        Ok(GovernanceOutcome::Aborted) => return ExitCode::Success,
        Err(code) => return code,
    };

    let opts = qdev_core::RelationChangeOptions {
        workspace_root: root.clone(),
        storage: Some(annotated_config.config.storage.clone()),
        entity_kind: None,
        entity_id: unrelate_args.source_id.clone(),
        relation: unrelate_args.relation.clone(),
        target_id: unrelate_args.target_id.clone(),
        add: false,
        if_version,
        author: author.clone(),
    };

    // Unrelating an absent entry, with a relation name that exists, is an idempotent no-op
    // (exit 0), matching `relate`/`unrelate`'s I/O contract: `apply_relation_change` returns
    // `changed: false` without writing. It still compares `--if-version` first, so the fence
    // holds on that path too (exit 5).
    let res = match qdev_core::apply_relation_change(&opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if let Some((dec_type, dec_just)) = override_decision {
        if let Err(e) = qdev_core::create_governance_override_decision(
            &root,
            Some(&annotated_config.config.storage),
            &unrelate_args.source_id,
            &dec_type,
            &dec_just,
            &author,
            None,
        ) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(RelatePayload {
            id: res.id.clone(),
            relation: unrelate_args.relation.clone(),
            target_id: unrelate_args.target_id.clone(),
            version: res.new_version,
            changed: res.changed,
            relations: res.relations,
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit unrelate envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else if res.changed {
        println!(
            "Unrelated {} --[{}]--> {} (version {}) at {}",
            res.id, unrelate_args.relation, unrelate_args.target_id, res.new_version, res.rel_path
        );
    } else {
        println!(
            "No-op: {} has no '{}' relation to '{}'",
            res.id, unrelate_args.relation, unrelate_args.target_id
        );
    }

    ExitCode::Success
}

/// Renders a single entity projection as a compact `key: value` text block.
fn render_get_entity_text(p: &qdev_core::EntityProjection) -> String {
    let mut out = String::new();
    out.push_str(&format!("id: {}\n", p.id));
    out.push_str(&format!("kind: {}\n", p.kind));
    if let Some(ref epic_id) = p.epic_id {
        out.push_str(&format!("epic_id: {}\n", epic_id));
    }
    if let Some(ref title) = p.title {
        out.push_str(&format!("title: {}\n", title));
    }
    if let Some(ref status) = p.status {
        out.push_str(&format!("status: {}\n", status));
    }
    out.push_str(&format!("blocked: {}\n", p.blocked));
    if p.stale {
        out.push_str("stale: true\n");
    }
    if let Some(ref appetite) = p.appetite {
        out.push_str(&format!("appetite: {}\n", appetite));
    }
    if let Some(ref safety_class) = p.safety_class {
        out.push_str(&format!("safety_class: {}\n", safety_class));
    }
    out.push_str(&format!("owners: {}\n", p.owners.join(", ")));
    if let Some(ref modules) = p.target_modules {
        out.push_str(&format!("target_modules: {}\n", modules.join(", ")));
    }
    out.push_str(&format!("version: {}\n", p.version));

    out.push_str("constraints:\n");
    if p.constraints.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for c in &p.constraints {
            match &c.inherited_from {
                Some(from) => out.push_str(&format!(
                    "  {} [{}] {} (inherited from {})\n",
                    c.id, c.kind, c.text, from
                )),
                None => out.push_str(&format!("  {} [{}] {}\n", c.id, c.kind, c.text)),
            }
        }
    }

    out.push_str("relations:\n");
    if p.relations.is_empty() {
        out.push_str("  (none)\n");
    } else {
        for (relation, targets) in &p.relations {
            out.push_str(&format!("  {}: {}\n", relation, targets.join(", ")));
        }
    }

    if let Some(ref scratch) = p.scratch {
        out.push_str("scratch:\n");
        if scratch.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for entry in scratch {
                let kind = entry.kind.as_deref().unwrap_or("note");
                let author = match (&entry.author_type, &entry.author_id) {
                    (Some(t), Some(id)) => format!("{}:{}", t, id),
                    _ => "unknown".to_string(),
                };
                // Collapse embedded newlines so a multi-line note can't break this block's
                // one-line-per-entry alignment (JSON mode preserves the text verbatim).
                let text = entry.text.as_deref().unwrap_or("").replace('\n', " ");
                out.push_str(&format!(
                    "  [{}] {} ({}, {}) {}\n",
                    entry.seq, entry.at, author, kind, text
                ));
            }
        }
    }

    if let Some(ref assignments) = p.assignments {
        out.push_str("assignments:\n");
        if assignments.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for a in assignments {
                let carried = match a.carried_from {
                    Some(from) => format!(" (carried from {})", from),
                    None => String::new(),
                };
                out.push_str(&format!(
                    "  {}{} assigned: {}\n",
                    a.story, carried, a.assigned_at
                ));
            }
        }
    }

    if let Some(ref counts) = p.status_counts {
        out.push_str("status_counts:\n");
        if counts.is_empty() {
            out.push_str("  (none)\n");
        } else {
            let mut sorted_counts: Vec<_> = counts.iter().collect();
            sorted_counts.sort_by_key(|(k, _)| *k);
            for (status, count) in sorted_counts {
                out.push_str(&format!("  {}: {}\n", status, count));
            }
        }
    }

    if let Some(ref evidence) = p.evidence {
        out.push_str("evidence:\n");
        if evidence.is_empty() {
            out.push_str("  (none)\n");
        } else {
            for e in evidence {
                let status_upper = e.status.to_uppercase();
                let clean_summary = e
                    .summary
                    .as_deref()
                    .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                let duration_str = e
                    .duration_ms
                    .map(qdev_core::gate::format_duration)
                    .unwrap_or_else(|| "0 ms".to_string());
                let summary_disp = if clean_summary.is_empty() {
                    "-"
                } else {
                    &clean_summary
                };
                out.push_str(&format!(
                    "  [{}] {} | {} | {} | {} | evidence {}\n",
                    status_upper, e.gate, summary_disp, e.commit, duration_str, e.evidence_path
                ));
            }
        }
    }

    out
}

/// Renders a bare constraint lookup (`qdev get E12S4/NG-1`) as a compact `key: value` block.
fn render_get_constraint_text(c: &qdev_core::ConstraintRecord) -> String {
    format!(
        "id: {}\nowner: {}\nkind: {}\ntext: {}\n",
        c.id, c.owner_id, c.kind, c.text
    )
}

/// Renders `list` rows as a fixed-width column table, truncating long fields to
/// terminal-friendly widths. No table-formatting crate is used per spec.
fn render_list_text(rows: &[qdev_core::ListEntryProjection]) -> String {
    const ID_WIDTH: usize = 12;
    const TITLE_WIDTH: usize = 32;
    const STATUS_WIDTH: usize = 12;

    let mut out = String::new();
    out.push_str(&format!(
        "{:<id_w$}  {:<title_w$}  {:<status_w$}  OWNERS\n",
        "ID",
        "TITLE",
        "STATUS",
        id_w = ID_WIDTH,
        title_w = TITLE_WIDTH,
        status_w = STATUS_WIDTH
    ));

    if rows.is_empty() {
        out.push_str("(no matching entities)\n");
        return out;
    }

    for row in rows {
        // The `id` column is never truncated: it's the one column whose entire purpose is
        // unique identification, so an ellipsis here could make two different ids
        // indistinguishable. A row with a longer-than-usual id just widens that row's column.
        let title = truncate_field(row.title.as_deref().unwrap_or(""), TITLE_WIDTH);
        let status = truncate_field(row.status.as_deref().unwrap_or(""), STATUS_WIDTH);
        let owners = row.owners.join(", ");
        out.push_str(&format!(
            "{:<id_w$}  {:<title_w$}  {:<status_w$}  {}\n",
            row.id,
            title,
            status,
            owners,
            id_w = ID_WIDTH,
            title_w = TITLE_WIDTH,
            status_w = STATUS_WIDTH
        ));
    }

    out
}

#[derive(Serialize)]
struct ValidatePayload {
    findings: Vec<qdev_core::FindingRecord>,
}

/// Renders findings as a flat text table, one line per finding, sorted by path then code, so
/// output is deterministic across runs against the same cache state.
fn render_validate_text(findings: &[qdev_core::FindingRecord]) -> String {
    if findings.is_empty() {
        return "No findings.\n".to_string();
    }
    let mut out = String::new();
    for f in findings {
        out.push_str(&format!(
            "[{}] {} {}{}\n",
            f.severity,
            f.code,
            f.path,
            f.message
                .as_deref()
                .map(|m| format!(" -- {}", m.replace(['\n', '\r'], " ")))
                .unwrap_or_default()
        ));
    }
    out
}

fn handle_validate(
    validate_args: &cli::ValidateArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    if validate_args.fix_ids {
        if validate_args.changed {
            let err = QdevError::usage_error(
                "'--changed' is not supported with '--fix-ids': the guided renumber always \
                 operates workspace-wide",
            );
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
        return handle_fix_ids(
            &root,
            annotated_config,
            interactivity,
            validate_args.yes,
            cli,
            output,
        );
    }

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let mut findings = match qdev_core::run_validation(&store, &root, &annotated_config.config) {
        Ok(f) => f,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if validate_args.changed {
        let changed_paths = match qdev_core::git_changed_files(
            &root,
            &annotated_config.config.git.integration_branch,
        ) {
            Ok(p) => p,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };
        findings = qdev_core::filter_by_changed(findings, &changed_paths);
    }

    qdev_core::sort_findings(&mut findings);

    let exit_code = if qdev_core::has_error_finding(&findings) {
        ExitCode::LogicalFailure
    } else {
        ExitCode::Success
    };

    if cli.json {
        let envelope = JsonEnvelope::new(ValidatePayload { findings });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit validate envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_validate_text(&findings);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit validate output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    exit_code
}

/// Renders a `SweepSummary` as a one-line human-readable count summary.
fn render_sync_text(summary: &qdev_core::SweepSummary) -> String {
    format!(
        "parsed={}, unchanged={}, retained={}, hashed={}, purged={}, findings={}\n",
        summary.parsed,
        summary.unchanged,
        summary.retained,
        summary.hashed,
        summary.purged,
        summary.findings
    )
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
fn render_doctor_text(
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

fn handle_sync(
    sync_args: &cli::SyncArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    boot_summary: Option<&qdev_core::SweepSummary>,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let storage = &annotated_config.config.storage;

    // `sweep_workspace` takes the advisory write lock itself, so the plain branch must not (it
    // would deadlock). `reset_and_rebuild` does not — `ensure_cache` calls it while already
    // holding the lock — so this, its only other caller, takes it here: a rebuild drops and
    // repopulates every table, and a concurrent `qdev update` landing in the middle of that
    // would be rebuilt away or read mid-write.
    let summary = if sync_args.rebuild {
        let lock_path = root.join(&storage.cache_dir).join("write.lock");
        match qdev_core::acquire_write_lock(&lock_path, std::time::Duration::from_millis(5000)) {
            Ok(_guard) => store.reset_and_rebuild(&root, storage),
            Err(e) => Err(e),
        }
    } else if let Some(boot) = boot_summary {
        // The boot-time sweep already ran against this workspace in this process. Sweeping a
        // second time here would find everything settled and report `parsed: 0`, making sync
        // look like a no-op in exactly the invocation where real hydration happened (at
        // boot). Report the boot pass instead.
        Ok(*boot)
    } else {
        store.sweep_workspace(&root, storage)
    };

    let summary = match summary {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if cli.json {
        let envelope = JsonEnvelope::new(summary);
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sync envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        let text = render_sync_text(&summary);
        if let Err(e) = output.emit_text(&text) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit sync output: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    ExitCode::Success
}

#[derive(Serialize)]
struct DoctorPayload {
    sections: Vec<qdev_core::DoctorSectionReport>,
}

fn handle_doctor(
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

/// Guided duplicate-planning-id renumber for `qdev validate --fix-ids`. Gated exactly like
/// `handle_init`'s non-interactive flag requirements: refuses with no writes unless the terminal
/// is interactive or `--yes` was passed.
/// One file the renumber is going to rewrite, decided (and confirmed) before any write starts.
struct PlannedRenumber {
    path: String,
    old_id: String,
    new_id: String,
}

fn handle_fix_ids(
    root: &std::path::Path,
    annotated_config: &qdev_core::AnnotatedConfig,
    interactivity: Interactivity,
    yes: bool,
    cli: &Cli,
    output: &OutputEmitter,
) -> ExitCode {
    if !interactivity.is_interactive() && !yes {
        let err = QdevError::policy_refusal(
            "needs_confirmation",
            "'--fix-ids' requires an interactive terminal or '--yes'; refusing without writing",
        )
        .with_details(serde_json::json!({ "flag": "--yes" }))
        .with_attribution(
            qdev_core::RejectionAttribution::new(
                "'--fix-ids' requires confirmation in non-interactive mode",
            )
            .with_policy("fix_ids_confirmation"),
        );
        let _ = output.emit_error(&err);
        return ExitCode::PolicyRefusal;
    }

    let storage = &annotated_config.config.storage;
    let scan = match qdev_core::scan_duplicate_planning_ids(root, storage) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if scan.groups.is_empty() {
        let store = match open_query_store(root, annotated_config) {
            Ok(s) => s,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };
        let mut findings = match qdev_core::run_validation(&store, root, &annotated_config.config) {
            Ok(f) => f,
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };
        qdev_core::sort_findings(&mut findings);

        if cli.json {
            let envelope = JsonEnvelope::new(FixIdsPayload {
                renumbered: Vec::new(),
                skipped: Vec::new(),
                error: None,
                findings: findings.clone(),
            });
            if let Err(e) = output.emit_envelope(&envelope) {
                let err = QdevError::infrastructure_failure(
                    "io_error",
                    format!("Failed to emit fix-ids envelope: {}", e),
                );
                let _ = output.emit_error(&err);
                return ExitCode::InfrastructureFailure;
            }
        } else {
            let _ = output.emit_text("No duplicate planning ids found.\n");
            if !findings.is_empty() {
                let _ = output.emit_text(&render_validate_text(&findings));
            }
        }

        return if qdev_core::has_error_finding(&findings) {
            ExitCode::LogicalFailure
        } else {
            ExitCode::Success
        };
    }

    let author = match resolve_author(None, None, annotated_config, root) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let store = match open_query_store(root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // The in-use id set comes from `ids_in_use_from_scan` — the one authority, the same
    // function `qdev create story`'s allocator asks — rather than from a seeding recipe kept
    // here. Allocating from the on-disk scan alone can still hand out an id that belongs to a
    // hydrated entity whose file has become unreadable, minting a fresh duplicate while fixing
    // one; the two allocators having their own recipes is what let them diverge.
    let mut used_ids = match qdev_core::ids_in_use_from_scan(&scan, Some(&store)) {
        Ok(ids) => ids,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let all_relations = match store.list_relations() {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // --- Plan phase: allocate ids and collect confirmations, before taking the write lock. ---
    //
    // The prompt below waits on a human. Holding the advisory lock across that wait would make
    // every concurrent `qdev update` / `relate` / `sync` in the workspace fail with a 5s
    // `lock_timeout` for as long as the prompt is unanswered.
    let mut plan: Vec<PlannedRenumber> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    // Files that keep a duplicated id and must be re-parsed once the renumber is done. The
    // `entities.id` primary key means only one of the colliding files ever owned the cache row;
    // if that was the file being renumbered, the keeper's id would simply disappear from the
    // cache, because its own content is unchanged and an incremental sweep would skip it.
    let mut keepers_to_rehydrate: Vec<String> = Vec::new();

    for (old_id, paths) in &scan.groups {
        // The keeper is chosen by sort order, which says nothing about whether it satisfies the
        // identity rule. An off-convention *keeper* would otherwise escape the refusal
        // altogether: the group's other files get renumbered, the run reports a successful
        // repair, and the id is left owned by a file no writer can resolve — the exact outcome
        // Option A exists to forbid. So the keeper is checked first, and a group whose keeper is
        // off-convention is refused whole.
        if let Some(keeper) = paths.first() {
            match off_convention_directory_target(root, storage, keeper, old_id) {
                Ok(Some(target)) => {
                    eprintln!(
                        "Skipping the '{}' group: its keeper '{}' is not in its kind directory, \
                         so renumbering the others would leave '{}' owned by a file no writer \
                         can resolve. Move it to '{}' first.",
                        old_id, keeper, old_id, target.expected_dir_path
                    );
                    for path in paths.iter().skip(1) {
                        skipped.push(path.clone());
                    }
                    skipped.push(keeper.clone());
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    if !cli.json {
                        eprintln!("Skipping {}: {}", keeper, e);
                    }
                    for path in paths.iter().skip(1) {
                        skipped.push(path.clone());
                    }
                    skipped.push(keeper.clone());
                    continue;
                }
            }

            let keeper_filename = match keeper.replace('\\', "/").rsplit_once('/') {
                Some((_, name)) => name.to_string(),
                None => keeper.clone(),
            };
            if !qdev_core::filename_carries_id(&keeper_filename, old_id) {
                eprintln!(
                    "Skipping the '{}' group: its keeper '{}' does not carry that id in its filename, \
                     so renumbering the others would leave '{}' owned by a file no writer \
                     can resolve. Rename it first.",
                    old_id, keeper, old_id
                );
                for path in paths.iter().skip(1) {
                    skipped.push(path.clone());
                }
                skipped.push(keeper.clone());
                continue;
            }
        }

        // The first (lexicographically sorted) path keeps the id; every other file declaring it
        // is offered a renumber.
        for path in paths.iter().skip(1) {
            let old_identifier: qdev_core::Identifier = match old_id.parse() {
                Ok(id) => id,
                Err(_) => {
                    // Reachable since the duplicate scan widened to `state_dir`: sprint,
                    // deferred-work, decision, release and SOUP ids are not sequentially
                    // renumberable, so a collision among them is reported and skipped rather
                    // than repaired. Says so, like the sibling branch below — an unexplained
                    // entry in `skipped` reads as a decline.
                    if !cli.json {
                        eprintln!(
                            "Skipping {}: '{}' is not a renumberable identifier; \
                             resolve this duplicate by hand",
                            path, old_id
                        );
                    }
                    skipped.push(path.clone());
                    continue;
                }
            };

            let new_identifier = match qdev_core::next_available_id(&old_identifier, &used_ids) {
                Ok(id) => id,
                Err(e) => {
                    // Recorded as skipped rather than emitted here: in JSON mode `emit_error`
                    // writes a second document to stdout, which would leave the report of what
                    // *was* written unparseable.
                    if !cli.json {
                        eprintln!("Skipping {}: {}", path, e);
                    }
                    skipped.push(path.clone());
                    continue;
                }
            };
            let new_id = new_identifier.to_string();

            // Option A (Decision 2026-09-10, Simon): a duplicate whose file is not in its kind's
            // directory is refused, not repaired. Renumbering it in place would report a repair
            // it did not achieve — the entity would still be unresolvable by every writer — and
            // moving a user's file across directories is not a decision the tool takes silently.
            // The expected path is named here, as the `entity_file_off_convention` warning on the
            // same file already names its destination.
            match off_convention_directory_target(root, storage, path, &new_id) {
                Ok(Some(expected)) => {
                    // On stderr in both modes, unlike the sibling skips above: the expected path
                    // is the actionable half of this refusal, and stderr is not the JSON
                    // document, so naming it cannot make stdout unparseable.
                    eprintln!(
                        "Skipping {}: repairing it would write '{}', which is outside the \
                         directory every writer resolves entities in; move the file to '{}' \
                         and re-run",
                        path, expected.in_place, expected.expected_dir_path
                    );
                    skipped.push(path.clone());
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    if !cli.json {
                        eprintln!("Skipping {}: {}", path, e);
                    }
                    skipped.push(path.clone());
                    continue;
                }
            }

            // Pre-flight relation sources targeting `old_id`: do not rewrite an entity's id
            // on disk if redirecting incoming edges targeting that id will fail. An unresolvable
            // relation source causes the candidate renumber to be refused and skipped before
            // touching disk.
            let incoming_relations = all_relations.iter().filter(|r| r.target_id == *old_id);
            let mut unresolvable_source: Option<(String, QdevError)> = None;
            for rel in incoming_relations {
                if let Err(e) =
                    qdev_core::resolve_entity_file(root, None, &rel.source_id, Some(storage))
                {
                    unresolvable_source = Some((rel.source_id.clone(), e));
                    break;
                }
            }
            if let Some((source_id, err)) = unresolvable_source {
                eprintln!(
                    "Skipping {}: incoming relation source '{}' cannot be resolved ({}); \
                     resolve this duplicate by hand",
                    path, source_id, err
                );
                skipped.push(path.clone());
                continue;
            }

            if interactivity.is_interactive() && !yes {
                let prompt = format!(
                    "'{}' duplicates id '{}'. Renumber it to '{}'? [y/N]: ",
                    path, old_id, new_id
                );
                match prompt_input(&prompt) {
                    Ok(resp)
                        if resp.eq_ignore_ascii_case("y") || resp.eq_ignore_ascii_case("yes") => {}
                    Ok(_) => {
                        skipped.push(path.clone());
                        continue;
                    }
                    Err(e) => {
                        let err = QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to read renumber confirmation: {}", e),
                        );
                        let _ = output.emit_error(&err);
                        return ExitCode::InfrastructureFailure;
                    }
                }
            }

            // Reserved now so the next allocation in this run cannot pick the same id.
            used_ids.insert(new_id.clone());
            plan.push(PlannedRenumber {
                path: path.clone(),
                old_id: old_id.clone(),
                new_id,
            });
            if let Some(keeper) = paths.first() {
                if !keepers_to_rehydrate.contains(keeper) {
                    keepers_to_rehydrate.push(keeper.clone());
                }
            }
        }
    }

    // --- Write phase: everything below mutates the workspace. ---
    let mut renumbered: Vec<FixIdsEntry> = Vec::new();
    // A failure part-way through must still report what was already written: returning straight
    // out of the loop would leave renumbered files on disk with no record of which ones.
    let mut aborted: Option<QdevError> = None;
    let lock_path = root.join(&storage.cache_dir).join("write.lock");
    let lock_timeout = std::time::Duration::from_millis(5000);

    // True once the workspace has been touched at all. The reconcile below is gated on this,
    // not on `renumbered`: the relation and citation steps are fallible, so a run that aborted
    // in one of them had already written a file whose id the cache does not know yet.
    let mut wrote_anything = false;
    // Entries refused for their own reason (an occupied rename target) rather than aborting the
    // run, and entries never reached because the run aborted. Both end up in `skipped`; these
    // carry the detail the payload would otherwise lose.
    let mut refused: Vec<QdevError> = Vec::new();
    let mut unprocessed: Vec<String> = Vec::new();

    for planned in &plan {
        // The renumber writes the entity file and its cache row directly, so it takes the same
        // advisory lock `qdev update` and `qdev relate` take. The lock is scoped to each write
        // rather than held across the whole loop, because `apply_relation_change` below acquires
        // it for itself and the lock is not reentrant.
        let renumber_result = match qdev_core::acquire_write_lock(&lock_path, lock_timeout) {
            Ok(_guard) => {
                // Set before the call, not after: a failure inside it can still have written
                // the renamed file, and the reconcile is what makes that state coherent.
                wrote_anything = true;
                renumber_duplicate_file(
                    root,
                    annotated_config,
                    &planned.path,
                    &planned.new_id,
                    &author,
                )
            }
            Err(e) => Err(e),
        };
        let new_path = match renumber_result {
            Ok(new_path) => new_path,
            // A refused rename target is one entry's problem, not the run's: the frozen matrix
            // says refuse *that entry* and record it as skipped, so the remaining duplicate
            // groups — which the user already confirmed at the prompt — still get repaired.
            // Anything else aborts the loop, because a failure we cannot attribute to this one
            // file may well repeat on the next.
            Err(e) if e.code() == "rename_target_exists" => {
                skipped.push(planned.path.clone());
                refused.push(e);
                continue;
            }
            Err(e) => {
                skipped.push(planned.path.clone());
                aborted = Some(e);
                // Everything still planned is reported too, so the payload never leaves a
                // planned file in neither list.
                unprocessed.extend(
                    plan.iter()
                        .skip_while(|p| p.path != planned.path)
                        .skip(1)
                        .map(|p| p.path.clone()),
                );
                break;
            }
        };

        // Recorded the moment the file is written, before the two fallible steps below: an abort
        // in either of them must still report the write that already happened. Gating the report
        // on reaching the end of the loop body is what lost it before.
        renumbered.push(FixIdsEntry {
            old_path: planned.path.clone(),
            new_path,
            old_id: planned.old_id.clone(),
            new_id: planned.new_id.clone(),
            relations_rewritten: Vec::new(),
            citations_rewritten: 0,
        });
        let entry = renumbered.len() - 1;

        // References follow the renumbered entity. Each `apply_relation_change` takes the write
        // lock for its own write, so this runs outside the guard above.
        match rewrite_relations_to(
            root,
            annotated_config,
            &planned.old_id,
            &planned.new_id,
            &author,
        ) {
            Ok(sources) => renumbered[entry].relations_rewritten = sources,
            Err(e) => {
                aborted = Some(e);
                break;
            }
        };

        let citation_result = match qdev_core::acquire_write_lock(&lock_path, lock_timeout) {
            Ok(_guard) => rewrite_citations_under_modules(
                root,
                &annotated_config.config,
                &planned.old_id,
                &planned.new_id,
            ),
            Err(e) => Err(e),
        };
        match citation_result {
            Ok(count) => renumbered[entry].citations_rewritten = count,
            Err(e) => {
                aborted = Some(e);
                break;
            }
        };
    }

    // Reconcile the cache with what is now on disk before anything reads it back. The renumber
    // rewrote frontmatter ids directly, so without this a later read would see a cache still
    // holding the pre-renumber id and its relation rows.
    //
    // Runs even when the loop aborted: a partial renumber is exactly the case where the cache
    // and the workspace have diverged, and skipping it there would leave the keeper's id absent
    // from the cache with its `sync_state` row intact, so no later incremental sweep would
    // restore it. That is why the guard is "did we write anything" and not "did an entry make it
    // all the way through the loop body" — the latter is false in precisely the case that needs
    // the repair most.
    skipped.extend(unprocessed);
    // A run whose only failures were per-entry refusals still reports one of them, so the
    // command does not exit 0 having silently declined work the user confirmed.
    if aborted.is_none() {
        aborted = refused.into_iter().next();
    }

    if wrote_anything {
        let reconcile = open_query_store(root, annotated_config).and_then(|store| {
            // Dropping each keeper's `sync_state` row forces the sweep to re-parse it even
            // though its content is unchanged, so the id it kept is hydrated back into the
            // cache after the file that had been holding that row was renumbered away.
            for keeper in &keepers_to_rehydrate {
                store.delete_sync_state(keeper)?;
            }
            store.sweep_workspace(root, storage)
        });
        if let Err(e) = reconcile {
            aborted = aborted.or(Some(e));
        }
    }

    // Re-check the full validation surface (not just remaining duplicates), per the shared
    // exit-code rule: exit 1 iff any error-severity finding survives anywhere.
    let (mut findings, validation_err) = if aborted.is_none() {
        match open_query_store(root, annotated_config) {
            Ok(store) => match qdev_core::run_validation(&store, root, &annotated_config.config) {
                Ok(f) => (f, None),
                Err(e) => (Vec::new(), Some(e)),
            },
            Err(e) => (Vec::new(), Some(e)),
        }
    } else {
        (Vec::new(), None)
    };
    qdev_core::sort_findings(&mut findings);
    if aborted.is_none() {
        aborted = validation_err;
    }

    // Exactly one document on stdout in JSON mode: an abort is carried *inside* the payload, so
    // the report of what was already written stays parseable by the consumer that needs it most.
    if cli.json {
        let envelope = JsonEnvelope::new(FixIdsPayload {
            renumbered,
            skipped,
            error: aborted.as_ref().map(FixIdsError::from),
            findings: findings.clone(),
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit fix-ids envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        for entry in &renumbered {
            println!(
                "Renumbered {} ({} -> {})",
                entry.old_path, entry.old_id, entry.new_id
            );
            if entry.new_path != entry.old_path {
                println!("  Renamed {} -> {}", entry.old_path, entry.new_path);
            }
            if !entry.relations_rewritten.is_empty() || entry.citations_rewritten > 0 {
                println!(
                    "  {} relation source(s) and {} citation(s) redirected to {}",
                    entry.relations_rewritten.len(),
                    entry.citations_rewritten,
                    entry.new_id
                );
            }
        }
        for path in &skipped {
            println!("Skipped {}", path);
        }
        if !findings.is_empty() {
            let _ = output.emit_text(&render_validate_text(&findings));
        }
        // Text mode writes errors to stderr, so this cannot corrupt the report above.
        if let Some(e) = &aborted {
            let _ = output.emit_error(e);
        }
    }

    if let Some(e) = aborted {
        return e.exit_code();
    }

    if qdev_core::has_error_finding(&findings) {
        ExitCode::LogicalFailure
    } else {
        ExitCode::Success
    }
}

#[derive(Serialize)]
struct FixIdsEntry {
    /// The file's path before the renumber. The rename is performed as a write of the new path
    /// followed by a delete of this one, so git sees a delete/add pair whose rename detection is
    /// heuristic — this field and `new_path` are what make the move legible regardless.
    old_path: String,
    /// The file's path after the renumber: renamed to carry `new_id`, preserving any
    /// `-slug`/`_slug` suffix. Equal to `old_path` only if the name already carried `new_id`.
    new_path: String,
    old_id: String,
    new_id: String,
    /// Source entity ids whose relations were redirected from `old_id` to `new_id`.
    relations_rewritten: Vec<String>,
    /// How many citation occurrences under the configured module globs were redirected.
    citations_rewritten: usize,
}

/// An abort carried inside the payload rather than emitted as a second envelope. In JSON mode
/// `emit_error` writes to stdout, so emitting both would put two documents there — precisely
/// when the caller most needs to read which files were already rewritten.
#[derive(Serialize)]
struct FixIdsError {
    code: String,
    message: String,
}

impl From<&QdevError> for FixIdsError {
    fn from(e: &QdevError) -> Self {
        Self {
            code: e.code().to_string(),
            message: e.to_string(),
        }
    }
}

#[derive(Serialize, Default)]
struct FixIdsPayload {
    renumbered: Vec<FixIdsEntry>,
    skipped: Vec<String>,
    /// Present only when the renumber stopped part-way; `renumbered` still lists what was
    /// written before it stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<FixIdsError>,
    /// Present only when surviving validation findings exist. Clean runs omit the field.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    findings: Vec<qdev_core::FindingRecord>,
}

/// Where a refused renumber's file would have landed, and where it belongs instead.
struct OffConventionTarget {
    /// The path renumbering in place would have written — off-convention, so unwritable.
    in_place: String,
    /// The path in the kind's own directory the file must be moved to for a repair to work.
    expected_dir_path: String,
}

/// `Some(target)` when `rel_path` holds a duplicate that lives outside its kind's directory, so
/// `--fix-ids` must refuse it (Decision 2026-09-10, Simon: option A).
///
/// Renumbering such a file in place writes a correctly named file in the wrong directory: still
/// readable, because hydration walks both trees recursively, and still unresolvable by every
/// writer, which resolves an entity in its kind's directory only. Reporting that as a repair
/// would be a claim the run did not achieve — the class of defect `--fix-ids`' rename was added
/// to end. Moving the file is not a decision the tool takes silently, so the entry is skipped
/// with both paths named.
///
/// Kind comes from `kind_for_write`, hydration's own rule (frontmatter `kind:` first), so this
/// judges the file by the directory the following sweep will expect it in.
fn off_convention_directory_target(
    root: &std::path::Path,
    storage: &qdev_core::StorageConfig,
    rel_path: &str,
    new_id: &str,
) -> Result<Option<OffConventionTarget>, QdevError> {
    let rel_slashed = rel_path.replace('\\', "/");
    let abs_path = root.join(&rel_slashed);
    let content = std::fs::read_to_string(&abs_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to read '{}': {}", abs_path.display(), e),
        )
    })?;
    let kind = qdev_core::kind_for_write(&abs_path, &content, qdev_core::EntityKind::Story);
    let kind_dir = qdev_core::directory_for_kind(Some(storage), kind)
        .to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string();

    let (dir, old_name) = match rel_slashed.rsplit_once('/') {
        Some((dir, name)) => (dir.to_string(), name.to_string()),
        None => (String::new(), rel_slashed.clone()),
    };
    if dir == kind_dir {
        return Ok(None);
    }

    let existing_id = qdev_core::extract_frontmatter(&content)
        .ok()
        .and_then(|fm| fm.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()))
        .unwrap_or_default();
    let new_name = qdev_core::renamed_file_name(&old_name, &existing_id, new_id);
    let in_place = if dir.is_empty() {
        new_name.clone()
    } else {
        format!("{}/{}", dir, new_name)
    };
    Ok(Some(OffConventionTarget {
        in_place,
        expected_dir_path: format!("{}/{}", kind_dir, old_name),
    }))
}

/// Rewrites one duplicate file's frontmatter `id`, bumping `version`/`updated_by` via the same
/// line-based patch engine `qdev update` uses, then validates and atomically writes it —
/// **renaming the file to carry the new id** — and upserts the cache the same way
/// `apply_entity_update` does. Returns the file's new workspace-relative path.
///
/// The rename is what makes the renumbered entity writable. Every writer resolves an entity by
/// file name (`resolve_entity_file`), so rewriting the frontmatter id alone manufactured an
/// entity `qdev get` could read and `qdev update` could not find. The `-slug`/`_slug` suffix is
/// preserved (`renamed_file_name`), and an occupied target is refused rather than clobbered.
fn renumber_duplicate_file(
    root: &std::path::Path,
    annotated_config: &qdev_core::AnnotatedConfig,
    rel_path: &str,
    new_id: &str,
    author: &qdev_core::Author,
) -> Result<String, QdevError> {
    let abs_path = root.join(rel_path);
    let existing = std::fs::read_to_string(&abs_path).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to read '{}': {}", abs_path.display(), e),
        )
    })?;

    // Reuse hydration's own kind-inference (frontmatter `kind:` -> directory convention ->
    // identifier grammar -> `Story` default) so `--fix-ids` never diverges from how hydration
    // would classify the same file.
    let existing_frontmatter =
        qdev_core::extract_frontmatter(&existing).unwrap_or(serde_json::Value::Null);
    let existing_id = existing_frontmatter
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    // The same rule the other two writers use, so all three validate against the kind the next
    // sweep will classify the file as.
    // `EntityKind::Story` is the fallback only for content whose frontmatter cannot be
    // extracted, and it is what `determine_entity_kind` itself defaults to — so the fallback
    // agrees with hydration too.
    let kind = qdev_core::kind_for_write(&abs_path, &existing, qdev_core::EntityKind::Story);

    // The file must end up named for the id it declares. Refuse an occupied target rather than
    // clobbering a file that is very likely a real entity of its own: this runs on a workspace
    // that is already known to be damaged, and overwriting here would destroy the only copy.
    let old_name = match rel_path.rsplit_once('/') {
        Some((_, name)) => name,
        None => rel_path,
    };
    let new_name = qdev_core::renamed_file_name(old_name, existing_id, new_id);
    // `rsplit_once('/')` alone would treat a backslash-separated path as having no directory
    // and write the renamed file into the workspace root.
    let rel_path_slashed = rel_path.replace('\\', "/");
    let rel_path = rel_path_slashed.as_str();
    let new_rel_path = match rel_path.rsplit_once('/') {
        Some((dir, _)) => format!("{}/{}", dir, new_name),
        None => new_name.clone(),
    };
    let new_abs_path = root.join(&new_rel_path);
    if new_rel_path != rel_path {
        // `symlink_metadata` rather than `exists`, which follows a symlink and answers `false`
        // for a dangling one — writing through that link is exactly what refusing means to
        // prevent. The directory is then checked for *any* file the write path would resolve
        // for the new id, not just the exact target name: leaving an `E1S2-old.md` beside a
        // fresh `E1S2.md` makes every later write fail "multiple entity files match", which
        // would defeat the acceptance criterion this rename exists to satisfy.
        let occupied = new_abs_path.symlink_metadata().is_ok();
        let sibling = new_abs_path
            .parent()
            .map(|dir| qdev_core::find_file_in_dir_for_id(dir, new_id))
            .transpose()?
            .flatten()
            .filter(|found| *found != root.join(rel_path));
        if occupied || sibling.is_some() {
            let blocker = if occupied {
                new_rel_path.clone()
            } else {
                sibling
                    .map(|p| p.strip_prefix(root).unwrap_or(&p).display().to_string())
                    .unwrap_or_else(|| new_rel_path.clone())
            };
            return Err(QdevError::logical_failure(
                "rename_target_exists",
                format!(
                    "Renumbering '{}' to '{}' requires renaming it to '{}', but '{}' already \
                     claims that id; refusing to overwrite or shadow it",
                    rel_path, new_id, new_rel_path, blocker
                ),
            ));
        }
    }

    let id_rewritten = qdev_core::rewrite_frontmatter_id(&existing, new_id)?;

    let patch_opts = qdev_core::FrontmatterPatchOptions {
        status: None,
        title: None,
        custom_fields: Vec::new(),
        author: Some(author.clone()),
        if_version: None,
    };
    let (patched, new_version) = qdev_core::patch_frontmatter(&id_rewritten, &patch_opts)?;

    qdev_core::validate_frontmatter(kind, &patched).map_err(|errs| {
        QdevError::logical_failure(
            "schema_validation_failed",
            format!(
                "Renumbered frontmatter for '{}' failed schema validation: {:?}",
                rel_path, errs
            ),
        )
    })?;

    // Write the renamed file first, then drop the old one: an interruption between the two
    // leaves both copies on disk (a duplicate id, which is the condition already being repaired
    // and which `validate` reports) rather than no copy at all.
    qdev_core::write_file_atomic(&new_abs_path, &patched)?;
    if new_rel_path != rel_path {
        std::fs::remove_file(&abs_path).map_err(|e| {
            QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Renumbered '{}' was written to '{}' but the original could not be removed: {}",
                    rel_path, new_rel_path, e
                ),
            )
        })?;
    }

    let updated_frontmatter = qdev_core::extract_frontmatter(&patched).map_err(|e| {
        QdevError::infrastructure_failure(
            "parse_error",
            format!("Failed to parse renumbered frontmatter: {}", e),
        )
    })?;

    let content_hash = qdev_core::sha256_digest(patched.as_bytes());
    let title_val = updated_frontmatter
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let status_val = updated_frontmatter
        .get("status")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let owners_val = updated_frontmatter.get("owners").map(|v| v.to_string());
    let c_author = updated_frontmatter
        .get("created_by")
        .and_then(|v| serde_json::from_value::<qdev_core::Author>(v.clone()).ok());

    let epic_id = if kind == qdev_core::EntityKind::Story {
        match new_id.parse::<qdev_core::Identifier>() {
            Ok(qdev_core::Identifier::Story { epic, .. }) => Some(format!("E{}", epic)),
            _ => None,
        }
    } else {
        None
    };
    let seq = if kind == qdev_core::EntityKind::Story {
        match new_id.parse::<qdev_core::Identifier>() {
            Ok(qdev_core::Identifier::Story { story, .. }) => Some(story),
            _ => None,
        }
    } else {
        None
    };
    let appetite = updated_frontmatter
        .get("appetite")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let safety_class = updated_frontmatter
        .get("safety_class")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let target_modules = updated_frontmatter
        .get("target_modules")
        .map(|v| v.to_string());

    let record = qdev_core::EntityRecord {
        id: new_id.to_string(),
        kind,
        title: title_val,
        status: status_val,
        owners: owners_val,
        source_path: new_rel_path.clone(),
        content_hash,
        version: new_version,
        created_by: c_author,
        updated_by: Some(author.clone()),
        updated_at: qdev_core::current_iso8601(),
        stale: false,
        epic_id,
        seq,
        appetite,
        safety_class,
        target_modules,
    };

    let cache_db_path = root
        .join(&annotated_config.config.storage.cache_dir)
        .join("cache.sqlite");
    qdev_core::upsert_cache_and_mark_dirty(&cache_db_path, &record)?;

    // Two rows must not describe one entity. The row naming the old id at the old path now
    // describes a file that is not there any more, and `qdev get <old id>` would keep answering
    // from it. It is dropped only if it still claims *this* path: in the duplicate case that row
    // may belong to the keeper file, which legitimately holds the old id and is untouched.
    if new_rel_path != rel_path && !existing_id.is_empty() {
        qdev_core::purge_entity_row_for_moved_file(&cache_db_path, existing_id, rel_path)?;
    }

    Ok(new_rel_path)
}

/// Redirects every relation in the workspace that targets `old_id` to target `new_id` instead,
/// via `apply_relation_change` (`qdev relate`/`qdev unrelate`'s own write path): a relate-to-new
/// followed by an unrelate-from-old for each `(source, relation)` pair found via
/// `list_relations`. Returns the source ids whose files were rewritten.
///
/// The renumbered entity's own outgoing relations need no separate rewrite: their `source_id` is
/// re-derived from its frontmatter `id` at the next hydration sweep.
///
/// Note what this necessarily does in the duplicate case: the group's first file *keeps*
/// `old_id`, so an incoming edge that meant the keeper is redirected away from it. Nothing in
/// the workspace records which of the colliding files a reference meant, so references follow
/// the renumbered entity by decision — see the boundary note in spec-1-11.
fn rewrite_relations_to(
    root: &std::path::Path,
    annotated_config: &qdev_core::AnnotatedConfig,
    old_id: &str,
    new_id: &str,
    author: &qdev_core::Author,
) -> Result<Vec<String>, QdevError> {
    let affected: Vec<qdev_core::RelationRecord> = {
        let store = open_query_store(root, annotated_config)?;
        store
            .list_relations()?
            .into_iter()
            .filter(|r| r.target_id == old_id)
            .collect()
    };

    let mut rewritten: Vec<String> = Vec::new();
    for rel in affected {
        let options = |target: &str, add: bool| qdev_core::RelationChangeOptions {
            workspace_root: root.to_path_buf(),
            storage: Some(annotated_config.config.storage.clone()),
            entity_kind: None,
            entity_id: rel.source_id.clone(),
            relation: rel.relation.clone(),
            target_id: target.to_string(),
            add,
            if_version: None,
            author: author.clone(),
        };

        // Add the new edge before removing the old one: if the second call fails, the source is
        // left with an extra (visible) edge rather than silently losing the relation entirely.
        qdev_core::apply_relation_change(&options(new_id, true))?;
        qdev_core::apply_relation_change(&options(old_id, false))?;
        rewritten.push(rel.source_id);
    }
    rewritten.sort();
    rewritten.dedup();
    Ok(rewritten)
}

/// Redirects citations of `old_id` to `new_id` under the union of `config.modules[].paths`
/// (skipped entirely when no `[[modules]]` are configured), using
/// `config.hygiene.citation_pattern` (or the built-in default). Returns the number of citation
/// occurrences rewritten.
///
/// Only occurrences whose captured id is exactly `old_id` are touched: the citation pattern
/// matches every bracket citation kind, so rewriting every match would corrupt unrelated ids.
fn rewrite_citations_under_modules(
    root: &std::path::Path,
    config: &qdev_core::Config,
    old_id: &str,
    new_id: &str,
) -> Result<usize, QdevError> {
    let patterns = qdev_core::module_path_patterns(config);
    if patterns.is_empty() {
        return Ok(0);
    }

    let pattern_str = config
        .hygiene
        .citation_pattern
        .clone()
        .unwrap_or_else(|| qdev_core::DEFAULT_CITATION_PATTERN.to_string());
    let regex = qdev_core::regex::Regex::new(&pattern_str).map_err(|e| {
        QdevError::infrastructure_failure(
            "invalid_citation_pattern",
            format!("Invalid hygiene.citation_pattern: {}", e),
        )
    })?;

    // Prune the walk by the module globs rather than collecting the whole tree and filtering
    // afterwards: without pruning this enumerates `target/` and `node_modules/` in full.
    let mut candidate_files = Vec::new();
    qdev_core::collect_workspace_files_matching(root, root, &patterns, &mut candidate_files);

    let mut total_rewritten = 0usize;
    for rel_path in candidate_files {
        if !patterns.iter().any(|p| qdev_core::glob_match(p, &rel_path)) {
            continue;
        }
        let abs_path = root.join(&rel_path);
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue, // binary or unreadable file; nothing to rewrite
        };
        let (rewritten, count) = qdev_core::rewrite_citations(&content, &regex, old_id, new_id);
        if count > 0 {
            qdev_core::write_file_atomic(&abs_path, &rewritten)?;
            total_rewritten += count;
        }
    }

    Ok(total_rewritten)
}

/// Truncates `s` to at most `max` characters, appending an ellipsis when truncated.
fn truncate_field(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 1 {
        return s.chars().take(max).collect();
    }
    let truncated: String = s.chars().take(max - 1).collect();
    format!("{}…", truncated)
}
