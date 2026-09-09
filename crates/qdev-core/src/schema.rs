use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::errors::QdevError;

/// All 13 canonical entity kinds in qdev.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Prd,
    Requirement,
    Epic,
    Story,
    Adr,
    Hazard,
    Sprint,
    Release,
    #[serde(rename = "dw", alias = "deferred_work")]
    DeferredWork,
    Decision,
    Scratchpad,
    Soup,
    Evidence,
}

impl EntityKind {
    /// Returns the raw JSON Schema embedded at compile-time.
    pub fn schema_str(&self) -> &'static str {
        match self {
            EntityKind::Prd => include_str!("../schemas/prd.json"),
            EntityKind::Requirement => include_str!("../schemas/requirement.json"),
            EntityKind::Epic => include_str!("../schemas/epic.json"),
            EntityKind::Story => include_str!("../schemas/story.json"),
            EntityKind::Adr => include_str!("../schemas/adr.json"),
            EntityKind::Hazard => include_str!("../schemas/hazard.json"),
            EntityKind::Sprint => include_str!("../schemas/sprint.json"),
            EntityKind::Release => include_str!("../schemas/release.json"),
            EntityKind::DeferredWork => include_str!("../schemas/dw.json"),
            EntityKind::Decision => include_str!("../schemas/decision.json"),
            EntityKind::Scratchpad => include_str!("../schemas/scratchpad.json"),
            EntityKind::Soup => include_str!("../schemas/soup.json"),
            EntityKind::Evidence => include_str!("../schemas/evidence.json"),
        }
    }

    /// Returns the parsed JSON Schema as a `serde_json::Value`.
    pub fn schema_json(&self) -> serde_json::Value {
        serde_json::from_str(self.schema_str()).expect("embedded schema must be valid JSON")
    }

    /// Returns the pretty-printed JSON Schema string.
    pub fn pretty_schema_str(&self) -> String {
        serde_json::to_string_pretty(&self.schema_json())
            .expect("embedded schema must be serializable")
    }

    /// Returns the canonical slug for this entity kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            EntityKind::Prd => "prd",
            EntityKind::Requirement => "requirement",
            EntityKind::Epic => "epic",
            EntityKind::Story => "story",
            EntityKind::Adr => "adr",
            EntityKind::Hazard => "hazard",
            EntityKind::Sprint => "sprint",
            EntityKind::Release => "release",
            EntityKind::DeferredWork => "dw",
            EntityKind::Decision => "decision",
            EntityKind::Scratchpad => "scratchpad",
            EntityKind::Soup => "soup",
            EntityKind::Evidence => "evidence",
        }
    }

    /// Returns an array of all 13 entity kinds.
    pub const fn all() -> &'static [EntityKind; 13] {
        &[
            EntityKind::Prd,
            EntityKind::Requirement,
            EntityKind::Epic,
            EntityKind::Story,
            EntityKind::Adr,
            EntityKind::Hazard,
            EntityKind::Sprint,
            EntityKind::Release,
            EntityKind::DeferredWork,
            EntityKind::Decision,
            EntityKind::Scratchpad,
            EntityKind::Soup,
            EntityKind::Evidence,
        ]
    }

    /// Resolves kind name and aliases loosely (case-insensitive, hyphens/underscores, plurals).
    pub fn from_str_loose(s: &str) -> Result<EntityKind, QdevError> {
        let normalized = s.trim().to_lowercase().replace('-', "_");
        match normalized.as_str() {
            "prd" | "prds" => Ok(EntityKind::Prd),
            "requirement" | "requirements" | "req" | "reqs" | "fr" | "nfr" => {
                Ok(EntityKind::Requirement)
            }
            "epic" | "epics" => Ok(EntityKind::Epic),
            "story" | "stories" => Ok(EntityKind::Story),
            "adr" | "adrs" => Ok(EntityKind::Adr),
            "hazard" | "hazards" | "haz" => Ok(EntityKind::Hazard),
            "sprint" | "sprints" => Ok(EntityKind::Sprint),
            "release" | "releases" => Ok(EntityKind::Release),
            "dw" | "deferred_work" | "deferred_works" => Ok(EntityKind::DeferredWork),
            "decision" | "decisions" | "dec" | "decs" => Ok(EntityKind::Decision),
            "scratchpad" | "scratchpad_entry" | "scratchpads" | "scratch"
            | "scratchpad_entries" => Ok(EntityKind::Scratchpad),
            "soup" | "soup_dependency" | "soups" | "soup_dependencies" => Ok(EntityKind::Soup),
            "evidence" | "evidence_record" | "evidence_records" | "gate_run" | "gate_runs"
            | "evidences" => Ok(EntityKind::Evidence),
            _ => Err(QdevError::usage_error(format!(
                "Unknown schema kind '{}'. Valid schema kinds: prd, requirement, epic, story, adr, hazard, sprint, release, deferred_work (dw), decision, scratchpad, soup, evidence",
                s
            ))),
        }
    }
}

