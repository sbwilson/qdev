//! Read-only query engine backing `qdev get` and `qdev list`.
//!
//! All reads come from the SQLite cache (`Store` trait methods) — nothing here re-parses
//! Markdown. Every serialized collection uses deterministically-ordered structures (`Vec`, or
//! key-sorted `BTreeMap`) so JSON output is byte-identical across runs for identical cache
//! state.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::errors::QdevError;
use crate::schema::EntityKind;
use crate::store::{
    ConstraintRecord, EntityFilter, EntityRecord, RelationRecord, ScratchpadRecord, Store,
};

/// A single constraint attached to (or inherited by) an entity, as returned in a `get` payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConstraintProjection {
    pub id: String,
    pub kind: String,
    pub text: String,
    /// Set to the owning epic's id when this constraint was inherited rather than declared
    /// directly on the queried entity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherited_from: Option<String>,
}

/// One scratchpad ledger entry, included only when `--expand scratch` is requested.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScratchEntryProjection {
    pub seq: u32,
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl From<ScratchpadRecord> for ScratchEntryProjection {
    fn from(r: ScratchpadRecord) -> Self {
        Self {
            seq: r.seq,
            at: r.at,
            author_type: r.author_type,
            author_id: r.author_id,
            kind: r.kind,
            text: r.text,
        }
    }
}

/// Isolated single-entity projection returned by `qdev get`.
///
/// The default projection (no `--expand`) always carries `constraints` (own + inherited),
/// `relations` (grouped by relation name), and computed `blocked`. `--expand scratch` adds
/// `scratch`; nothing else changes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EntityProjection {
    pub id: String,
    pub kind: EntityKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Stories only: true if any `depends_on` target's cached status isn't `done`. Always
    /// `false` for kinds that never carry a `depends_on` relation.
    pub blocked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appetite: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety_class: Option<String>,
    pub owners: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_modules: Option<Vec<String>>,
    pub constraints: Vec<ConstraintProjection>,
    pub relations: BTreeMap<String, Vec<String>>,
    pub version: u64,
    /// True when the cached row is stale: the source file currently fails to parse (schema
    /// violation, merge conflict, etc.) and this is its last-known-good hydrated state.
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scratch: Option<Vec<ScratchEntryProjection>>,
}

/// Result of resolving a `qdev get` target: either a full entity projection, or — when the
/// resolved id contains `/` — the raw constraint row it names.
#[derive(Debug, Clone, PartialEq)]
pub enum GetResult {
    Entity(Box<EntityProjection>),
    Constraint(ConstraintRecord),
}

/// Options controlling what `query_entity` includes beyond the always-on default projection.
#[derive(Debug, Clone, Copy, Default)]
pub struct QueryOptions {
    /// `--expand scratch`: append the entity's scratchpad ledger.
    pub expand_scratch: bool,
}

/// Parses a cache column storing a JSON array of strings (e.g. `owners`, `target_modules`)
/// into a `Vec<String>`. Absent or unparseable columns yield an empty vector.
fn parse_json_string_array(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

/// Groups relation rows (already ordered by relation, target_id) into an insertion-ordered map
/// keyed by relation name, preserving the store's target ordering within each group.
fn group_relations(rows: &[RelationRecord]) -> BTreeMap<String, Vec<String>> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        grouped
            .entry(row.relation.clone())
            .or_default()
            .push(row.target_id.clone());
    }
    grouped
}

