use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::{Deref, DerefMut};

/// Project configuration section `[project]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_sprint: Option<u32>,
}

/// Teams configuration section `[teams]`.
/// Maps team name to team member developer IDs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct TeamsConfig {
    pub teams: BTreeMap<String, Vec<String>>,
}

impl TeamsConfig {
    pub fn new(teams: BTreeMap<String, Vec<String>>) -> Self {
        Self { teams }
    }
}

impl Deref for TeamsConfig {
    type Target = BTreeMap<String, Vec<String>>;

    fn deref(&self) -> &Self::Target {
        &self.teams
    }
}

impl DerefMut for TeamsConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.teams
    }
}

/// Git integration configuration section `[git]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitConfig {
    #[serde(default = "default_git_remote")]
    pub remote: String,
    #[serde(default = "default_integration_branch")]
    pub integration_branch: String,
    #[serde(default = "default_branching_mode")]
    pub branching_mode: String,
    #[serde(default = "default_branch_template")]
    pub branch_template: String,
    #[serde(default = "default_true")]
    pub require_clean_tree_in_scope: bool,
    #[serde(default = "default_max_integration_staleness_commits")]
    pub max_integration_staleness_commits: u32,
}

fn default_git_remote() -> String {
    "origin".to_string()
}

fn default_integration_branch() -> String {
    "develop".to_string()
}

fn default_branching_mode() -> String {
    "story-branch".to_string()
}

fn default_branch_template() -> String {
    "feature/{story_id}-{slug}".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_integration_staleness_commits() -> u32 {
    20
}

impl Default for GitConfig {
    fn default() -> Self {
        Self {
            remote: default_git_remote(),
            integration_branch: default_integration_branch(),
            branching_mode: default_branching_mode(),
            branch_template: default_branch_template(),
            require_clean_tree_in_scope: default_true(),
            max_integration_staleness_commits: default_max_integration_staleness_commits(),
        }
    }
}

/// Storage layout configuration section `[storage]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_specs_dir")]
    pub specs_dir: String,
    #[serde(default = "default_state_dir")]
    pub state_dir: String,
    #[serde(default = "default_cache_dir")]
    pub cache_dir: String,
}

fn default_specs_dir() -> String {
    "docs/specs".to_string()
}

fn default_state_dir() -> String {
    "docs/state".to_string()
}

fn default_cache_dir() -> String {
    ".qdev/cache".to_string()
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            specs_dir: default_specs_dir(),
            state_dir: default_state_dir(),
            cache_dir: default_cache_dir(),
        }
    }
}

/// Module registry entry in array section `[[modules]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleConfig {
    pub id: String,
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub may_depend_on: Vec<String>,
}

/// Code hygiene configuration section `[hygiene]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HygieneConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_inline_comment_lines")]
    pub max_inline_comment_lines: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbid_patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation_pattern: Option<String>,
    #[serde(default = "default_languages")]
    pub languages: Vec<String>,
}

fn default_max_inline_comment_lines() -> u32 {
    6
}

fn default_languages() -> Vec<String> {
    vec![
        "rust".to_string(),
        "swift".to_string(),
        "python".to_string(),
    ]
}

impl Default for HygieneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_inline_comment_lines: default_max_inline_comment_lines(),
            forbid_patterns: Vec::new(),
            citation_pattern: None,
            languages: default_languages(),
        }
    }
}

/// Regulatory configuration section `[regulatory]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RegulatoryConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iec62304_class: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub require_rationale_for: Vec<String>,
}

/// SOUP (Software of Unknown Provenance) audit configuration `[soup]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SoupConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deny_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbom_command: Option<String>,
}

/// Advisory model selection configuration section `[models]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specify: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub develop: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
}

/// Commit message policy configuration section `[commit_messages]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitMessagesConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_commit_format")]
    pub format: String,
}

fn default_commit_format() -> String {
    "conventional".to_string()
}

impl Default for CommitMessagesConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            format: default_commit_format(),
        }
    }
}

/// Environment variables section `[environment]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct EnvironmentConfig {
    pub variables: BTreeMap<String, String>,
}

impl EnvironmentConfig {
    pub fn new(variables: BTreeMap<String, String>) -> Self {
        Self { variables }
    }
}

impl Deref for EnvironmentConfig {
    type Target = BTreeMap<String, String>;

    fn deref(&self) -> &Self::Target {
        &self.variables
    }
}

impl DerefMut for EnvironmentConfig {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.variables
    }
}

/// Gate definition entry in array section `[[gates]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateConfig {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_transition: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verifies: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip: Option<bool>,
}

/// Local developer identity configuration section `[identity]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct IdentityConfig {
    #[serde(default)]
    pub developer_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub teams: Vec<String>,
}

/// Local developer preferences configuration section `[preferences]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreferencesConfig {
    #[serde(default = "default_true")]
    pub color: bool,
    #[serde(default = "default_format")]
    pub default_format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<String>,
}

fn default_format() -> String {
    "text".to_string()
}

impl Default for PreferencesConfig {
    fn default() -> Self {
        Self {
            color: true,
            default_format: default_format(),
            editor: None,
        }
    }
}

/// Universal typed configuration schema combining all 14 sections.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub teams: TeamsConfig,
    #[serde(default)]
    pub git: GitConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub modules: Vec<ModuleConfig>,
    #[serde(default)]
    pub hygiene: HygieneConfig,
    #[serde(default)]
    pub regulatory: RegulatoryConfig,
    #[serde(default)]
    pub soup: SoupConfig,
    #[serde(default)]
    pub models: ModelsConfig,
    #[serde(default)]
    pub commit_messages: CommitMessagesConfig,
    #[serde(default)]
    pub environment: EnvironmentConfig,
    #[serde(default)]
    pub gates: Vec<GateConfig>,
    #[serde(default)]
    pub identity: IdentityConfig,
    #[serde(default)]
    pub preferences: PreferencesConfig,
}
