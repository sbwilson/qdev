pub mod config;
pub mod envelope;
pub mod errors;
pub mod init;
pub mod interactivity;

pub use config::{
    find_workspace_root, load_config, merge_configs, resolve_git_email, validate_config_table,
    AnnotatedConfig, AnnotatedValue, CommitMessagesConfig, Config, ConfigSource, EnvironmentConfig,
    GateConfig, GitConfig, HygieneConfig, IdentityConfig, ModelsConfig, ModuleConfig,
    PreferencesConfig, ProjectConfig, RegulatoryConfig, SoupConfig, StorageConfig, TeamsConfig,
};
pub use envelope::{ErrorPayload, JsonEnvelope, JsonErrorEnvelope, SCHEMA_VERSION};
pub use errors::{ExitCode, QdevError};
pub use init::{
    check_cache_status, init, CacheStatus, InitOptions, InitResult, CACHE_SCHEMA_VERSION,
    GITIGNORE_ENTRIES, STANDARD_DIRECTORIES,
};
pub use interactivity::Interactivity;
pub use rusqlite;

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
