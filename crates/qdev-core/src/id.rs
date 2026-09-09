use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;

/// Canonical classification kind for identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentifierKind {
    Epic,
    Story,
    Adr,
    FunctionalRequirement,
    NonFunctionalRequirement,
    Hazard,
    Prd,
    DeferredWork,
    Decision,
    Constraint,
}

impl IdentifierKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            IdentifierKind::Epic => "epic",
            IdentifierKind::Story => "story",
            IdentifierKind::Adr => "adr",
            IdentifierKind::FunctionalRequirement => "functional_requirement",
            IdentifierKind::NonFunctionalRequirement => "non_functional_requirement",
            IdentifierKind::Hazard => "hazard",
            IdentifierKind::Prd => "prd",
            IdentifierKind::DeferredWork => "deferred_work",
            IdentifierKind::Decision => "decision",
            IdentifierKind::Constraint => "constraint",
        }
    }
}

impl fmt::Display for IdentifierKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Owning entity for a negative or rabbit-hole constraint.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConstraintOwner {
    Epic(u32),
    Story(u32, u32),
}

impl ConstraintOwner {
    pub fn epic_number(&self) -> u32 {
        match self {
            ConstraintOwner::Epic(n) => *n,
            ConstraintOwner::Story(e, _) => *e,
        }
    }

    pub fn story_number(&self) -> Option<u32> {
        match self {
            ConstraintOwner::Epic(_) => None,
            ConstraintOwner::Story(_, s) => Some(*s),
        }
    }
}

impl fmt::Display for ConstraintOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConstraintOwner::Epic(n) => write!(f, "E{}", n),
            ConstraintOwner::Story(e, s) => write!(f, "E{}S{}", e, s),
        }
    }
}

impl Serialize for ConstraintOwner {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ConstraintOwner {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.parse::<Identifier>() {
            Ok(Identifier::Epic { number }) => Ok(ConstraintOwner::Epic(number)),
            Ok(Identifier::Story { epic, story }) => Ok(ConstraintOwner::Story(epic, story)),
            _ => Err(serde::de::Error::custom(format!(
                "invalid constraint owner '{}', expected Epic (e.g. E12) or Story (e.g. E12S4)",
                s
            ))),
        }
    }
}

/// Kind of constraint (NoGo or RabbitHole).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    NoGo,
    RabbitHole,
}

impl ConstraintKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConstraintKind::NoGo => "NG",
            ConstraintKind::RabbitHole => "RH",
        }
    }
}

impl fmt::Display for ConstraintKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Strongly typed entity identifier accepting all 10 canonical forms.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Identifier {
    Epic {
        number: u32,
    },
    Story {
        epic: u32,
        story: u32,
    },
    Adr {
        number: u32,
    },
    FunctionalRequirement {
        number: u32,
    },
    NonFunctionalRequirement {
        number: u32,
    },
    Hazard {
        number: u32,
    },
    Prd {
        number: u32,
    },
    DeferredWork {
        hash: String,
    },
    Decision {
        hash: String,
    },
    Constraint {
        owner: ConstraintOwner,
        kind: ConstraintKind,
        number: u32,
    },
}

