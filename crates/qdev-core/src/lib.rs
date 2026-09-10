pub mod config;
pub mod dag;
pub mod doctor;
pub mod envelope;
pub mod errors;
pub mod id;
pub mod init;
pub mod interactivity;
pub mod query;
pub mod schema;
pub mod store;
pub mod validate;
pub mod write;

pub use store::{
    create_schema, determine_entity_kind, drop_all_user_tables, ensure_cache, inspect_cache_schema,
    newer_cache_conflict, stamp_cache_version, CacheSchemaStatus, ConstraintRecord, DecisionRecord,
    DeferredWorkRecord, DirtyEntityRecord, EntityFilter, EntityRecord, FindingRecord, GateRecord,
    GateRunRecord, RelationRecord, ScratchpadRecord, SoupRecord, SprintAssignmentRecord,
    SprintRecord, SqliteStore, Store, StoryRecord, SweepSummary, SyncStateRecord, ALL_TABLE_NAMES,
    BUSY_TIMEOUT_MS,
};

pub use dag::{allowed_kind_pairs, find_dependency_cycle, is_valid_kind_pair, would_create_cycle};

pub use doctor::{
    default_doctor_sections, CacheDoctorSection, DoctorSection, DoctorSectionReport,
    ValidationDoctorSection,
};

pub use config::{
    find_workspace_root, load_config, load_project_storage, merge_configs, resolve_git_email,
    validate_config_table, AnnotatedConfig, AnnotatedValue, CommitMessagesConfig, Config,
    ConfigSource, EnvironmentConfig, GateConfig, GitConfig, HygieneConfig, IdentityConfig,
    ModelsConfig, ModuleConfig, PreferencesConfig, ProjectConfig, RegulatoryConfig, SoupConfig,
    StorageConfig, TeamsConfig, DEFAULT_CITATION_PATTERN, LOCAL_CONFIG_FILENAME,
    PROJECT_CONFIG_FILENAME,
};
pub use envelope::{ErrorPayload, JsonEnvelope, JsonErrorEnvelope, SCHEMA_VERSION};
pub use errors::{ExitCode, QdevError};
pub use id::{
    allocate_decision_id, allocate_decision_id_in_with_rng, allocate_decision_id_with_rng,
    allocate_deferred_work_id, allocate_deferred_work_id_in_with_rng,
    allocate_deferred_work_id_with_rng, allocate_next_story_id, allocate_next_story_id_in,
    ConstraintKind, ConstraintOwner, IdParseError, Identifier, IdentifierKind,
};
pub use init::{
    check_cache_status, gitignore_entries, init, standard_directories, CacheStatus, InitLayout,
    InitOptions, InitResult, CACHE_SCHEMA_VERSION, SPEC_SUBDIRECTORIES, STANDARD_DIRECTORIES,
    STATE_SUBDIRECTORIES,
};
pub use interactivity::Interactivity;
pub use query::{
    query_entity, query_list, ConstraintProjection, EntityProjection, GetResult,
    ListEntryProjection, ListQueryOptions, QueryOptions, ScratchEntryProjection,
};
pub use regex;
pub use rusqlite;
pub use schema::{
    extract_frontmatter, extract_frontmatter_str, validate_frontmatter,
    validate_frontmatter_detailed, validate_frontmatter_value, validate_value_detailed, EntityKind,
    PayloadKind, SchemaError, ValidationError,
};
pub use serde_yaml;
pub use validate::{
    collect_workspace_files_matching, filter_by_changed, find_duplicate_planning_ids,
    find_dw_missing_rationale, find_off_convention_entity_files, find_orphan_deferred_work,
    find_unregistered_target_modules, git_changed_files, glob_match, has_error_finding, ids_in_use,
    ids_in_use_from_scan, module_path_patterns, next_available_id, rewrite_citations,
    rewrite_frontmatter_id, run_validation, scan_duplicate_planning_ids, sort_findings,
    DuplicateIdScan,
};
pub use write::{
    acquire_write_lock, apply_entity_update, apply_relation_change, canonical_file_name,
    create_story, current_iso8601, directory_for_kind, find_file_in_dir_for_id,
    id_carried_by_filename, iso8601_from_timestamp, kind_for_write, patch_frontmatter,
    purge_entity_row_for_moved_file, renamed_file_name, replace_markdown_section,
    resolve_entity_file, sha256_digest, upsert_cache_and_mark_dirty, upsert_cache_with_relation,
    write_file_atomic, AdvisoryLockGuard, Author, EntityUpdateOptions, EntityUpdateResult,
    FrontmatterPatchOptions, RelationChangeOptions, RelationChangeResult, RelationRowChange,
    StoryCreateOptions, StoryCreateResult,
};

use serde::{Deserialize, Serialize};

/// Basic pulse status returned by core routine for default command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PulseStatus {
    pub name: String,
    pub version: String,
    pub interactivity: Interactivity,
}

/// Core routine returning the current status given the resolved interactivity mode.
pub fn get_pulse_status(interactivity: Interactivity) -> PulseStatus {
    PulseStatus {
        name: "qdev".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        interactivity,
    }
}
