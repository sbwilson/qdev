pub mod chore;
pub mod config;
pub mod context;
pub mod dag;
pub mod decision;
pub mod doctor;
pub mod dw;
pub mod envelope;
pub mod errors;
pub mod gate;
pub mod governance;
pub mod graph;
pub mod hook;
pub mod hygiene;
pub mod id;
pub mod impact;
pub mod init;
pub mod interactivity;
pub mod lease;
pub mod mcp;
pub mod modules;
pub mod next;
pub mod preflight;
pub mod pulse;
pub mod query;
pub mod review;
pub mod schema;
pub mod scratch;
pub mod skills;
pub mod soup;
pub mod sprint;
pub mod store;
pub mod transition;
pub mod validate;
pub mod write;

pub use impact::{
    format_impact_text, run_impact, ImpactDependent, ImpactOptions, ImpactOutcome, ImpactPayload,
    ImpactStory,
};

pub use hygiene::{
    check_hygiene, lint_comments, tokenize_comments, CommentKind, CommentLine, CommentToken,
    HygieneCheckOptions, HygieneCheckOutcome, HygieneFinding, HygieneLinter, SupportedLanguage,
    RULE_FORBID_PATTERNS, RULE_MAX_INLINE_COMMENT_LINES, RULE_REVIEW_ROUND, RULE_STORY_BANNER,
};

pub use preflight::{
    check_branch_freshness, check_validation_health, check_working_tree_scope,
    format_preflight_text, is_metadata_exempt, resolve_story_target_modules, run_preflight,
    PreflightDiagnostic, PreflightOptions, PreflightOutcome, PreflightPayload, PreflightStatus,
};

pub use gate::{
    evaluate_ratchet, execute_configured_command, execute_deps_gate, execute_gate,
    execute_gate_set, execute_hygiene_gate, execute_scope_gate, format_duration,
    format_metric_number, get_gate_list, parse_with_adapter, read_baseline, resolve_baseline_path,
    resolve_collision_free_evidence_path, resolve_commit_sha, resolve_gate_execution_order,
    scan_rust_imports, scan_swift_imports, validate_adapter_name, validate_gate_dependencies,
    write_baseline, write_evidence_bundle, EvidenceBundle, GateBaselinePayload, GateFailure,
    GateListItem, GateListPayload, GateResultDocument, GateRunOptions, GateRunOutcome,
    GateRunPayload, GateRunSetOutcome, GateRunSetPayload, GateStatus, HeadTailBuffer,
    RatchetBaseline, RatchetDirection, RatchetEvaluation, BUILTIN_GATE_DEPS, BUILTIN_GATE_HYGIENE,
    BUILTIN_GATE_SCOPE, VALID_ADAPTERS,
};
pub use soup::{
    parse_cargo_audit_json, parse_cargo_audit_json_reported, persist_soup_records,
    record_sbom_artifact, SoupAuditFinding, SoupAuditParse, SoupSummary,
};

pub use chore::{
    abort_chore, chore_dir, close_chore, commit_chore, derive_chore_id, find_open_chore,
    list_chore_records, start_chore, ChoreCommitResult, ChoreRecord, CommitChoreInput,
    ExcludedPath, FinishChoreInput, StartChoreInput,
};

pub use dw::{
    add_deferred_work, add_deferred_work_with_store, close_deferred_work, is_rationale_required,
    list_deferred_work_records, validate_safety_risk, AddDeferredWorkInput, CloseDeferredWorkInput,
    DeferredWorkItem, DeferredWorkPayload, ListDeferredWorkFilter, ListDeferredWorkPayload,
    VALID_SAFETY_RISKS,
};

pub use decision::{
    log_decision, log_decision_with_store, DecisionInput, DecisionLogPayload, VALID_DECISION_TYPES,
};

pub use scratch::{
    append_scratch_entry, estimate_tokens, filter_scratch_entries_by_budget, read_scratch_entries,
    summarize_scratch_entries, ScratchAppendPayload, ScratchReadPayload, ScratchpadAuthor,
    ScratchpadEntry, VALID_SCRATCH_KINDS,
};

pub use governance::{
    add_team_to_entity_owners, canonical_team_string, classify_mutation,
    create_governance_override_decision, extract_entity_owners, is_user_owner, normalize_team_name,
    resolve_user_teams, ScopeClassification,
};