impl Identifier {
    /// Returns the classification kind for this identifier.
    pub fn kind(&self) -> IdentifierKind {
        match self {
            Identifier::Epic { .. } => IdentifierKind::Epic,
            Identifier::Story { .. } => IdentifierKind::Story,
            Identifier::Adr { .. } => IdentifierKind::Adr,
            Identifier::FunctionalRequirement { .. } => IdentifierKind::FunctionalRequirement,
            Identifier::NonFunctionalRequirement { .. } => IdentifierKind::NonFunctionalRequirement,
            Identifier::Hazard { .. } => IdentifierKind::Hazard,
            Identifier::Prd { .. } => IdentifierKind::Prd,
            Identifier::DeferredWork { .. } => IdentifierKind::DeferredWork,
            Identifier::Decision { .. } => IdentifierKind::Decision,
            Identifier::Constraint { .. } => IdentifierKind::Constraint,
        }
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Identifier::Epic { number } => write!(f, "E{}", number),
            Identifier::Story { epic, story } => write!(f, "E{}S{}", epic, story),
            Identifier::Adr { number } => write!(f, "AD-{}", number),
            Identifier::FunctionalRequirement { number } => write!(f, "FR-{}", number),
            Identifier::NonFunctionalRequirement { number } => write!(f, "NFR-{}", number),
            Identifier::Hazard { number } => write!(f, "HAZ-{}", number),
            Identifier::Prd { number } => write!(f, "PRD-{}", number),
            Identifier::DeferredWork { hash } => write!(f, "DW-{}", hash),
            Identifier::Decision { hash } => write!(f, "DEC-{}", hash),
            Identifier::Constraint {
                owner,
                kind,
                number,
            } => write!(f, "{}/{}-{}", owner, kind, number),
        }
    }
}

impl Serialize for Identifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Identifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<Identifier>().map_err(serde::de::Error::custom)
    }
}

/// Errors occurring during identifier grammar parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdParseError {
    Empty,
    SprintPrefixed(String),
    InvalidPrefix(String),
    NonPositiveInteger(String),
    InvalidHex {
        prefix: String,
        hash: String,
        reason: String,
    },
    InvalidConstraint(String),
    InvalidFormat(String),
}

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdParseError::Empty => write!(f, "Identifier string cannot be empty"),
            IdParseError::SprintPrefixed(s) => write!(
                f,
                "Sprint-prefixed identifier '{}' is rejected; identifiers must be sprint-independent per AD-7",
                s
            ),
            IdParseError::InvalidPrefix(s) => {
                write!(f, "Unrecognized identifier prefix in '{}'", s)
            }
            IdParseError::NonPositiveInteger(s) => {
                write!(
                    f,
                    "Identifier numbers must be positive integers (n >= 1), got '{}'",
                    s
                )
            }
            IdParseError::InvalidHex {
                prefix,
                hash,
                reason,
            } => {
                write!(f, "Invalid hex identifier '{}-{}': {}", prefix, hash, reason)
            }
            IdParseError::InvalidConstraint(s) => {
                write!(f, "Invalid constraint identifier '{}'", s)
            }
            IdParseError::InvalidFormat(s) => write!(f, "Invalid identifier format: '{}'", s),
        }
    }
}

impl std::error::Error for IdParseError {}

impl From<IdParseError> for QdevError {
    fn from(err: IdParseError) -> Self {
        QdevError::usage_error(err.to_string())
    }
}

fn is_sprint_prefixed(s: &str) -> bool {
    s.starts_with('S') && s.len() > 1 && s.as_bytes()[1].is_ascii_digit()
}

fn parse_positive_int(num_str: &str, full_str: &str) -> Result<u32, IdParseError> {
    if num_str.is_empty() {
        return Err(IdParseError::InvalidFormat(full_str.to_string()));
    }
    if !num_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(IdParseError::InvalidFormat(full_str.to_string()));
    }
    if num_str == "0" {
        return Err(IdParseError::NonPositiveInteger(full_str.to_string()));
    }
    if num_str.starts_with('0') && num_str.len() > 1 {
        return Err(IdParseError::InvalidFormat(full_str.to_string()));
    }
    match num_str.parse::<u32>() {
        Ok(0) => Err(IdParseError::NonPositiveInteger(full_str.to_string())),
        Ok(n) => Ok(n),
        Err(_) => Err(IdParseError::InvalidFormat(full_str.to_string())),
    }
}

fn validate_hex_hash(prefix: &str, hash: &str) -> Result<(), IdParseError> {
    if hash.len() < 4 {
        return Err(IdParseError::InvalidHex {
            prefix: prefix.to_string(),
            hash: hash.to_string(),
            reason: "hex hash must be at least 4 characters".to_string(),
        });
    }
    if !hash
        .chars()
        .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(IdParseError::InvalidHex {
            prefix: prefix.to_string(),
            hash: hash.to_string(),
            reason: "hex hash must contain only lowercase hexadecimal characters (0-9, a-f)"
                .to_string(),
        });
    }
    Ok(())
}