/// Computes `blocked` per the frozen spec: true if any `depends_on` target's cached status
/// isn't `done` (including a dangling target with no cached row). No cycle traversal.
///
/// Stories only, matching the documented invariant and `payload-story.json`. `depends_on` is a
/// Story -> Story relation, but hydration only *records* an out-of-band edge on another kind as
/// an `invalid_relation_kind` finding — it keeps the row — so without this gate an epic
/// carrying a stray `depends_on` would report `blocked: true` and contradict the payload schema
/// every consumer reads.
fn compute_blocked(
    store: &dyn Store,
    kind: EntityKind,
    relation_rows: &[RelationRecord],
) -> Result<bool, QdevError> {
    if kind != EntityKind::Story {
        return Ok(false);
    }
    for row in relation_rows {
        if row.relation == "depends_on" {
            let target_status = store.get_entity(&row.target_id)?.and_then(|e| e.status);
            if target_status.as_deref() != Some("done") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Builds the full isolated projection for a resolved entity record: own + inherited
/// constraints, grouped relations, computed `blocked`, and (optionally) the scratchpad ledger.
fn build_entity_projection(
    store: &dyn Store,
    entity: &EntityRecord,
    options: &QueryOptions,
) -> Result<EntityProjection, QdevError> {
    let owners = parse_json_string_array(entity.owners.as_deref());
    let target_modules = if entity.kind == EntityKind::Story {
        Some(parse_json_string_array(entity.target_modules.as_deref()))
    } else {
        None
    };

    // Own constraints first, then constraints inherited from the owning epic (stories only),
    // each tagged with `inherited_from`. `get_constraints_for_owner` already orders by id.
    let mut constraints: Vec<ConstraintProjection> = store
        .get_constraints_for_owner(&entity.id)?
        .into_iter()
        .map(|c| ConstraintProjection {
            id: c.id,
            kind: c.kind,
            text: c.text,
            inherited_from: None,
        })
        .collect();

    if entity.kind == EntityKind::Story {
        if let Some(epic_id) = entity.epic_id.as_deref().filter(|e| !e.is_empty()) {
            let inherited = store.get_constraints_for_owner(epic_id)?;
            constraints.extend(inherited.into_iter().map(|c| ConstraintProjection {
                id: c.id,
                kind: c.kind,
                text: c.text,
                inherited_from: Some(epic_id.to_string()),
            }));
        }
    }

    let relation_rows = store.get_relations_for_source(&entity.id)?;
    let relations = group_relations(&relation_rows);
    let blocked = compute_blocked(store, entity.kind, &relation_rows)?;

    let scratch = if options.expand_scratch {
        let entries = store.get_scratchpad_entries(&entity.id)?;
        Some(
            entries
                .into_iter()
                .map(ScratchEntryProjection::from)
                .collect(),
        )
    } else {
        None
    };

    Ok(EntityProjection {
        id: entity.id.clone(),
        kind: entity.kind,
        epic_id: entity.epic_id.clone(),
        title: entity.title.clone(),
        status: entity.status.clone(),
        blocked,
        appetite: entity.appetite.clone(),
        safety_class: entity.safety_class.clone(),
        owners,
        target_modules,
        constraints,
        relations,
        version: entity.version,
        stale: entity.stale,
        scratch,
    })
}

/// Resolves and projects a single `qdev get` target.
///
/// - An id containing `/` is always resolved as a constraint via `get_constraint`, regardless
///   of any kind hint (constraints are embedded rows, not top-level entities).
/// - Otherwise the id is looked up directly against `entities.id` (the globally unique primary
///   key) rather than re-deriving kind from the identifier grammar. If `kind_hint` is given
///   (the caller passed an explicit kind, e.g. `qdev get story E12S4`) and the resolved
///   entity's actual kind differs, this is a usage error.
pub fn query_entity(
    store: &dyn Store,
    kind_hint: Option<EntityKind>,
    id: &str,
    options: &QueryOptions,
) -> Result<GetResult, QdevError> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(QdevError::usage_error("Entity ID cannot be empty"));
    }

    if trimmed.contains('/') {
        let constraint = store.get_constraint(trimmed)?.ok_or_else(|| {
            QdevError::usage_error_with_code(
                "entity_not_found",
                format!("Constraint '{}' not found", trimmed),
            )
        })?;
        return Ok(GetResult::Constraint(constraint));
    }

    let entity = store.get_entity(trimmed)?.ok_or_else(|| {
        QdevError::usage_error_with_code(
            "entity_not_found",
            format!("Entity '{}' not found", trimmed),
        )
    })?;

    if let Some(expected_kind) = kind_hint {
        if entity.kind != expected_kind {
            return Err(QdevError::usage_error(format!(
                "Entity '{}' is kind '{}', not '{}'",
                trimmed,
                entity.kind.as_str(),
                expected_kind.as_str()
            )));
        }
    }

    let projection = build_entity_projection(store, &entity, options)?;
    Ok(GetResult::Entity(Box::new(projection)))
}

/// A single row in `qdev list` output.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListEntryProjection {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub owners: Vec<String>,
    pub version: u64,
    /// True when the cached row is stale: the source file currently fails to parse (schema
    /// violation, merge conflict, etc.) and this is its last-known-good hydrated state.
    pub stale: bool,
}

impl From<EntityRecord> for ListEntryProjection {
    fn from(e: EntityRecord) -> Self {
        Self {
            id: e.id,
            epic_id: e.epic_id,
            title: e.title,
            status: e.status,
            owners: parse_json_string_array(e.owners.as_deref()),
            version: e.version,
            stale: e.stale,
        }
    }
}

/// Filter options for `qdev list <kind> [--epic --status --owner --module --sprint]`.
/// `--owner me` resolution (against the active identity) happens at the CLI layer before this
/// is constructed — `owner` here is always the already-resolved literal owner/team name.
#[derive(Debug, Clone)]
pub struct ListQueryOptions {
    pub kind: EntityKind,
    pub epic_id: Option<String>,
    pub status: Option<String>,
    pub owner: Option<String>,
    pub module: Option<String>,
    pub sprint: Option<i64>,
}

impl ListQueryOptions {
    pub fn new(kind: EntityKind) -> Self {
        Self {
            kind,
            epic_id: None,
            status: None,
            owner: None,
            module: None,
            sprint: None,
        }
    }
}

/// Runs a filtered, id-ordered entity listing straight from the cache. Every filter applies as
/// an AND.
pub fn query_list(
    store: &dyn Store,
    options: &ListQueryOptions,
) -> Result<Vec<ListEntryProjection>, QdevError> {
    let filter = EntityFilter {
        kind: Some(options.kind),
        status: options.status.clone(),
        epic_id: options.epic_id.clone(),
        owner: options.owner.clone(),
        module: options.module.clone(),
        sprint: options.sprint,
    };

    let entities = store.list_entities(&filter)?;
    Ok(entities
        .into_iter()
        .map(ListEntryProjection::from)
        .collect())
}
