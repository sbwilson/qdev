use std::collections::BTreeMap;

use crate::config::{Config, ModuleConfig};
use crate::validate::glob_match;

/// Module registry managing declared modules, path mappings, and target module validation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleRegistry {
    modules: Vec<ModuleConfig>,
}

impl ModuleRegistry {
    /// Create a new registry from a list of module configurations.
    pub fn new(modules: Vec<ModuleConfig>) -> Self {
        Self { modules }
    }

    /// Construct a module registry from the loaded project configuration.
    pub fn from_config(config: &Config) -> Self {
        Self {
            modules: config.modules.clone(),
        }
    }

    /// Return a slice of all registered module configurations in declaration order.
    pub fn modules(&self) -> &[ModuleConfig] {
        &self.modules
    }

    /// Look up a module configuration by its unique ID.
    pub fn get(&self, id: &str) -> Option<&ModuleConfig> {
        self.modules.iter().find(|m| m.id == id)
    }

    /// Check whether a module ID is registered in the configuration.
    pub fn is_registered(&self, id: &str) -> bool {
        self.modules.iter().any(|m| m.id == id)
    }

    /// Map a repository path to zero or more matching module IDs in declaration order.
    ///
    /// Paths are normalized by converting backslashes to slashes, stripping leading `./` and
    /// leading slashes before matching against each module's `paths` globs.
    pub fn resolve_path(&self, rel_path: &str) -> Vec<String> {
        let mut normalized = rel_path.replace('\\', "/");
        loop {
            if let Some(stripped) = normalized.strip_prefix('/') {
                normalized = stripped.to_string();
            } else if let Some(stripped) = normalized.strip_prefix("./") {
                normalized = stripped.to_string();
            } else {
                break;
            }
        }

        let mut matched = Vec::new();
        for m in &self.modules {
            if m.paths
                .iter()
                .any(|pattern| glob_match(pattern, &normalized))
            {
                matched.push(m.id.clone());
            }
        }
        matched
    }

    /// Validate a collection of target module IDs against the registry.
    ///
    /// Returns `Ok(())` if all IDs are registered, or `Err(unregistered_ids)` containing
    /// the unrecognized IDs without duplicates in first-seen order.
    pub fn validate_target_modules<T: AsRef<str>>(
        &self,
        target_modules: &[T],
    ) -> Result<(), Vec<String>> {
        let mut unregistered = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for m in target_modules {
            let s = m.as_ref();
            if !self.is_registered(s) && seen.insert(s.to_string()) {
                unregistered.push(s.to_string());
            }
        }
        if unregistered.is_empty() {
            Ok(())
        } else {
            Err(unregistered)
        }
    }

    /// Return a resolved mapping of module IDs to their declared path globs.
    pub fn to_module_paths_map(&self) -> BTreeMap<String, Vec<String>> {
        self.modules
            .iter()
            .map(|m| (m.id.clone(), m.paths.clone()))
            .collect()
    }
}

impl From<&Config> for ModuleRegistry {
    fn from(config: &Config) -> Self {
        Self::from_config(config)
    }
}
