pub mod config;
pub mod envelope;
pub mod errors;
pub mod id;
pub mod init;
pub mod interactivity;
pub mod schema;
pub mod store;
pub mod write;

pub use store::{
    create_schema_v1, drop_all_user_tables, ensure_cache, inspect_cache_schema, CacheSchemaStatus,
    ConstraintRecord, DecisionRecord, DeferredWorkRecord, DirtyEntityRecord, EntityFilter,
    EntityRecord, GateRecord, GateRunRecord, RelationRecord, ScratchpadRecord, SoupRecord,
    SprintAssignmentRecord, SprintRecord, SqliteStore, Store, StoryRecord, SyncStateRecord,
    ALL_TABLE_NAMES, BUSY_TIMEOUT_MS, CACHE_USER_VERSION,
};

pub use config::{
    find_workspace_root, load_config, merge_configs, resolve_git_email, validate_config_table,
    AnnotatedConfig, AnnotatedValue, CommitMessagesConfig, Config, ConfigSource, EnvironmentConfig,
    GateConfig, GitConfig, HygieneConfig, IdentityConfig, ModelsConfig, ModuleConfig,
    PreferencesConfig, ProjectConfig, RegulatoryConfig, SoupConfig, StorageConfig, TeamsConfig,
    DEFAULT_CITATION_PATTERN,
};
pub use envelope::{ErrorPayload, JsonEnvelope, JsonErrorEnvelope, SCHEMA_VERSION};
pub use errors::{ExitCode, QdevError};
pub use id::{
    allocate_decision_id, allocate_decision_id_with_rng, allocate_deferred_work_id,
    allocate_deferred_work_id_with_rng, allocate_next_story_id, ConstraintKind, ConstraintOwner,
    IdParseError, Identifier, IdentifierKind,
};
pub use init::{
    check_cache_status, init, CacheStatus, InitOptions, InitResult, CACHE_SCHEMA_VERSION,
    GITIGNORE_ENTRIES, STANDARD_DIRECTORIES,
};
pub use interactivity::Interactivity;
pub use rusqlite;
pub use schema::{
    extract_frontmatter, extract_frontmatter_str, validate_frontmatter,
    validate_frontmatter_detailed, validate_frontmatter_value, validate_value_detailed, EntityKind,
    SchemaError, ValidationError,
};
pub use serde_yaml;
pub use write::{
    acquire_write_lock, apply_entity_update, patch_frontmatter, replace_markdown_section,
    resolve_entity_file, sha256_digest, upsert_cache_and_mark_dirty, write_file_atomic,
    AdvisoryLockGuard, Author, EntityUpdateOptions, EntityUpdateResult, FrontmatterPatchOptions,
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
