mod cli;
mod output;

use std::io::IsTerminal;
use std::panic;
use std::process::ExitCode as StdExitCode;

use clap::Parser;
use qdev_core::{ExitCode, Interactivity, JsonEnvelope, JsonErrorEnvelope, QdevError};
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

    let annotated_config = match qdev_core::load_config(&current_dir) {
        Ok(cfg) => cfg,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

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
        Some(Commands::Init(_)) => unreachable!(),
    }
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
            println!("✔ cache schema migrated to v1");
        } else if result.already_initialized {
            println!("✔ cache schema v1 up to date");
        } else {
            println!("✔ cache schema v1");
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
    let story_id = match qdev_core::allocate_next_story_id(&root, epic_num) {
        Ok(id) => id,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let rel_path = format!("docs/specs/stories/{}.md", story_id);
    let abs_path = root.join(&rel_path);

    if let Some(parent) = abs_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to create directory '{}': {}", parent.display(), e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    }

    let author = if !annotated_config.config.identity.developer_id.is_empty() {
        annotated_config.config.identity.developer_id.clone()
    } else {
        qdev_core::resolve_git_email(Some(&root)).unwrap_or_else(|| "developer".to_string())
    };

    let title = story_args.title.as_deref().unwrap_or("");
    let title_json = serde_json::to_string(title).unwrap_or_default();

    let mut frontmatter = format!(
        "---\nid: {}\ntitle: {}\nstatus: draft\n",
        story_id, title_json
    );

    if let Some(ref appetite) = story_args.appetite {
        frontmatter.push_str(&format!("appetite: {}\n", appetite));
    }

    if let Some(ref safety) = story_args.safety_class {
        frontmatter.push_str(&format!("safety_class: {}\n", safety));
    }

    if !story_args.module.is_empty() {
        let modules_json =
            serde_json::to_string(&story_args.module).unwrap_or_else(|_| "[]".to_string());
        frontmatter.push_str(&format!("target_modules: {}\n", modules_json));
    }

    if !story_args.owner.is_empty() {
        let owners_json =
            serde_json::to_string(&story_args.owner).unwrap_or_else(|_| "[]".to_string());
        frontmatter.push_str(&format!("owners: {}\n", owners_json));
    }

    frontmatter.push_str(&format!(
        "version: 1\ncreated_by:\n  type: human\n  id: {}\nupdated_by:\n  type: human\n  id: {}\n---\n\n## Acceptance Criteria\n",
        author, author
    ));

    use std::io::Write;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&abs_path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let err = QdevError::conflict(
                "file_exists",
                format!("Story file already exists: {}", abs_path.display()),
            );
            let _ = output.emit_error(&err);
            return ExitCode::Conflict;
        }
        Err(e) => {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!(
                    "Failed to create story file '{}': {}",
                    abs_path.display(),
                    e
                ),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    };

    if let Err(e) = file.write_all(frontmatter.as_bytes()) {
        let err = QdevError::infrastructure_failure(
            "io_error",
            format!("Failed to write story file '{}': {}", abs_path.display(), e),
        );
        let _ = output.emit_error(&err);
        return ExitCode::InfrastructureFailure;
    }

    if cli.json {
        let envelope = JsonEnvelope::new(CreateStoryPayload {
            id: story_id.to_string(),
            path: rel_path,
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
        println!("Created story {} at {}", story_id, rel_path);
    }

    ExitCode::Success
}
