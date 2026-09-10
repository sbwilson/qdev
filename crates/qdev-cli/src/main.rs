mod cli;
mod output;

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
        Err(_) => ExitCode::InfrastructureFailure,
    };

    StdExitCode::from(exit_code)
}

fn run(raw_args: &[String]) -> ExitCode {
    let json_mode = is_json_requested(raw_args);
    let output = OutputEmitter::new(json_mode);

    let cli = match Cli::try_parse_from(raw_args) {
        Ok(cli) => cli,
        Err(clap_err) => {
            if clap_err.kind() == clap::error::ErrorKind::DisplayHelp {
                print!("{}", clap_err);
                return ExitCode::Success;
            }

            let err_msg = clap_err.to_string();
            let qdev_err = QdevError::usage_error(err_msg.trim());
            let _ = output.emit_error(&qdev_err);
            return ExitCode::UsageError;
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
                return ExitCode::InfrastructureFailure;
            }
        } else {
            println!("qdev {}", env!("CARGO_PKG_VERSION"));
        }
        return ExitCode::Success;
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
            return ExitCode::InfrastructureFailure;
        }
    };

    // Dispatch init command before loading config so bootstrapping works in uninitialized directories
    if let Some(Commands::Init(ref init_args)) = cli.command {
        return handle_init(init_args, interactivity, &cli, &output, &current_dir);
    }

    // Dispatch schema command before loading config so it works without an initialized workspace
    if let Some(Commands::Schema(ref schema_args)) = cli.command {
        return handle_schema(schema_args, &cli, &output);
    }

    let annotated_config = match qdev_core::load_config(&current_dir) {
        Ok(cfg) => cfg,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let root = qdev_core::find_workspace_root(&current_dir);

    // Every command that touches the cache is refused outside an initialized workspace, checked
    // once here rather than per handler. Opening the store directly creates an empty,
    // schema-less `cache.sqlite`, so a command that forgot the check would litter the directory
    // and fail with a raw `sqlite_error: no such table` instead of a clean usage error.
    if requires_workspace(cli.command.as_ref()) {
        if let Err(e) = ensure_query_workspace(&root) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    // Boot-time cache verification and initialization per spec-1-6 (in initialized workspaces)
    if root.join("qdev.toml").is_file() {
        if let Err(e) = qdev_core::ensure_cache(&root, &annotated_config.config.storage) {
            // A cache stamped by a newer binary is refused on boot for every command — except
            // `qdev sync --rebuild`, the documented recovery path the refusal itself names.
            // That command drops and repopulates every table from the Markdown files, so it is
            // the one caller that does not need to read the newer cache first. Every other
            // command, `qdev doctor` included, still exits 5 here.
            let recoverable_by_this_command = e.code() == "schema_version_mismatch"
                && matches!(cli.command, Some(Commands::Sync(ref args)) if args.rebuild);
            if !recoverable_by_this_command {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        }
    }

    // Dispatch commands
    match cli.command {
        None | Some(Commands::Status) => {
            let status = qdev_core::get_pulse_status(interactivity);
            if cli.json {
                let envelope = JsonEnvelope::new(status);
                if let Err(e) = output.emit_envelope(&envelope) {
                    let err = QdevError::infrastructure_failure(
                        "io_error",
                        format!("Failed to emit status envelope: {}", e),
                    );
                    let _ = output.emit_error(&err);
                    return ExitCode::InfrastructureFailure;
                }
            } else {
                println!("qdev {}", env!("CARGO_PKG_VERSION"));
            }
            ExitCode::Success
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
                        return ExitCode::InfrastructureFailure;
                    }
                } else {
                    let report = annotated_config.to_text_report();
                    if let Err(e) = output.emit_text(&report) {
                        let err = QdevError::infrastructure_failure(
                            "io_error",
                            format!("Failed to emit config report: {}", e),
                        );
                        let _ = output.emit_error(&err);
                        return ExitCode::InfrastructureFailure;
                    }
                }
                ExitCode::Success
            }
        },
        Some(Commands::Create(ref create_args)) => match create_args.command {
            CreateCommands::Story(ref story_args) => {
                handle_create_story(story_args, &annotated_config, &cli, &output, &current_dir)
            }
        },
        Some(Commands::Update(ref update_args)) => {
            handle_update(update_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::Get(ref get_args)) => {
            handle_get(get_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::List(ref list_args)) => {
            handle_list(list_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::Relate(ref relate_args)) => {
            handle_relate(relate_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::Unrelate(ref unrelate_args)) => handle_unrelate(
            unrelate_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
        ),
        Some(Commands::Graph(ref graph_args)) => {
            handle_graph(graph_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::Validate(ref validate_args)) => handle_validate(
            validate_args,
            &annotated_config,
            &cli,
            &output,
            &current_dir,
            interactivity,
        ),
        Some(Commands::Sync(ref sync_args)) => {
            handle_sync(sync_args, &annotated_config, &cli, &output, &current_dir)
        }
        Some(Commands::Doctor) => handle_doctor(&annotated_config, &cli, &output, &current_dir),
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
fn resolve_kind_and_id(
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
fn reject_empty_filter_values(flags: &[(&str, Option<&str>)]) -> Result<(), QdevError> {
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
/// The exceptions are deliberate: `status` and `config show` report on whatever they find,
/// `init` and `schema` are dispatched before this point, and `create story` bootstraps — it is
/// specified and tested to work in a clean directory, allocating the first id and writing the
/// story file. Outside a workspace it deliberately leaves no `.qdev/` behind: no cache (an
/// unstamped one would make the next `qdev init` demand a confirmed migration) and no advisory
/// lock (there is no other writer to serialize against). Everything else reads or writes an
/// existing cache.
///
/// Deciding this from the command itself, rather than from a call inside each handler, is what
/// stops the next new command from silently shipping without the guard — the omission that let
/// `qdev update` keep creating a stray, schema-less `cache.sqlite` outside a workspace and fail
/// with a raw `sqlite_error` after `get`/`list` were fixed.
fn requires_workspace(command: Option<&Commands>) -> bool {
    match command {
        None | Some(Commands::Status) | Some(Commands::Init(_)) | Some(Commands::Schema(_)) => {
            false
        }
        // Matched at subcommand granularity, not by whole variant: a future `create epic` or
        // `config set` must be classified deliberately rather than inheriting the exemption.
        Some(Commands::Create(args)) => match args.command {
            CreateCommands::Story(_) => false,
        },
        Some(Commands::Config(args)) => match args.command {
            ConfigCommands::Show => false,
        },
        Some(Commands::Update(_))
        | Some(Commands::Get(_))
        | Some(Commands::List(_))
        | Some(Commands::Relate(_))
        | Some(Commands::Unrelate(_))
        | Some(Commands::Graph(_))
        | Some(Commands::Validate(_))
        | Some(Commands::Sync(_))
        | Some(Commands::Doctor) => true,
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
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let (name, developer, teams, allow_migration) = if interactivity.is_non_interactive() {
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
                }));
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
                }));
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
            }));
            let _ = output.emit_error(&err);
            return ExitCode::PolicyRefusal;
        }

        let allow_migration = init_args.yes;
        match qdev_core::check_cache_status(&root) {
            Ok(qdev_core::CacheStatus::NeedsMigration {
                current_version,
                target_version,
            }) if !allow_migration => {
                let err = QdevError::policy_refusal(
                    "needs_confirmation",
                    format!(
                        "Cache schema migration from v{} to v{} requires confirmation or --yes",
                        current_version, target_version
                    ),
                )
                .with_details(serde_json::json!({
                    "current_version": current_version,
                    "target_version": target_version,
                    "flag": "--yes",
                }));
                let _ = output.emit_error(&err);
                return ExitCode::PolicyRefusal;
            }
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
            _ => {}
        }

        (name, developer, teams, allow_migration)
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

        let mut allow_migration = init_args.yes;
        if !allow_migration {
            match qdev_core::check_cache_status(&root) {
                Ok(qdev_core::CacheStatus::NeedsMigration {
                    current_version,
                    target_version,
                }) => {
                    let prompt = format!(
                        "Migrate cache schema from v{} to v{}? [y/N]: ",
                        current_version, target_version
                    );
                    match prompt_input(&prompt) {
                        Ok(resp)
                            if resp.eq_ignore_ascii_case("y")
                                || resp.eq_ignore_ascii_case("yes") =>
                        {
                            allow_migration = true;
                        }
                        Ok(_) => {
                            let err = QdevError::policy_refusal(
                                "needs_confirmation",
                                "Cache schema migration requires confirmation or --yes",
                            )
                            .with_details(serde_json::json!({
                                "current_version": current_version,
                                "target_version": target_version,
                                "flag": "--yes",
                            }));
                            let _ = output.emit_error(&err);
                            return ExitCode::PolicyRefusal;
                        }
                        Err(e) => {
                            let err = QdevError::infrastructure_failure(
                                "io_error",
                                format!("Failed to read migration confirmation: {}", e),
                            );
                            let _ = output.emit_error(&err);
                            return ExitCode::InfrastructureFailure;
                        }
                    }
                }
                Err(e) => {
                    let _ = output.emit_error(&e);
                    return e.exit_code();
                }
                _ => {}
            }
        }

        (name, developer, teams, allow_migration)
    };

    let options = qdev_core::InitOptions {
        root,
        name,
        developer,
        teams,
        allow_migration,
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
        println!("✔ qdev.toml");
        println!("✔ .qdev.local.toml (gitignored)");
        println!("✔ .qdev/cache/ (gitignored), .qdev/gates/");
        println!("✔ docs/specs/{{prd,requirements,epics,stories,adrs,hazards}}");
        println!("✔ docs/state/{{sprints,releases,dw,decisions,scratch,evidence,baselines,soup}}");
        if result.cache_migrated {
            println!("✔ cache schema migrated to v{}", CACHE_SCHEMA_VERSION);
        } else if result.already_initialized {
            println!("✔ cache schema v{} up to date", CACHE_SCHEMA_VERSION);
        } else {
            println!("✔ cache schema v{}", CACHE_SCHEMA_VERSION);
        }
    }

    ExitCode::Success
}