pub use lease::{
    auto_release_lease, auto_release_lease_with_storage, claim_story,
    create_lease_override_decision, discover_git_branch, discover_git_common_dir,
    find_active_lease, find_active_lease_with_storage, find_workspace_leases,
    find_workspace_leases_with_storage, generate_session_token, get_lease, get_lease_with_storage,
    lease_dir, list_leases, list_leases_with_storage, parse_iso8601_to_timestamp, release_story,
    ReleasePayload, StoryLease,
};

pub use modules::ModuleRegistry;

pub use next::{
    select_next, NextBlocker, NextOptions, NextOwnerFilter, NextReason, NextSelection,
    NextStoryRecord,
};

pub use pulse::{
    build_pulse, CacheHealth, DeferredWorkCounts, EnvironmentPulse, GateSummary, IntegrationStatus,
    LeaseInfo, PulseOptions, PulsePayload, SprintPulse, StoryCounters, WorkingTreeStatus,
};
pub use review::{
    review_epic, review_epic_with_gates, review_sprint, EpicReview, ReviewDebt, ReviewStory,
    SprintReview,
};

pub use store::{
    create_schema, determine_entity_kind, drop_all_user_tables, ensure_cache,
    ensure_cache_with_summary, inspect_cache_schema, newer_cache_conflict, stamp_cache_version,
    CacheSchemaStatus, ConstraintRecord, DecisionRecord, DeferredWorkRecord, DirtyEntityRecord,
    EntityFilter, EntityPresence, EntityRecord, FindingRecord, GateRecord, GateRunRecord,
    RelationRecord, ScratchpadRecord, SoupRecord, SprintAssignmentRecord, SprintRecord,
    SqliteStore, Store, StoryRecord, SweepSummary, SyncStateRecord, ALL_TABLE_NAMES,
    BUSY_TIMEOUT_MS,
};

pub use dag::{
    allowed_kind_pairs, find_dependency_cycle, is_known_relation, is_valid_kind_pair,
    relation_names, validate_proposed_relation_map, would_create_cycle,
};

pub use graph::{
    build_story_graph, render_graph_dot, GraphEdge, GraphNode, GraphPayload, StoryGraphOptions,
    GRAPH_EDGE_RELATIONS,
};

pub use doctor::{
    default_doctor_sections, CacheDoctorSection, DoctorSection, DoctorSectionReport,
    GatesDoctorSection, GitDoctorSection, HooksDoctorSection, LeasesDoctorSection,
    McpDoctorSection, ModulesDoctorSection, SkillsDoctorSection, ValidationDoctorSection,
};

pub use mcp::{
    inspect_mcp, inspect_mcp_with_handshake_override, install_mcp, CallToolResult, JsonRpcError,
    JsonRpcRequest, JsonRpcResponse, McpDoctorStatus, McpInstallReport, McpServer, ToolContentItem,
    ToolDefinition,
};

pub use skills::{
    generate_agent_skills, generate_agent_skills_configured, generate_agent_skills_with_models,
    generate_claude_skills, generate_claude_skills_configured, generate_claude_skills_with_models,
    generate_cursor_rule, generate_cursor_rule_configured, generate_cursor_rule_content,
    generate_cursor_rule_content_configured, generate_cursor_rule_content_with_models,
    generate_cursor_rule_with_models, generate_skill_content, generate_skill_content_configured,
    generate_skill_content_with_models, inspect_skills, install_skills, install_skills_configured,
    install_skills_with_models, CommandCatalog, CommandDefinition, OptionDefinition,
    SkillInstallOptions, SkillsInstallReport, SkillsStatus, SubcommandDefinition, CORE_SKILL_NAMES,
    MANAGED_SKILL_PATHS, STRUCTURED_SYNTHESIS_TEMPLATE,
};

pub use hook::{
    inspect_hooks, install_hooks, resolve_hooks_dir, run_legacy_hook, run_pre_commit, run_pre_push,
    run_prepare_commit_msg, scan_staged_secrets, shim_content, HookInstallReport, HookStatus,
    SecretViolation, EXPECTED_HOOKS,
};

