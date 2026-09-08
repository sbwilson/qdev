//! Relation kind-pair validation and `depends_on` cycle detection, per architecture.md §8.
//!
//! Used by hydration (to validate every stored relation and record findings for problems) and
//! by `qdev relate` (to refuse a bad edge before it is ever written).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::schema::EntityKind;

/// Returns the allowed `(source_kind, target_kind)` pairs for a relation name, per
/// architecture.md §8's 8-row table. An unknown relation name returns an empty slice, as does
/// `verifies` (`Gate -> Requirement`): no `Gate` `EntityKind` variant exists yet (deferred to
/// Epic 3), so `verifies` is unreachable until then even though it is a known relation name.
pub fn allowed_kind_pairs(relation: &str) -> &'static [(EntityKind, EntityKind)] {
    use EntityKind::{Adr, DeferredWork, Epic, Hazard, Requirement, Story};
    match relation {
        "depends_on" => &[(Story, Story)],
        "extends" => &[(Story, Story)],
        "supersedes" => &[(Story, Story), (Story, Epic), (Epic, Story), (Epic, Epic)],
        "traces_to" => &[(Story, Requirement)],
        "verifies" => &[],
        "mitigates" => &[(Story, Hazard)],
        "closes_dw" => &[(Story, DeferredWork)],
        "governed_by" => &[(Story, Adr), (Epic, Adr)],
        _ => &[],
    }
}

/// Returns true if `(relation, source, target)` is one of the allowed kind pairs.
pub fn is_valid_kind_pair(relation: &str, source: EntityKind, target: EntityKind) -> bool {
    allowed_kind_pairs(relation)
        .iter()
        .any(|&(s, t)| s == source && t == target)
}

/// Finds one `depends_on` cycle in `edges` (a list of `(source_id, target_id)` pairs), if any.
/// Uses a DFS with a recursion-stack guard, visiting nodes in sorted order for determinism.
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
        let mut stack: Vec<&str> = Vec::new();
        let mut on_stack: HashSet<&str> = HashSet::new();
        if let Some(cycle) = dfs_find_cycle(start, &graph, &mut visited, &mut stack, &mut on_stack)
        {
            return Some(cycle.into_iter().map(str::to_string).collect());
        }
    }
    None
}

fn dfs_find_cycle<'a>(
    node: &'a str,
    graph: &HashMap<&'a str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    stack: &mut Vec<&'a str>,
    on_stack: &mut HashSet<&'a str>,
) -> Option<Vec<&'a str>> {
    visited.insert(node);
    stack.push(node);
    on_stack.insert(node);

    if let Some(children) = graph.get(node) {
        for &child in children {
            if on_stack.contains(child) {
                let pos = stack.iter().position(|&n| n == child).unwrap_or(0);
                let mut cycle: Vec<&str> = stack[pos..].to_vec();
                cycle.push(child);
                return Some(cycle);
            }
            if !visited.contains(child) {
                if let Some(cycle) = dfs_find_cycle(child, graph, visited, stack, on_stack) {
                    return Some(cycle);
                }
            }
        }
    }

    stack.pop();
    on_stack.remove(node);
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