fn parse_epic_or_story(rest: &str, full_str: &str) -> Result<Identifier, IdParseError> {
    if let Some((epic_str, story_str)) = rest.split_once('S') {
        let epic = parse_positive_int(epic_str, full_str)?;
        let story = parse_positive_int(story_str, full_str)?;
        Ok(Identifier::Story { epic, story })
    } else {
        let number = parse_positive_int(rest, full_str)?;
        Ok(Identifier::Epic { number })
    }
}

fn parse_constraint(
    full_str: &str,
    owner_str: &str,
    suffix_str: &str,
) -> Result<Identifier, IdParseError> {
    if is_sprint_prefixed(owner_str) {
        return Err(IdParseError::SprintPrefixed(full_str.to_string()));
    }

    let owner = if let Some(rest) = owner_str.strip_prefix('E') {
        match parse_epic_or_story(rest, full_str)? {
            Identifier::Epic { number } => ConstraintOwner::Epic(number),
            Identifier::Story { epic, story } => ConstraintOwner::Story(epic, story),
            _ => return Err(IdParseError::InvalidConstraint(full_str.to_string())),
        }
    } else {
        return Err(IdParseError::InvalidConstraint(full_str.to_string()));
    };

    if let Some((kind_str, num_str)) = suffix_str.split_once('-') {
        let kind = match kind_str {
            "NG" => ConstraintKind::NoGo,
            "RH" => ConstraintKind::RabbitHole,
            _ => return Err(IdParseError::InvalidConstraint(full_str.to_string())),
        };
        let number = parse_positive_int(num_str, full_str)?;
        Ok(Identifier::Constraint {
            owner,
            kind,
            number,
        })
    } else {
        Err(IdParseError::InvalidConstraint(full_str.to_string()))
    }
}

impl FromStr for Identifier {
    type Err = IdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(IdParseError::Empty);
        }

        // Strictly reject sprint-prefixed identifiers (e.g. S5E2, S5E2S4, S1E2) per AD-7
        if is_sprint_prefixed(s) {
            return Err(IdParseError::SprintPrefixed(s.to_string()));
        }

        // Check for constraint format: {owner}/NG-{k} or {owner}/RH-{k}
        if let Some((owner_str, constraint_suffix)) = s.split_once('/') {
            return parse_constraint(s, owner_str, constraint_suffix);
        }

        if let Some(rest) = s.strip_prefix("AD-") {
            let number = parse_positive_int(rest, s)?;
            return Ok(Identifier::Adr { number });
        }
        if let Some(rest) = s.strip_prefix("FR-") {
            let number = parse_positive_int(rest, s)?;
            return Ok(Identifier::FunctionalRequirement { number });
        }
        if let Some(rest) = s.strip_prefix("NFR-") {
            let number = parse_positive_int(rest, s)?;
            return Ok(Identifier::NonFunctionalRequirement { number });
        }
        if let Some(rest) = s.strip_prefix("HAZ-") {
            let number = parse_positive_int(rest, s)?;
            return Ok(Identifier::Hazard { number });
        }
        if let Some(rest) = s.strip_prefix("PRD-") {
            let number = parse_positive_int(rest, s)?;
            return Ok(Identifier::Prd { number });
        }
        if let Some(rest) = s.strip_prefix("DW-") {
            validate_hex_hash("DW", rest)?;
            return Ok(Identifier::DeferredWork {
                hash: rest.to_string(),
            });
        }
        if let Some(rest) = s.strip_prefix("DEC-") {
            validate_hex_hash("DEC", rest)?;
            return Ok(Identifier::Decision {
                hash: rest.to_string(),
            });
        }

        if let Some(rest) = s.strip_prefix('E') {
            return parse_epic_or_story(rest, s);
        }

        Err(IdParseError::InvalidPrefix(s.to_string()))
    }
}