pub use config::{
    default_synthesis_headings, find_workspace_root, load_config, load_project_storage,
    merge_configs, resolve_git_email, validate_config_table, AnnotatedConfig, AnnotatedValue,
    CommitMessagesConfig, Config, ConfigSource, EnvironmentConfig, GateConfig, GitConfig,
    HygieneConfig, IdentityConfig, LeasesConfig, ModelsConfig, ModuleConfig, PreferencesConfig,
    ProjectConfig, RegulatoryConfig, SoupConfig, StorageConfig, SynthesisConfig, TeamsConfig,
    DEFAULT_CITATION_PATTERN, LOCAL_CONFIG_FILENAME, PROJECT_CONFIG_FILENAME,
};
pub use context::{
    build_context, render_context_markdown, render_context_text, ContextOptions, ContextPayload,
    ContextPhase, ContextSection, ContextStats, TruncatedSection, HYGIENE_DIRECTIVE,
};
pub use envelope::{ErrorPayload, JsonEnvelope, JsonErrorEnvelope, SCHEMA_VERSION};
pub use errors::{
    get_refusal_entry, ExitCode, QdevError, RefusalCatalogEntry, RejectionAttribution,
    REFUSAL_CATALOG,
};
pub use id::{
    allocate_decision_id, allocate_decision_id_in_with_rng, allocate_decision_id_with_rng,
    allocate_deferred_work_id, allocate_deferred_work_id_in_with_rng,
    allocate_deferred_work_id_with_rng, allocate_next_constraint_id,
    allocate_next_constraint_id_in, allocate_next_story_id, allocate_next_story_id_in,
    parse_constraint_seq, ConstraintKind, ConstraintOwner, IdParseError, Identifier,
    IdentifierKind,
};
pub use init::{
    gitignore_entries, init, qdev_dir, standard_directories, verify_cache_compatible, InitLayout,
    InitOptions, InitResult, CACHE_SCHEMA_VERSION, SPEC_SUBDIRECTORIES, STANDARD_DIRECTORIES,
    STATE_SUBDIRECTORIES,
};
pub use interactivity::Interactivity;
pub use query::{
    query_entity, query_list, ConstraintProjection, EntityProjection, EvidenceProjection,
    GetResult, ListEntryProjection, ListQueryOptions, QueryOptions, ScratchEntryProjection,
    SprintAssignmentProjection,
};
pub use regex;
pub use rusqlite;
pub use schema::{
    evidence_bundle_schema_json, evidence_bundle_schema_str, extract_frontmatter,
    extract_frontmatter_str, validate_evidence, validate_evidence_detailed, validate_frontmatter,
    validate_frontmatter_detailed, validate_frontmatter_value, validate_gate_result,
    validate_gate_result_detailed, validate_value_detailed, EntityKind, PayloadKind, SchemaError,
    ValidationError,
};
pub use serde_yaml;
pub use sprint::{
    assign_to_sprint, close_sprint, normalize_sprint_id, open_sprint, resolve_sprint_selection,
    sprint_entity_id, SprintAssignOptions, SprintAssignResult, SprintCloseOptions,
    SprintCloseResult, SprintOpenOptions, SprintOpenResult,
};
pub use transition::{
    append_scratchpad_entry, classify_transition, create_transition_decision,
    record_transition_decision, PostTransitionHook, PreTransitionHook, StoryState,
    TransitionContext, TransitionEngine, TransitionGateHook, TransitionKind, TransitionOptions,
    TransitionPayload,
};
pub use validate::{
    collect_workspace_files_matching, filter_by_changed, find_duplicate_active_sprint_assignments,
    find_duplicate_planning_ids, find_dw_missing_rationale, find_off_convention_entity_files,
    find_orphan_deferred_work, find_unmatched_module_globs, find_unregistered_target_modules,
    git_changed_files, glob_match, has_error_finding, ids_in_use, ids_in_use_from_scan,
    module_path_patterns, next_available_id, rewrite_citations, rewrite_frontmatter_id,
    run_validation, scan_duplicate_planning_ids, sort_findings, DuplicateIdScan,
};
pub use write::{
    acquire_workspace_write_lock, acquire_write_lock, apply_constraint_add,
    apply_constraint_remove, apply_entity_update, apply_relation_change, canonical_file_name,
    create_story, current_iso8601, directory_for_kind, filename_carries_id,
    find_file_in_dir_for_id, id_carried_by_filename, iso8601_from_timestamp, kind_for_write,
    patch_frontmatter, purge_entity_row_for_moved_file, relation_map_from_yaml, renamed_file_name,
    replace_markdown_section, resolve_author, resolve_entity_file, sha256_digest,
    upsert_cache_and_mark_dirty, upsert_cache_with_constraint, upsert_cache_with_full_relations,
    upsert_cache_with_relation, write_file_atomic, AdvisoryLockGuard, Author, ConstraintAddOptions,
    ConstraintAddResult, ConstraintRemoveOptions, ConstraintRemoveResult, ConstraintRowChange,
    EntityUpdateOptions, EntityUpdateResult, FrontmatterPatchOptions, RelationCacheUpdate,
    RelationChangeOptions, RelationChangeResult, RelationRowChange, StoryCreateOptions,
    StoryCreateResult,
};
