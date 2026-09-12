//! Relation kind-pair validation and `depends_on` cycle detection, per architecture.md §8.
//!
//! Used by hydration (to validate every stored relation and record findings for problems), by
//! `qdev relate`/`qdev unrelate` (to refuse an unknown relation name before anything else) and
//! by `qdev relate` (to refuse a bad edge before it is ever written).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::errors::QdevError;
use crate::schema::EntityKind;

/// The one list of relations: architecture.md §8's table, in its order, each name beside the
/// `(source_kind, target_kind)` pairs it allows. `allowed_kind_pairs` and `is_known_relation`
/// are both derived from this table, so a ninth relation cannot be added to one and not the
/// other — there is no second list to forget.
///
/// `verifies` (`Gate -> Requirement`) has an empty pair list: no `Gate` `EntityKind` variant
/// exists yet (deferred to Epic 3), so it is unreachable until then even though it is a known
/// relation name. That is exactly why knowing a name is not the same question as knowing its
/// pairs, and why both questions are answered from here.
const RELATION_KIND_PAIRS: &[(&str, &[(EntityKind, EntityKind)])] = {
    use EntityKind::{Adr, DeferredWork, Epic, Hazard, Requirement, Story};
    &[
        ("depends_on", &[(Story, Story)]),
        ("extends", &[(Story, Story)]),
        (
            "supersedes",
            &[(Story, Story), (Story, Epic), (Epic, Story), (Epic, Epic)],
        ),
        ("traces_to", &[(Story, Requirement)]),
        ("verifies", &[]),
        ("mitigates", &[(Story, Hazard)]),
        ("closes_dw", &[(Story, DeferredWork)]),
        ("governed_by", &[(Story, Adr), (Epic, Adr)]),
    ]
};

/// Every relation name architecture.md §8 defines, in the table's order — suitable for naming
/// the valid choices in a usage error.
pub fn relation_names() -> impl Iterator<Item = &'static str> {
    RELATION_KIND_PAIRS.iter().map(|&(name, _)| name)
}

/// Returns true if `relation` is one of architecture.md §8's relation names, whatever kind pairs
/// it allows. `verifies` is known even though it allows none yet, so an unknown *name* and a
/// disallowed *pair* stay distinguishable: the command surface refuses the first as a usage
/// error (exit 2) and the second as `invalid_relation_kind` (exit 1).
pub fn is_known_relation(relation: &str) -> bool {
    RELATION_KIND_PAIRS
        .iter()
        .any(|&(name, _)| name == relation)
}

/// Returns the allowed `(source_kind, target_kind)` pairs for a relation name, per
/// architecture.md §8's 8-row table. An empty slice means "no pair is allowed", which covers
/// both an unknown relation name and `verifies`; use `is_known_relation` to tell those apart.
pub fn allowed_kind_pairs(relation: &str) -> &'static [(EntityKind, EntityKind)] {
    RELATION_KIND_PAIRS
        .iter()
        .find(|&&(name, _)| name == relation)
        .map(|&(_, pairs)| pairs)
        .unwrap_or(&[])
}

/// Returns true if `(relation, source, target)` is one of the allowed kind pairs.
pub fn is_valid_kind_pair(relation: &str, source: EntityKind, target: EntityKind) -> bool {
    allowed_kind_pairs(relation)
        .iter()
        .any(|&(s, t)| s == source && t == target)
}

/// Validates the complete relation map a source entity is about to own, against the graph that
/// would exist after replacing that source's old edges.  This is the command-write gate shared
/// by `relate` and `update --field relations=…`; hydration remains the backstop for hand edits.
///
/// `entities` and `existing_relations` are intentionally plain data rather than a store handle:
/// the gate is pure, so callers can validate a proposed graph without mutating cache state.
pub fn validate_proposed_relation_map(
    source_id: &str,
    source_kind: EntityKind,
    proposed_relations: &BTreeMap<String, Vec<String>>,
    entities: &[(String, EntityKind)],
    existing_relations: &[(String, String, String)],
) -> Result<(), QdevError> {
    let entity_kinds: HashMap<&str, EntityKind> = entities
        .iter()
        .map(|(id, kind)| (id.as_str(), *kind))
        .collect();

    for (relation, targets) in proposed_relations {
        if !is_known_relation(relation) {
            return Err(QdevError::usage_error(format!(
                "Unknown relation '{}', must be one of: {}",
                relation,
                relation_names().collect::<Vec<_>>().join(", ")
            )));
        }

        for target_id in targets {
            let Some(&target_kind) = entity_kinds.get(target_id.as_str()) else {
                return Err(QdevError::logical_failure(
                    "dangling_relation",
                    format!("Target entity '{}' does not exist", target_id),
                ));
            };
            if !is_valid_kind_pair(relation, source_kind, target_kind) {
                return Err(QdevError::logical_failure(
                    "invalid_relation_kind",
                    format!(
                        "Relation '{}' from {} ({}) to {} ({}) is not an allowed kind pair",
                        relation, source_id, source_kind, target_id, target_kind
                    ),
                ));
            }
        }
    }

    // A replacement removes every old source edge before adding its proposed ones.  Checking
    // this final graph (rather than adding candidates to the old graph) permits a replacement
    // that removes an edge from a formerly cyclic source map.
    let mut dependency_edges: Vec<(String, String)> = existing_relations
        .iter()
        .filter(|(source, relation, _)| source != source_id && relation == "depends_on")
        .map(|(source, _, target)| (source.clone(), target.clone()))
        .collect();
    if let Some(targets) = proposed_relations.get("depends_on") {
        dependency_edges.extend(
            targets
                .iter()
                .cloned()
                .map(|target| (source_id.to_string(), target)),
        );
    }

    if let Some(cycle) = find_dependency_cycle(&dependency_edges) {
        return Err(QdevError::logical_failure(
            "dependency_cycle",
            format!(
                "Proposed depends_on relations for '{}' would create a cycle: {}",
                source_id,
                cycle.join(" -> ")
            ),
        ));
    }

    Ok(())
}