impl FromStr for EntityKind {
    type Err = QdevError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EntityKind::from_str_loose(s)
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// The CLI output *payload* kinds with a live command today (Story 1.13): `qdev schema payload
/// <name>` prints a hand-authored JSON Schema for a command's `--json` output envelope, distinct
/// from `EntityKind`'s frontmatter schemas (`qdev schema <entity-kind>`, Story 1.4). Kept fully
/// separate from `EntityKind` — never resolved by, or resolving via, `EntityKind::from_str_loose`.
///
/// Every `--json` payload with a live command has a schema here — the invariant is that a
/// shipped payload is either schematized or recorded in `deferred-work.md`, never neither.
/// `context`, `next`, and `gate_run`-as-a-payload have no live command yet and are deferred
/// (Epic 2/3 scope); they are intentionally absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    Story,
    Error,
    Validate,
    FixIds,
    List,
    Sync,
    Doctor,
}

impl PayloadKind {
    /// Returns the raw JSON Schema embedded at compile-time.
    pub fn schema_str(&self) -> &'static str {
        match self {
            PayloadKind::Story => include_str!("../schemas/payload-story.json"),
            PayloadKind::Error => include_str!("../schemas/payload-error.json"),
            PayloadKind::Validate => include_str!("../schemas/payload-validate.json"),
            PayloadKind::FixIds => include_str!("../schemas/payload-fix-ids.json"),
            PayloadKind::List => include_str!("../schemas/payload-list.json"),
            PayloadKind::Sync => include_str!("../schemas/payload-sync.json"),
            PayloadKind::Doctor => include_str!("../schemas/payload-doctor.json"),
        }
    }

    /// Returns the parsed JSON Schema as a `serde_json::Value`.
    pub fn schema_json(&self) -> serde_json::Value {
        serde_json::from_str(self.schema_str()).expect("embedded payload schema must be valid JSON")
    }

    /// Returns the pretty-printed JSON Schema string.
    pub fn pretty_schema_str(&self) -> String {
        serde_json::to_string_pretty(&self.schema_json())
            .expect("embedded payload schema must be serializable")
    }

    /// Returns the canonical slug for this payload kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            PayloadKind::Story => "story",
            PayloadKind::Error => "error",
            PayloadKind::Validate => "validate",
            PayloadKind::FixIds => "fix_ids",
            PayloadKind::List => "list",
            PayloadKind::Sync => "sync",
            PayloadKind::Doctor => "doctor",
        }
    }

    /// Returns an array of all currently-supported payload kinds.
    pub const fn all() -> &'static [PayloadKind; 7] {
        &[
            PayloadKind::Story,
            PayloadKind::Error,
            PayloadKind::Validate,
            PayloadKind::FixIds,
            PayloadKind::List,
            PayloadKind::Sync,
            PayloadKind::Doctor,
        ]
    }

    /// Comma-separated canonical slugs of every supported payload kind, for usage-error messages.
    pub fn valid_names() -> String {
        PayloadKind::all()
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Resolves a payload name loosely (case-insensitive, hyphens/underscores). Names deferred to
    /// a future story (`context`, `next`, `gate_run`) are reported as unknown, same as any other
    /// unrecognized name, until their command ships.
    pub fn from_str_loose(s: &str) -> Result<PayloadKind, QdevError> {
        let normalized = s.trim().to_lowercase().replace('-', "_");
        match normalized.as_str() {
            "story" | "stories" => Ok(PayloadKind::Story),
            "error" | "errors" => Ok(PayloadKind::Error),
            "validate" | "validation" => Ok(PayloadKind::Validate),
            "fix_ids" | "fixids" => Ok(PayloadKind::FixIds),
            "list" => Ok(PayloadKind::List),
            "sync" => Ok(PayloadKind::Sync),
            "doctor" => Ok(PayloadKind::Doctor),
            _ => Err(QdevError::usage_error(format!(
                "Unknown payload name '{}'. Valid payload names: {}",
                s,
                PayloadKind::valid_names()
            ))),
        }
    }
}

impl FromStr for PayloadKind {
    type Err = QdevError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        PayloadKind::from_str_loose(s)
    }
}

impl fmt::Display for PayloadKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Errors occurring during frontmatter extraction or schema handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaError {
    MissingDelimiters,
    UnclosedFrontmatter,
    YamlParseError(String),
    InvalidFrontmatterType(String),
    SchemaCompilationError(String),
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SchemaError::MissingDelimiters => {
                write!(f, "Missing opening frontmatter delimiter '---'")
            }
            SchemaError::UnclosedFrontmatter => {
                write!(f, "Unclosed frontmatter delimiter '---'")
            }
            SchemaError::YamlParseError(msg) => write!(f, "YAML parse error: {}", msg),
            SchemaError::InvalidFrontmatterType(msg) => {
                write!(f, "Invalid frontmatter: {}", msg)
            }
            SchemaError::SchemaCompilationError(msg) => {
                write!(f, "JSON Schema compilation error: {}", msg)
            }
        }
    }
}

