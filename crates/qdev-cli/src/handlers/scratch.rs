use crate::cli::{self, Cli};
use crate::output::OutputEmitter;
use crate::{open_query_store, prompt_input};
use qdev_core::{resolve_author, ExitCode, Interactivity, JsonEnvelope, QdevError};

pub fn handle_scratch(
    args: &cli::ScratchArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    match args.command {
        cli::ScratchCommands::Append(ref append_args) => handle_scratch_append(
            append_args,
            annotated_config,
            cli,
            output,
            current_dir,
            interactivity,
        ),
        cli::ScratchCommands::Read(ref read_args) => {
            handle_scratch_read(read_args, annotated_config, cli, output, current_dir)
        }
    }
}

pub fn handle_scratch_append(
    append_args: &cli::ScratchAppendArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
    interactivity: Interactivity,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // 1. Resolve target story
    let (_entity_kind, canonical_story_id, _story_path) = match qdev_core::resolve_entity_file(
        &root,
        Some(qdev_core::EntityKind::Story),
        &append_args.story,
        Some(&annotated_config.config.storage),
    ) {
        Ok(res) => res,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // 2. Validate kind
    if let Some(ref k) = append_args.kind {
        let trimmed = k.trim();
        if !qdev_core::VALID_SCRATCH_KINDS.contains(&trimmed) {
            let err = qdev_core::QdevError::usage_error(format!(
                "Invalid scratchpad kind '{}', must be one of: note, decision, tradeoff, transition",
                trimmed
            ));
            let _ = output.emit_error(&err);
            return ExitCode::UsageError;
        }
    }

    // 3. Validate text
    if append_args.text.trim().is_empty() {
        let err = qdev_core::QdevError::usage_error("Scratchpad text cannot be empty");
        let _ = output.emit_error(&err);
        return ExitCode::UsageError;
    }

    // 4. Resolve author (single resolution for lease check, override audit, and append)
    let author = match resolve_author(
        append_args.author_type.as_deref(),
        append_args.author_id.as_deref(),
        annotated_config,
        &root,
    ) {
        Ok(a) => a,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // 5. Check lease authorization
    let workspace_leases = match qdev_core::find_workspace_leases(&root) {
        Ok(l) => l,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    let has_active_lease = workspace_leases
        .iter()
        .any(|l| l.story_id == canonical_story_id);

    let mut override_decision: Option<String> = None;

    if !has_active_lease {
        if cli.r#override {
            let just = cli.justification.as_deref().unwrap_or("").trim();
            if just.is_empty() {
                let err = qdev_core::QdevError::policy_refusal(
                    "needs_justification",
                    "--override requires a non-empty --justification",
                )
                .with_details(serde_json::json!({
                    "story_id": canonical_story_id,
                }))
                .with_attribution(
                    qdev_core::RejectionAttribution::new(
                        "Override requires non-empty justification",
                    )
                    .with_policy("justification_required"),
                );
                let _ = output.emit_error(&err);
                return ExitCode::PolicyRefusal;
            }
            override_decision = Some(just.to_string());
        } else if interactivity.is_non_interactive() {
            let mut attr = qdev_core::RejectionAttribution::new(
                "Scratchpad append outside active lease requires override and justification",
            )
            .with_policy("lease_scope");
            if let Some(l) = workspace_leases.first() {
                attr = attr.with_holder(&l.holder);
            }

            let mut details = serde_json::json!({
                "story_id": canonical_story_id,
            });
            if let Some(l) = workspace_leases.first() {
                details["active_lease"] = serde_json::json!({
                    "story_id": l.story_id,
                    "holder": l.holder,
                    "worktree_path": l.worktree_path,
                });
            }

            let err = qdev_core::QdevError::policy_refusal(
                "needs_confirmation",
                format!(
                    "Scratchpad append on story '{}' requires an active lease; re-run with --override --justification \"<rationale>\"",
                    canonical_story_id
                ),
            )
            .with_details(details)
            .with_attribution(attr);
            let _ = output.emit_error(&err);
            return ExitCode::PolicyRefusal;
        } else {
            eprintln!("⚠ Out-of-lease scratchpad append");
            eprintln!("  Story   {}", canonical_story_id);
            if let Some(l) = workspace_leases.first() {
                eprintln!("  Lease   {} (held in {})", l.story_id, l.worktree_path);
            }
            eprintln!("  [1] Override with justification");
            eprintln!("  [2] Abort");

            let choice = match prompt_input("") {
                Ok(c) => c,
                Err(_) => {
                    println!("Aborted.");
                    return ExitCode::Success;
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
                            return ExitCode::PolicyRefusal;
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
                        return ExitCode::PolicyRefusal;
                    }
                    override_decision = Some(trimmed_just.to_string());
                }
                _ => {
                    println!("Aborted.");
                    return ExitCode::Success;
                }
            }
        }
    }

    // 6. Append scratchpad entry
    let opt_store = open_query_store(&root, annotated_config).ok();
    let entry = match qdev_core::append_scratch_entry(
        &root,
        Some(&annotated_config.config.storage),
        &canonical_story_id,
        append_args.kind.as_deref(),
        &append_args.text,
        &author,
        opt_store.as_ref().map(|s| s as &dyn qdev_core::Store),
    ) {
        Ok(e) => e,
        Err(err) => {
            let _ = output.emit_error(&err);
            return err.exit_code();
        }
    };

    // 7. If an override took place, log governance decision record after append succeeded
    if let Some(just) = override_decision {
        if let Err(e) = qdev_core::create_governance_override_decision(
            &root,
            Some(&annotated_config.config.storage),
            &canonical_story_id,
            "lease_override",
            &just,
            &author,
            Some(&format!(
                "Scratchpad append on unleased story {} overridden by {}",
                canonical_story_id, author.id
            )),
        ) {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    }

    if cli.json {
        let envelope = JsonEnvelope::new(qdev_core::ScratchAppendPayload {
            story_id: canonical_story_id.clone(),
            seq: entry.seq,
            at: entry.at.clone(),
            author: entry.author.clone(),
            kind: entry.kind.clone(),
            text: entry.text.clone(),
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit scratchpad envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else {
        println!(
            "Appended entry #{} [{}] to scratchpad for {}",
            entry.seq, entry.kind, canonical_story_id
        );
    }

    ExitCode::Success
}

pub fn handle_scratch_read(
    read_args: &cli::ScratchReadArgs,
    annotated_config: &qdev_core::AnnotatedConfig,
    cli: &Cli,
    output: &OutputEmitter,
    current_dir: &std::path::Path,
) -> ExitCode {
    let root = qdev_core::find_workspace_root(current_dir);

    // 1. Resolve target story
    let (_entity_kind, canonical_story_id, _story_path) = match qdev_core::resolve_entity_file(
        &root,
        Some(qdev_core::EntityKind::Story),
        &read_args.story,
        Some(&annotated_config.config.storage),
    ) {
        Ok(res) => res,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // 2. Read entries
    let entries = match qdev_core::read_scratch_entries(
        &root,
        Some(&annotated_config.config.storage),
        &canonical_story_id,
    ) {
        Ok(e) => e,
        Err(e) => {
            let _ = output.emit_error(&e);
            return e.exit_code();
        }
    };

    // 3. Filter entries
    let filtered_entries = if read_args.summary {
        let last_n = read_args.last.unwrap_or(5);
        qdev_core::summarize_scratch_entries(&entries, last_n, read_args.budget)
    } else if let Some(budget) = read_args.budget {
        qdev_core::filter_scratch_entries_by_budget(&entries, budget)
    } else {
        entries
    };

    // 4. Output
    if cli.json {
        let envelope = JsonEnvelope::new(qdev_core::ScratchReadPayload {
            story_id: canonical_story_id.clone(),
            entries: filtered_entries,
        });
        if let Err(e) = output.emit_envelope(&envelope) {
            let err = QdevError::infrastructure_failure(
                "io_error",
                format!("Failed to emit scratchpad envelope: {}", e),
            );
            let _ = output.emit_error(&err);
            return ExitCode::InfrastructureFailure;
        }
    } else if filtered_entries.is_empty() {
        println!("(empty)");
    } else {
        for entry in &filtered_entries {
            println!(
                "[{}] [{}] {}:{} {}: {}",
                entry.seq, entry.kind, entry.author.r#type, entry.author.id, entry.at, entry.text
            );
        }
    }

    ExitCode::Success
}