/// Finds one `depends_on` cycle in `edges` (a list of `(source_id, target_id)` pairs), if any.
/// Uses an iterative DFS with an on-stack guard, visiting nodes in sorted order for determinism.
/// Returns the cycle as an ordered list of ids where the first and last entries are equal
/// (e.g. `["E1S1", "E1S2", "E1S1"]`), or `None` if the graph is acyclic.
pub fn find_dependency_cycle(edges: &[(String, String)]) -> Option<Vec<String>> {
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    for (source, target) in edges {
        graph
            .entry(source.as_str())
            .or_default()
            .push(target.as_str());
    }
    for adj in graph.values_mut() {
        adj.sort_unstable();
    }

    let mut nodes: Vec<&str> = graph.keys().copied().collect();
    nodes.sort_unstable();

    let mut visited: HashSet<&str> = HashSet::new();
    for &start in &nodes {
        if visited.contains(start) {
            continue;
        }
        if let Some(cycle) = dfs_find_cycle(start, &graph, &mut visited) {
            return Some(cycle.into_iter().map(str::to_string).collect());
        }
    }
    None
}

/// Depth-first search from `start`, iterative rather than recursive: a `depends_on` chain is as
/// deep as the workspace is long, and a recursive walk would overflow the stack on a deeply
/// chained repository (aborting the hydration sweep that calls this). `frames` holds
/// `(node, next_child_index)`, `path` is the current root-to-node chain, and `on_stack` mirrors
/// `path` for O(1) membership tests. Child visit order and the cycle returned are identical to
/// the recursive formulation.
fn dfs_find_cycle<'a>(
    start: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
) -> Option<Vec<&'a str>> {
    let mut frames: Vec<(&'a str, usize)> = vec![(start, 0)];
    let mut path: Vec<&'a str> = vec![start];
    let mut on_stack: HashSet<&'a str> = HashSet::new();
    on_stack.insert(start);
    visited.insert(start);

    while let Some(&(node, idx)) = frames.last() {
        let children: &[&'a str] = graph.get(node).map(Vec::as_slice).unwrap_or(&[]);
        if idx >= children.len() {
            frames.pop();
            path.pop();
            on_stack.remove(node);
            continue;
        }
        if let Some(frame) = frames.last_mut() {
            frame.1 += 1;
        }
        let child = children[idx];
        if on_stack.contains(child) {
            let pos = path.iter().position(|&n| n == child).unwrap_or(0);
            let mut cycle: Vec<&'a str> = path[pos..].to_vec();
            cycle.push(child);
            return Some(cycle);
        }
        if visited.insert(child) {
            path.push(child);
            on_stack.insert(child);
            frames.push((child, 0));
        }
    }

    None
}

/// Returns the cycle that would be closed by adding a `depends_on` edge `source -> target` to
/// the graph already formed by `existing_edges`, or `None` if it would not create one. This is
/// a pre-check for `qdev relate`, used before any write: it does not mutate or require the new
/// edge to already be present in `existing_edges`.
///
/// The returned cycle is ordered starting and ending at `source` (e.g. for `source = "E1S2"`,
/// `target = "E1S1"` with an existing `E1S1 -> E1S2` edge: `["E1S2", "E1S1", "E1S2"]`).
pub fn would_create_cycle(
    existing_edges: &[(String, String)],
    source: &str,
    target: &str,
) -> Option<Vec<String>> {
    if source == target {
        return Some(vec![source.to_string(), target.to_string()]);
    }

    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    for (s, t) in existing_edges {
        graph.entry(s.as_str()).or_default().push(t.as_str());
    }

    // BFS from `target`: if `source` is reachable, the new edge source -> target closes a cycle.
    let mut visited: HashSet<&str> = HashSet::new();
    let mut parent: HashMap<&str, &str> = HashMap::new();
    let mut queue: VecDeque<&str> = VecDeque::new();
    visited.insert(target);
    queue.push_back(target);

    while let Some(node) = queue.pop_front() {
        if node == source {
            // Reconstruct the target -> ... -> source path via parent pointers.
            let mut reversed = vec![node.to_string()];
            let mut cur = node;
            while let Some(&p) = parent.get(cur) {
                reversed.push(p.to_string());
                cur = p;
            }
            reversed.reverse(); // now target -> ... -> source
            let mut cycle = vec![source.to_string()];
            cycle.extend(reversed);
            return Some(cycle);
        }
        if let Some(children) = graph.get(node) {
            for &child in children {
                if visited.insert(child) {
                    parent.insert(child, node);
                    queue.push_back(child);
                }
            }
        }
    }
    None
}