/// Allocates the next sequential story identifier for a given epic by scanning the configured
/// stories directory directly on disk. Never reads the SQLite cache per AD-7.
/// Takes `max(existing) + 1` (or 1 if none exist).
///
/// Uses the default `[storage]` layout; call `allocate_next_story_id_in` to honour a configured
/// `specs_dir`. Allocating against a directory the sweep does not scan restarts numbering at 1
/// on every invocation, silently minting duplicate ids.
pub fn allocate_next_story_id(
    workspace_root: &Path,
    epic_number: u32,
) -> Result<Identifier, QdevError> {
    allocate_next_story_id_in(workspace_root, &StorageConfig::default(), epic_number)
}

/// `allocate_next_story_id`, scanning `storage.specs_dir` rather than the default layout.
pub fn allocate_next_story_id_in(
    workspace_root: &Path,
    storage: &StorageConfig,
    epic_number: u32,
) -> Result<Identifier, QdevError> {
    if epic_number == 0 {
        return Err(QdevError::usage_error(
            "Epic number must be a positive integer (n >= 1)",
        ));
    }

    let stories_dir = workspace_root.join(&storage.specs_dir).join("stories");
    if !stories_dir.exists() {
        return Ok(Identifier::Story {
            epic: epic_number,
            story: 1,
        });
    }

    let entries = std::fs::read_dir(&stories_dir).map_err(|e| {
        QdevError::infrastructure_failure(
            "io_error",
            format!(
                "Failed to read stories directory '{}': {}",
                stories_dir.display(),
                e
            ),
        )
    })?;

    let mut max_story = 0u32;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
            continue;
        }

        let file_name = entry.file_name();
        let file_name_str = match file_name.to_str() {
            Some(s) => s,
            None => continue,
        };

        if let Some(stem) = file_name_str.strip_suffix(".md") {
            if let Ok(Identifier::Story { epic, story }) = stem.parse::<Identifier>() {
                if epic == epic_number && story > max_story {
                    max_story = story;
                }
            } else {
                let candidate = stem.split(['-', '_']).next().unwrap_or(stem);
                if let Ok(Identifier::Story { epic, story }) = candidate.parse::<Identifier>() {
                    if epic == epic_number && story > max_story {
                        max_story = story;
                    }
                } else if let Some((epic, story)) = parse_story_filename_lenient(candidate) {
                    if epic == epic_number && story > max_story {
                        max_story = story;
                    }
                }
            }
        }
    }

    let next_story = max_story
        .checked_add(1)
        .ok_or_else(|| QdevError::usage_error("Story ID sequence overflow"))?;

    Ok(Identifier::Story {
        epic: epic_number,
        story: next_story,
    })
}

fn parse_story_filename_lenient(s: &str) -> Option<(u32, u32)> {
    let rest = s.strip_prefix('E')?;
    let (epic_str, story_str) = rest.split_once('S')?;
    let epic_cleaned = epic_str.trim_start_matches('0');
    let story_cleaned = story_str.trim_start_matches('0');
    let epic = if epic_cleaned.is_empty() {
        0
    } else {
        epic_cleaned.parse::<u32>().ok()?
    };
    let story = if story_cleaned.is_empty() {
        0
    } else {
        story_cleaned.parse::<u32>().ok()?
    };
    if epic > 0 && story > 0 {
        Some((epic, story))
    } else {
        None
    }
}

fn generate_random_hex<R: rand::Rng>(rng: &mut R, count: usize) -> String {
    let mut s = String::with_capacity(count);
    for _ in 0..count {
        let b: u8 = rng.random();
        let hex_digit = b"0123456789abcdef"[(b & 0x0f) as usize] as char;
        s.push(hex_digit);
    }
    s
}

