use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::types::Config;

/// Source attribution for a configuration key or section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ConfigSource {
    #[serde(rename = "qdev.toml")]
    Project,
    #[serde(rename = ".qdev.local.toml")]
    Local,
    #[serde(rename = "git")]
    Git,
    #[serde(rename = "default")]
    Default,
}

impl std::fmt::Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigSource::Project => write!(f, "qdev.toml"),
            ConfigSource::Local => write!(f, ".qdev.local.toml"),
            ConfigSource::Git => write!(f, "git"),
            ConfigSource::Default => write!(f, "default"),
        }
    }
}

/// A value annotated with its configuration source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotatedValue<T> {
    pub value: T,
    pub source: ConfigSource,
}

impl<T> AnnotatedValue<T> {
    pub fn new(value: T, source: ConfigSource) -> Self {
        Self { value, source }
    }

    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn source(&self) -> ConfigSource {
        self.source
    }
}

/// Effective configuration alongside per-key source file annotations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotatedConfig {
    pub config: Config,
    pub sources: BTreeMap<String, ConfigSource>,
}

impl AnnotatedConfig {
    pub fn new(config: Config, sources: BTreeMap<String, ConfigSource>) -> Self {
        Self { config, sources }
    }

    pub fn get_source(&self, key: &str) -> Option<ConfigSource> {
        self.sources.get(key).copied()
    }

    pub fn get_annotated<T: Clone>(&self, value: T, key: &str) -> AnnotatedValue<T> {
        let source = self
            .sources
            .get(key)
            .copied()
            .unwrap_or(ConfigSource::Default);
        AnnotatedValue::new(value, source)
    }