fn prompt_input(prompt: &str) -> std::io::Result<String> {
    use std::io::{self, Write};
    eprint!("{}", prompt);
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
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
    let story_id = match qdev_core::allocate_next_story_id_in(&root, storage, epic_num) {
        Ok(id) => id,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

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

fn handle_update(
    update_args: &cli::UpdateArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
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
        workspace_root: root,
        storage: Some(annotated_config.config.storage.clone()),
        entity_kind,
        entity_id,
        status: update_args.status.clone(),
        title: update_args.title.clone(),
        custom_fields,
        section: update_args.section.clone(),
        section_file,
        if_version: update_args.if_version,
        author,
    };

    let res = match qdev_core::apply_entity_update(&update_opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

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
fn open_query_store(
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

    // Validate --expand values: only "scratch" changes behavior; "relations"/"constraints" are
    // accepted as no-ops since the default projection already includes them.
    let mut expand_scratch = false;
    for value in &get_args.expand {
        match value.trim() {
            "scratch" => expand_scratch = true,
            "relations" | "constraints" | "" => {}
            other => {
                let err = QdevError::usage_error(format!(
                    "Unknown --expand value '{}', expected one of: relations, constraints, scratch",
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

    let query_opts = qdev_core::QueryOptions { expand_scratch };

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

/// Relation kinds rendered as graph edges; `traces_to`, `verifies`, `mitigates`, `closes_dw`,
/// and `governed_by` connect stories to non-story entities and are out of scope for this
/// story-only graph.
const GRAPH_EDGE_RELATIONS: [&str; 3] = ["depends_on", "extends", "supersedes"];

fn dot_status_style(status: Option<&str>) -> (&'static str, &'static str) {
    match status {
        Some("draft") => ("lightgray", "solid"),
        Some("ready") => ("lightblue", "solid"),
        Some("in-progress") => ("yellow", "solid"),
        Some("review") => ("orange", "solid"),
        Some("done") => ("green", "solid"),
        Some("superseded") | Some("abandoned") => ("gray45", "dashed"),
        _ => ("white", "solid"),
    }
}

/// Escapes a value for use inside a double-quoted DOT string literal.
fn dot_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn render_graph_dot(
    nodes: &[qdev_core::ListEntryProjection],
    relations: &[qdev_core::RelationRecord],
) -> String {
    let node_ids: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();

    let mut dot = String::from("digraph qdev {\n");
    for node in nodes {
        let (color, style) = dot_status_style(node.status.as_deref());
        let label = match &node.title {
            Some(title) => format!("{}\\n{}", dot_escape(&node.id), dot_escape(title)),
            None => dot_escape(&node.id),
        };
        dot.push_str(&format!(
            "  \"{}\" [label=\"{}\", style=\"filled,{}\", fillcolor=\"{}\"];\n",
            dot_escape(&node.id),
            label,
            style,
            color
        ));
    }
    for rel in relations {
        if !GRAPH_EDGE_RELATIONS.contains(&rel.relation.as_str()) {
            continue;
        }
        if !node_ids.contains(rel.source_id.as_str()) || !node_ids.contains(rel.target_id.as_str())
        {
            continue;
        }
        dot.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{}\"];\n",
            dot_escape(&rel.source_id),
            dot_escape(&rel.target_id),
            dot_escape(&rel.relation)
        ));
    }
    dot.push_str("}\n");
    dot
}

fn handle_graph(
    graph_args: &cli::GraphArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    // Both refusals are about the shape of the flags, so they are usage errors (exit 2) like
    // every other flag rejection in this binary — not policy refusals (exit 3), which AD-13
    // reserves for preflight, lease, governance, and missing-confirmation refusals. A script
    // branching on exit 3 must not be sent down the "a human has to confirm something" path by
    // a plain typo.
    if !graph_args.dot {
        let err = QdevError::usage_error(
            "qdev graph currently requires '--dot'; no other output format is supported yet",
        )
        .with_details(serde_json::json!({ "flag": "--dot" }));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    if cli.json {
        let err = QdevError::usage_error(
            "qdev graph does not support '--json' yet; only '--dot' output is available",
        )
        .with_details(serde_json::json!({ "flag": "--json" }));
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    let root = qdev_core::find_workspace_root(current_dir);

    if let Err(e) = reject_empty_filter_values(&[("--epic", graph_args.epic.as_deref())]) {
        let _ = output.emit_error(&e);
        return e.exit_code();
    }

    let mut query_opts = qdev_core::ListQueryOptions::new(qdev_core::EntityKind::Story);
    query_opts.epic_id = graph_args.epic.clone();

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let nodes = match qdev_core::query_list(&store, &query_opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let relations = match store.list_relations() {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let dot = render_graph_dot(&nodes, &relations);
    if let Err(e) = output.emit_text(&dot) {
        let err = QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to emit graph output: {}", e),
        );
        let _ = output.emit_error(&err);
        return ExitCode::InfrastructureFailure;
    }

    ExitCode::Success
}

/// Resolves the active author attribution for every command that records one: explicit CLI
/// flags, then `QDEV_AUTHOR_TYPE`/`QDEV_AUTHOR_ID`, then config identity, then the git email.
///
/// This is the only author-resolution path. `qdev update` used to carry an inline copy, which is
/// how the two drifted: attribution is audited (AD-12), so a second copy is a second answer.
fn resolve_author(
    author_type: Option<&str>,
    author_id: Option<&str>,
    annotated_config: &qdev_core::AnnotatedConfig,
    root: &std::path::Path,
) -> Result<qdev_core::Author, QdevError> {
    // An unrecognized value is refused wherever it comes from. Silently rewriting a bad
    // `QDEV_AUTHOR_TYPE` to "human" wrote a wrong-but-plausible author into the audited
    // attribution, with nothing to tell the user their environment was misconfigured.
    let validate = |value: String, source: &str| -> Result<String, QdevError> {
        if value == "human" || value == "agent" {
            Ok(value)
        } else {
            Err(QdevError::usage_error(format!(
                "Invalid author type '{}' from {}, must be 'human' or 'agent'",
                value, source
            )))
        }
    };

    let resolved_type = if let Some(at) = author_type {
        validate(at.to_string(), "--author-type")?
    } else if let Ok(env_at) = std::env::var("QDEV_AUTHOR_TYPE") {
        validate(env_at, "QDEV_AUTHOR_TYPE")?
    } else {
        "human".to_string()
    };

    // An empty or whitespace-only value is treated as absent rather than accepted and then
    // rejected downstream by `Author::validate`: `QDEV_AUTHOR_ID=` (a var defined but never
    // given a value, common in CI) used to fail every mutation with "Author ID cannot be
    // empty" and no way to fall back to the config identity.
    let flag_id = author_id.filter(|aid| !aid.trim().is_empty());
    let env_id = std::env::var("QDEV_AUTHOR_ID")
        .ok()
        .filter(|aid| !aid.trim().is_empty());
    let resolved_id = if let Some(aid) = flag_id {
        aid.to_string()
    } else if let Some(env_aid) = env_id {
        env_aid
    } else if !annotated_config.config.identity.developer_id.is_empty() {
        annotated_config.config.identity.developer_id.clone()
    } else {
        qdev_core::resolve_git_email(Some(root)).unwrap_or_else(|| "developer".to_string())
    };

    Ok(qdev_core::Author::new(resolved_type, resolved_id))
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

fn handle_relate(
    relate_args: &cli::RelateArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    let store = match open_query_store(&root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Refuse before any write: wrong kind pair, dangling target, or a would-be depends_on cycle.
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

    let target_id = match store.get_entity(&relate_args.target_id) {
        Ok(Some(target)) => {
            if !qdev_core::is_valid_kind_pair(&relate_args.relation, source.kind, target.kind) {
                let err = QdevError::logical_failure(
                    "invalid_relation_kind",
                    format!(
                        "Relation '{}' from {} ({}) to {} ({}) is not an allowed kind pair",
                        relate_args.relation,
                        relate_args.source_id,
                        source.kind,
                        relate_args.target_id,
                        target.kind
                    ),
                );
                let _ = output.emit_error(&err);
                return err.exit_code();
            }
            target.id
        }
        Ok(None) => {
            let err = QdevError::logical_failure(
                "dangling_relation",
                format!("Target entity '{}' does not exist", relate_args.target_id),
            );
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if relate_args.relation == "depends_on" {
        let existing_edges = match store.list_relations() {
            Ok(rows) => rows
                .into_iter()
                .filter(|r| r.relation == "depends_on")
                .map(|r| (r.source_id, r.target_id))
                .collect::<Vec<_>>(),
            Err(e) => {
                let _ = output.emit_error(&e);
                return e.exit_code();
            }
        };
        if let Some(cycle) = qdev_core::would_create_cycle(&existing_edges, &source.id, &target_id)
        {
            let err = QdevError::logical_failure(
                "dependency_cycle",
                format!(
                    "Adding depends_on from '{}' to '{}' would create a cycle: {}",
                    source.id,
                    target_id,
                    cycle.join(" -> ")
                ),
            );
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
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

    let opts = qdev_core::RelationChangeOptions {
        workspace_root: root,
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
        if_version: relate_args.if_version,
        author,
    };

    let res = match qdev_core::apply_relation_change(&opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

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
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

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

    let opts = qdev_core::RelationChangeOptions {
        workspace_root: root,
        storage: Some(annotated_config.config.storage.clone()),
        entity_kind: None,
        entity_id: unrelate_args.source_id.clone(),
        relation: unrelate_args.relation.clone(),
        target_id: unrelate_args.target_id.clone(),
        add: false,
        if_version: None,
        author,
    };

    // Unrelating an absent entry is an idempotent no-op (exit 0), matching `relate`/`unrelate`'s
    // I/O contract; apply_relation_change already returns `changed: false` without writing.
    let res = match qdev_core::apply_relation_change(&opts) {
        Ok(r) => r,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

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
        "parsed={}, unchanged={}, purged={}, findings={}\n",
        summary.parsed, summary.unchanged, summary.purged, summary.findings
    )
}

/// Renders doctor section reports as a flat text block, one section per group of lines.
fn render_doctor_text(sections: &[qdev_core::DoctorSectionReport]) -> String {
    let mut out = String::new();
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
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

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
        let text = render_doctor_text(&sections);
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
        .with_details(serde_json::json!({ "flag": "--yes" }));
        let _ = output.emit_error(&err);
        return ExitCode::PolicyRefusal;
    }

    let storage = &annotated_config.config.storage;
    let scan = match qdev_core::scan_duplicate_planning_ids(root, &storage.specs_dir) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    if scan.groups.is_empty() {
        if cli.json {
            let envelope = JsonEnvelope::new(FixIdsPayload::default());
            let _ = output.emit_envelope(&envelope);
        } else {
            println!("No duplicate planning ids found.");
        }
        return ExitCode::Success;
    }

    let author = match resolve_author(None, None, annotated_config, root) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // Seed the in-use id set from the cache as well as the on-disk scan. `scan.all_ids` only
    // covers `specs_dir`, so allocating from it alone can hand out an id that already belongs
    // to an entity living elsewhere — minting a fresh duplicate while fixing one.
    let mut used_ids = scan.all_ids.clone();
    match open_query_store(root, annotated_config)
        .and_then(|store| store.list_entities(&qdev_core::EntityFilter::default()))
    {
        Ok(entities) => used_ids.extend(entities.into_iter().map(|e| e.id)),
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

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
        // The first (lexicographically sorted) path keeps the id; every other file declaring it
        // is offered a renumber.
        for path in paths.iter().skip(1) {
            let old_identifier: qdev_core::Identifier = match old_id.parse() {
                Ok(id) => id,
                Err(_) => {
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

    for planned in &plan {
        // The renumber writes the entity file and its cache row directly, so it takes the same
        // advisory lock `qdev update` and `qdev relate` take. The lock is scoped to each write
        // rather than held across the whole loop, because `apply_relation_change` below acquires
        // it for itself and the lock is not reentrant.
        let renumber_result = match qdev_core::acquire_write_lock(&lock_path, lock_timeout) {
            Ok(_guard) => renumber_duplicate_file(
                root,
                annotated_config,
                &planned.path,
                &planned.new_id,
                &author,
            ),
            Err(e) => Err(e),
        };
        if let Err(e) = renumber_result {
            aborted = Some(e);
            break;
        }

        // References follow the renumbered entity. Each `apply_relation_change` takes the write
        // lock for its own write, so this runs outside the guard above.
        let relations_rewritten = match rewrite_relations_to(
            root,
            annotated_config,
            &planned.old_id,
            &planned.new_id,
            &author,
        ) {
            Ok(sources) => sources,
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
        let citations_rewritten = match citation_result {
            Ok(count) => count,
            Err(e) => {
                aborted = Some(e);
                break;
            }
        };

        renumbered.push(FixIdsEntry {
            path: planned.path.clone(),
            old_id: planned.old_id.clone(),
            new_id: planned.new_id.clone(),
            relations_rewritten,
            citations_rewritten,
        });
    }

    // Reconcile the cache with what is now on disk before anything reads it back. The renumber
    // rewrote frontmatter ids directly, so without this a later read would see a cache still
    // holding the pre-renumber id and its relation rows.
    //
    // Runs even when the loop aborted: a partial renumber is exactly the case where the cache
    // and the workspace have diverged, and skipping it there would leave the keeper's id absent
    // from the cache with its `sync_state` row intact, so no later incremental sweep would
    // restore it.
    if !renumbered.is_empty() {
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

    // Exactly one document on stdout in JSON mode: an abort is carried *inside* the payload, so
    // the report of what was already written stays parseable by the consumer that needs it most.
    if cli.json {
        let envelope = JsonEnvelope::new(FixIdsPayload {
            renumbered,
            skipped,
            error: aborted.as_ref().map(FixIdsError::from),
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
                entry.path, entry.old_id, entry.new_id
            );
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
        // Text mode writes errors to stderr, so this cannot corrupt the report above.
        if let Some(e) = &aborted {
            let _ = output.emit_error(e);
        }
    }

    if let Some(e) = aborted {
        return e.exit_code();
    }

    // Re-check the full validation surface (not just remaining duplicates), per the shared
    // exit-code rule: exit 1 iff any error-severity finding survives anywhere.
    let store = match open_query_store(root, annotated_config) {
        Ok(s) => s,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };
    match qdev_core::run_validation(&store, root, &annotated_config.config) {
        Ok(findings) if !qdev_core::has_error_finding(&findings) => ExitCode::Success,
        Ok(_) => ExitCode::LogicalFailure,
        Err(e) => {
            let _ = output.emit_error(&e);
            e.exit_code()
        }
    }
}

#[derive(Serialize)]
struct FixIdsEntry {
    path: String,
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
}

/// Rewrites one duplicate file's frontmatter `id`, bumping `version`/`updated_by` via the same
/// line-based patch engine `qdev update` uses, then validates and atomically writes it, and
/// upserts the cache the same way `apply_entity_update` does.
fn renumber_duplicate_file(
    root: &std::path::Path,
    annotated_config: &qdev_core::AnnotatedConfig,
    rel_path: &str,
    new_id: &str,
    author: &qdev_core::Author,
) -> Result<(), QdevError> {
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
    let kind = qdev_core::determine_entity_kind(&abs_path, existing_id, &existing_frontmatter);

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

    qdev_core::write_file_atomic(&abs_path, &patched)?;

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
        source_path: rel_path.to_string(),
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
    qdev_core::upsert_cache_and_mark_dirty(&cache_db_path, &record)
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