impl std::error::Error for SchemaError {}

/// Detailed schema validation finding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Extracts the raw YAML frontmatter substring and markdown body substring from markdown content.
pub fn extract_frontmatter_str(content: &str) -> Result<(&str, &str), SchemaError> {
    let trimmed_start = content.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if !trimmed_start.starts_with("---") {
        return Err(SchemaError::MissingDelimiters);
    }

    let after_opening = &trimmed_start[3..];
    let rest = if let Some(after_newline) = after_opening.strip_prefix("\r\n") {
        after_newline
    } else if let Some(after_newline) = after_opening.strip_prefix('\n') {
        after_newline
    } else if after_opening.starts_with([' ', '\t']) {
        let line_end = after_opening
            .find('\n')
            .ok_or(SchemaError::UnclosedFrontmatter)?;
        if !after_opening[..line_end].trim().is_empty() {
            return Err(SchemaError::MissingDelimiters);
        }
        &after_opening[line_end + 1..]
    } else {
        return Err(SchemaError::MissingDelimiters);
    };

    let mut current_pos = 0;
    for line in rest.lines() {
        let line_trimmed = line.trim_end();
        if line_trimmed == "---" || line_trimmed == "..." {
            let frontmatter = &rest[..current_pos];
            let after_delimiter = current_pos + line.len();
            let body = if after_delimiter < rest.len() {
                let remainder = &rest[after_delimiter..];
                remainder
                    .strip_prefix("\r\n")
                    .or_else(|| remainder.strip_prefix('\n'))
                    .unwrap_or(remainder)
            } else {
                ""
            };
            return Ok((frontmatter, body));
        }
        current_pos += line.len();
        if rest[current_pos..].starts_with("\r\n") {
            current_pos += 2;
        } else if rest[current_pos..].starts_with('\n') {
            current_pos += 1;
        }
    }

    Err(SchemaError::UnclosedFrontmatter)
}

/// Extracts and parses frontmatter from markdown content (or raw YAML) into a `serde_json::Value`.
pub fn extract_frontmatter(content: &str) -> Result<serde_json::Value, SchemaError> {
    let trimmed = content.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    let yaml_str = if trimmed.starts_with("---") {
        let (fm, _) = extract_frontmatter_str(content)?;
        fm
    } else {
        content
    };

    let value: serde_json::Value =
        serde_yaml::from_str(yaml_str).map_err(|e| SchemaError::YamlParseError(e.to_string()))?;

    if !value.is_object() {
        return Err(SchemaError::InvalidFrontmatterType(
            "frontmatter must be a YAML mapping (object)".to_string(),
        ));
    }

    Ok(value)
}

/// Validates a parsed JSON frontmatter value against the schema for the given entity kind,
/// returning structured `ValidationError` items.
pub fn validate_value_detailed(
    kind: EntityKind,
    value: &serde_json::Value,
) -> Result<(), Vec<ValidationError>> {
    let schema_json = kind.schema_json();
    let validator = jsonschema::validator_for(&schema_json).map_err(|e| {
        vec![ValidationError {
            path: String::new(),
            message: format!("JSON schema compilation error: {}", e),
        }]
    })?;

    let mut errors = Vec::new();
    for error in validator.iter_errors(value) {
        let path = error.instance_path().to_string();
        errors.push(ValidationError {
            path,
            message: error.to_string(),
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Validates a parsed JSON frontmatter value against the schema for the given entity kind,
/// returning string error messages.
pub fn validate_frontmatter_value(
    kind: EntityKind,
    value: &serde_json::Value,
) -> Result<(), Vec<String>> {
    validate_value_detailed(kind, value)
        .map_err(|errs| errs.into_iter().map(|e| e.to_string()).collect())
}

/// Validates a markdown document or raw frontmatter against the schema for the given entity kind,
/// returning structured `ValidationError` items.
pub fn validate_frontmatter_detailed(
    kind: EntityKind,
    content: &str,
) -> Result<(), Vec<ValidationError>> {
    let value = match extract_frontmatter(content) {
        Ok(v) => v,
        Err(e) => {
            return Err(vec![ValidationError {
                path: String::new(),
                message: e.to_string(),
            }])
        }
    };
    validate_value_detailed(kind, &value)
}

/// Validates a markdown document or raw frontmatter against the schema for the given entity kind,
/// returning string error messages.
pub fn validate_frontmatter(kind: EntityKind, content: &str) -> Result<(), Vec<String>> {
    validate_frontmatter_detailed(kind, content)
        .map_err(|errs| errs.into_iter().map(|e| e.to_string()).collect())
}