    /// Render human-readable text report showing effective configuration and key sources per AD-1.
    pub fn to_text_report(&self) -> String {
        let mut out = String::new();
        out.push_str("Configuration:\n\n");

        out.push_str("[project]\n");
        let src = self
            .sources
            .get("project.name")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  name = {:?} (source: {})\n",
            self.config.project.name, src
        ));
        if let Some(sprint) = self.config.project.default_sprint {
            let src = self
                .sources
                .get("project.default_sprint")
                .unwrap_or(&ConfigSource::Default);
            out.push_str(&format!(
                "  default_sprint = {} (source: {})\n",
                sprint, src
            ));
        }

        if !self.config.teams.is_empty() {
            out.push_str("\n[teams]\n");
            for (team, members) in &self.config.teams.teams {
                let key = format!("teams.{}", team);
                let src = self.sources.get(&key).unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  {} = {:?} (source: {})\n", team, members, src));
            }
        }

        out.push_str("\n[git]\n");
        let src = self
            .sources
            .get("git.remote")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  remote = {:?} (source: {})\n",
            self.config.git.remote, src
        ));
        let src = self
            .sources
            .get("git.integration_branch")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  integration_branch = {:?} (source: {})\n",
            self.config.git.integration_branch, src
        ));
        let src = self
            .sources
            .get("git.branching_mode")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  branching_mode = {:?} (source: {})\n",
            self.config.git.branching_mode, src
        ));
        let src = self
            .sources
            .get("git.branch_template")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  branch_template = {:?} (source: {})\n",
            self.config.git.branch_template, src
        ));
        let src = self
            .sources
            .get("git.require_clean_tree_in_scope")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  require_clean_tree_in_scope = {} (source: {})\n",
            self.config.git.require_clean_tree_in_scope, src
        ));
        let src = self
            .sources
            .get("git.max_integration_staleness_commits")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  max_integration_staleness_commits = {} (source: {})\n",
            self.config.git.max_integration_staleness_commits, src
        ));

        out.push_str("\n[storage]\n");
        let src = self
            .sources
            .get("storage.specs_dir")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  specs_dir = {:?} (source: {})\n",
            self.config.storage.specs_dir, src
        ));
        let src = self
            .sources
            .get("storage.state_dir")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  state_dir = {:?} (source: {})\n",
            self.config.storage.state_dir, src
        ));
        let src = self
            .sources
            .get("storage.cache_dir")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  cache_dir = {:?} (source: {})\n",
            self.config.storage.cache_dir, src
        ));

        if !self.config.modules.is_empty() {
            let src = self
                .sources
                .get("modules")
                .unwrap_or(&ConfigSource::Default);
            out.push_str(&format!("\n[[modules]] (source: {})\n", src));
            for m in &self.config.modules {
                out.push_str(&format!("  - id = {:?}, paths = {:?}", m.id, m.paths));
                if let Some(l) = m.layer {
                    out.push_str(&format!(", layer = {}", l));
                }
                if !m.may_depend_on.is_empty() {
                    out.push_str(&format!(", may_depend_on = {:?}", m.may_depend_on));
                }
                out.push('\n');
            }
        }

        out.push_str("\n[hygiene]\n");
        let src = self
            .sources
            .get("hygiene.enabled")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  enabled = {} (source: {})\n",
            self.config.hygiene.enabled, src
        ));
        let src = self
            .sources
            .get("hygiene.max_inline_comment_lines")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  max_inline_comment_lines = {} (source: {})\n",
            self.config.hygiene.max_inline_comment_lines, src
        ));
        if !self.config.hygiene.forbid_patterns.is_empty() {
            let src = self
                .sources
                .get("hygiene.forbid_patterns")
                .unwrap_or(&ConfigSource::Default);
            out.push_str(&format!(
                "  forbid_patterns = {:?} (source: {})\n",
                self.config.hygiene.forbid_patterns, src
            ));
        }
        if let Some(cit) = &self.config.hygiene.citation_pattern {
            let src = self
                .sources
                .get("hygiene.citation_pattern")
                .unwrap_or(&ConfigSource::Default);
            out.push_str(&format!(
                "  citation_pattern = {:?} (source: {})\n",
                cit, src
            ));
        }
        let src = self
            .sources
            .get("hygiene.languages")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  languages = {:?} (source: {})\n",
            self.config.hygiene.languages, src
        ));

        if !self.config.regulatory.require_rationale_for.is_empty()
            || self.config.regulatory.iec62304_class.is_some()
        {
            out.push_str("\n[regulatory]\n");
            if let Some(cls) = &self.config.regulatory.iec62304_class {
                let src = self
                    .sources
                    .get("regulatory.iec62304_class")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  iec62304_class = {:?} (source: {})\n", cls, src));
            }
            if !self.config.regulatory.require_rationale_for.is_empty() {
                let src = self
                    .sources
                    .get("regulatory.require_rationale_for")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!(
                    "  require_rationale_for = {:?} (source: {})\n",
                    self.config.regulatory.require_rationale_for, src
                ));
            }
        }

        if self.config.soup.audit_command.is_some()
            || self.config.soup.deny_command.is_some()
            || self.config.soup.sbom_command.is_some()
        {
            out.push_str("\n[soup]\n");
            if let Some(cmd) = &self.config.soup.audit_command {
                let src = self
                    .sources
                    .get("soup.audit_command")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  audit_command = {:?} (source: {})\n", cmd, src));
            }
            if let Some(cmd) = &self.config.soup.deny_command {
                let src = self
                    .sources
                    .get("soup.deny_command")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  deny_command = {:?} (source: {})\n", cmd, src));
            }
            if let Some(cmd) = &self.config.soup.sbom_command {
                let src = self
                    .sources
                    .get("soup.sbom_command")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  sbom_command = {:?} (source: {})\n", cmd, src));
            }
        }

        if self.config.models.specify.is_some()
            || self.config.models.develop.is_some()
            || self.config.models.review.is_some()
        {
            out.push_str("\n[models]\n");
            if let Some(m) = &self.config.models.specify {
                let src = self
                    .sources
                    .get("models.specify")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  specify = {:?} (source: {})\n", m, src));
            }
            if let Some(m) = &self.config.models.develop {
                let src = self
                    .sources
                    .get("models.develop")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  develop = {:?} (source: {})\n", m, src));
            }
            if let Some(m) = &self.config.models.review {
                let src = self
                    .sources
                    .get("models.review")
                    .unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  review = {:?} (source: {})\n", m, src));
            }
        }

        out.push_str("\n[commit_messages]\n");
        let src = self
            .sources
            .get("commit_messages.enabled")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  enabled = {} (source: {})\n",
            self.config.commit_messages.enabled, src
        ));
        let src = self
            .sources
            .get("commit_messages.format")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  format = {:?} (source: {})\n",
            self.config.commit_messages.format, src
        ));

        if !self.config.environment.is_empty() {
            out.push_str("\n[environment]\n");
            for (k, v) in &self.config.environment.variables {
                let key = format!("environment.{}", k);
                let src = self.sources.get(&key).unwrap_or(&ConfigSource::Default);
                out.push_str(&format!("  {} = {:?} (source: {})\n", k, v, src));
            }
        }

        if !self.config.gates.is_empty() {
            let src = self.sources.get("gates").unwrap_or(&ConfigSource::Default);
            out.push_str(&format!("\n[[gates]] (source: {})\n", src));
            for g in &self.config.gates {
                out.push_str(&format!("  - id = {:?}", g.id));
                if let Some(cmd) = &g.command {
                    out.push_str(&format!(", command = {:?}", cmd));
                }
                if let Some(t) = g.timeout_ms {
                    out.push_str(&format!(", timeout_ms = {}", t));
                }
                if !g.depends_on.is_empty() {
                    out.push_str(&format!(", depends_on = {:?}", g.depends_on));
                }
                if let Some(oa) = &g.output_adapter {
                    out.push_str(&format!(", output_adapter = {:?}", oa));
                }
                if !g.on_transition.is_empty() {
                    out.push_str(&format!(", on_transition = {:?}", g.on_transition));
                }
                if !g.verifies.is_empty() {
                    out.push_str(&format!(", verifies = {:?}", g.verifies));
                }
                if let Some(k) = &g.kind {
                    out.push_str(&format!(", kind = {:?}", k));
                }
                if let Some(m) = &g.metric {
                    out.push_str(&format!(", metric = {:?}", m));
                }
                if let Some(d) = &g.direction {
                    out.push_str(&format!(", direction = {:?}", d));
                }
                if let Some(s) = g.skip {
                    out.push_str(&format!(", skip = {}", s));
                }
                out.push('\n');
            }
        }

        out.push_str("\n[identity]\n");
        let src = self
            .sources
            .get("identity.developer_id")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  developer_id = {:?} (source: {})\n",
            self.config.identity.developer_id, src
        ));
        let src = self
            .sources
            .get("identity.teams")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  teams = {:?} (source: {})\n",
            self.config.identity.teams, src
        ));

        out.push_str("\n[preferences]\n");
        let src = self
            .sources
            .get("preferences.color")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  color = {} (source: {})\n",
            self.config.preferences.color, src
        ));
        let src = self
            .sources
            .get("preferences.default_format")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  default_format = {:?} (source: {})\n",
            self.config.preferences.default_format, src
        ));
        if let Some(ed) = &self.config.preferences.editor {
            let src = self
                .sources
                .get("preferences.editor")
                .unwrap_or(&ConfigSource::Default);
            out.push_str(&format!("  editor = {:?} (source: {})\n", ed, src));
        }

        out.push_str("\n[leases]\n");
        let src = self
            .sources
            .get("leases.stale_age_days")
            .unwrap_or(&ConfigSource::Default);
        out.push_str(&format!(
            "  stale_age_days = {} (source: {})\n",
            self.config.leases.stale_age_days, src
        ));

        out
    }
}