fn hex_id_collides(dir: &Path, prefix: &str, hash: &str) -> bool {
    let exact = dir.join(format!("{}-{}.md", prefix, hash));
    if exact.exists() {
        return true;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        let prefix_dash = format!("{}-{}-", prefix, hash);
        let prefix_underscore = format!("{}-{}_", prefix, hash);
        let prefix_dot = format!("{}-{}.", prefix, hash);
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(&prefix_dash)
                    || name.starts_with(&prefix_underscore)
                    || name.starts_with(&prefix_dot)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn allocate_hex_id_with_rng<R: rand::Rng>(
    workspace_root: &Path,
    rel_dir: &str,
    prefix: &str,
    rng: &mut R,
) -> String {
    let dir = workspace_root.join(rel_dir);

    // 1. Generate 4 random hex characters
    let mut hash = generate_random_hex(rng, 4);
    if !hex_id_collides(&dir, prefix, &hash) {
        return hash;
    }

    // 2. Extend to 6 if a file with that ID already exists
    hash.push_str(&generate_random_hex(rng, 2));
    if !hex_id_collides(&dir, prefix, &hash) {
        return hash;
    }

    // 3. Extend to 8 if a collision persists
    hash.push_str(&generate_random_hex(rng, 2));
    if !hex_id_collides(&dir, prefix, &hash) {
        return hash;
    }

    // If collision persists after 8 chars, retry generation up to a bounded limit
    const MAX_RETRIES: usize = 100;
    for _ in 0..MAX_RETRIES {
        let retry_hash = generate_random_hex(rng, 8);
        if !hex_id_collides(&dir, prefix, &retry_hash) {
            return retry_hash;
        }
    }

    hash
}

/// Allocates a new Deferred Work identifier (`DW-{hex4+}`) using the provided random generator.
/// Generates 4 random hex characters, extending to 6 and 8 on file collisions under `docs/state/dw/`.
pub fn allocate_deferred_work_id_with_rng<R: rand::Rng>(
    workspace_root: &Path,
    rng: &mut R,
) -> Identifier {
    allocate_deferred_work_id_in_with_rng(workspace_root, &StorageConfig::default(), rng)
}

/// `allocate_deferred_work_id_with_rng`, scanning `storage.state_dir` rather than the default.
pub fn allocate_deferred_work_id_in_with_rng<R: rand::Rng>(
    workspace_root: &Path,
    storage: &StorageConfig,
    rng: &mut R,
) -> Identifier {
    let hash = allocate_hex_id_with_rng(
        workspace_root,
        &format!("{}/dw", storage.state_dir),
        "DW",
        rng,
    );
    Identifier::DeferredWork { hash }
}

/// Allocates a new Deferred Work identifier (`DW-{hex4+}`).
/// Generates 4 random hex characters, extending to 6 and 8 on file collisions under `docs/state/dw/`.
pub fn allocate_deferred_work_id(workspace_root: &Path) -> Identifier {
    let mut rng = rand::rng();
    allocate_deferred_work_id_with_rng(workspace_root, &mut rng)
}

/// Allocates a new Decision identifier (`DEC-{hex4+}`) using the provided random generator.
/// Generates 4 random hex characters, extending to 6 and 8 on file collisions under `docs/state/decisions/`.
pub fn allocate_decision_id_with_rng<R: rand::Rng>(
    workspace_root: &Path,
    rng: &mut R,
) -> Identifier {
    allocate_decision_id_in_with_rng(workspace_root, &StorageConfig::default(), rng)
}

/// `allocate_decision_id_with_rng`, scanning `storage.state_dir` rather than the default.
pub fn allocate_decision_id_in_with_rng<R: rand::Rng>(
    workspace_root: &Path,
    storage: &StorageConfig,
    rng: &mut R,
) -> Identifier {
    let hash = allocate_hex_id_with_rng(
        workspace_root,
        &format!("{}/decisions", storage.state_dir),
        "DEC",
        rng,
    );
    Identifier::Decision { hash }
}

/// Allocates a new Decision identifier (`DEC-{hex4+}`).
/// Generates 4 random hex characters, extending to 6 and 8 on file collisions under `docs/state/decisions/`.
pub fn allocate_decision_id(workspace_root: &Path) -> Identifier {
    let mut rng = rand::rng();
    allocate_decision_id_with_rng(workspace_root, &mut rng)
}
