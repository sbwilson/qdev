use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::StorageConfig;
use crate::errors::QdevError;
use crate::lease::list_leases_with_storage;
use crate::query::{query_list, ListQueryOptions};
use crate::schema::EntityKind;
use crate::store::Store;

/// Story-to-story relation kinds rendered as graph edges.
pub const GRAPH_EDGE_RELATIONS: [&str; 3] = ["depends_on", "extends", "supersedes"];

/// A single story node in the rendered graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub blocked: bool,
    pub lease_holder: Option<String>,
    pub critical_path: bool,
    pub epic_id: Option<String>,
}

/// A directed edge between two story nodes in the rendered graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    pub relation: String,
    pub critical_path: bool,
}

/// Output payload of `qdev graph --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphPayload {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_path: Option<Vec<String>>,
}

/// Options controlling graph querying and rendering.
#[derive(Debug, Clone, Default)]
pub struct StoryGraphOptions {
    pub epic: Option<String>,
    pub sprint: Option<i64>,
    pub highlight_critical_path: bool,
}

/// Computes whether a story is blocked: true iff status is not done and at least one
/// `depends_on` dependency is not live-`done`.
fn compute_story_blocked(
    store: &dyn Store,
    story_id: &str,
    status: Option<&str>,
) -> Result<bool, QdevError> {
    if status == Some("done") {
        return Ok(false);
    }
    let relations = store.get_relations_for_source(story_id)?;
    for rel in relations {
        if rel.relation == "depends_on" {
            let target_done = store
                .get_live_entity_for_derivation(&rel.target_id)?
                .and_then(|entity| entity.status)
                .is_some_and(|s| s == "done");
            if !target_done {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Builds the filtered story graph payload, computing blocked status, active leases,
/// and optional critical path highlighting.
pub fn build_story_graph(
    store: &dyn Store,
    workspace_root: &Path,
    storage: Option<&StorageConfig>,
    options: &StoryGraphOptions,
) -> Result<GraphPayload, QdevError> {
    let mut query_opts = ListQueryOptions::new(EntityKind::Story);
    query_opts.epic_id = options.epic.clone();
    query_opts.sprint = options.sprint;

    let story_projections = query_list(store, &query_opts)?;

    let leases = list_leases_with_storage(workspace_root, storage).unwrap_or_default();
    let lease_map: HashMap<String, String> =
        leases.into_iter().map(|l| (l.story_id, l.holder)).collect();

    let mut nodes: Vec<GraphNode> = Vec::with_capacity(story_projections.len());
    let mut node_ids: HashSet<String> = HashSet::with_capacity(story_projections.len());

    for story in story_projections {
        let blocked = compute_story_blocked(store, &story.id, story.status.as_deref())?;
        let lease_holder = lease_map.get(&story.id).cloned();
        node_ids.insert(story.id.clone());
        nodes.push(GraphNode {
            id: story.id,
            title: story.title,
            status: story.status,
            blocked,
            lease_holder,
            critical_path: false,
            epic_id: story.epic_id,
        });
    }

    nodes.sort_by(|a, b| a.id.cmp(&b.id));

    let all_relations = store.list_relations()?;
    let mut edges: Vec<GraphEdge> = Vec::new();

    for rel in all_relations {
        if !GRAPH_EDGE_RELATIONS.contains(&rel.relation.as_str()) {
            continue;
        }
        if node_ids.contains(&rel.source_id) && node_ids.contains(&rel.target_id) {
            edges.push(GraphEdge {
                source: rel.source_id,
                target: rel.target_id,
                relation: rel.relation,
                critical_path: false,
            });
        }
    }

    // Sort edges deterministically: source ascending, target ascending, relation ascending
    edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
            .then_with(|| a.relation.cmp(&b.relation))
    });

    let mut critical_path_chain: Option<Vec<String>> = None;

    if options.highlight_critical_path {
        let chain = compute_critical_path(&nodes, &edges);
        if !chain.is_empty() {
            let cp_nodes: HashSet<&str> = chain.iter().map(|s| s.as_str()).collect();
            let mut cp_edges: HashSet<(&str, &str)> = HashSet::new();
            for window in chain.windows(2) {
                cp_edges.insert((window[0].as_str(), window[1].as_str()));
            }

            for node in &mut nodes {
                if cp_nodes.contains(node.id.as_str()) {
                    node.critical_path = true;
                }
            }

            for edge in &mut edges {
                if edge.relation == "depends_on"
                    && cp_edges.contains(&(edge.source.as_str(), edge.target.as_str()))
                {
                    edge.critical_path = true;
                }
            }
        }
        critical_path_chain = Some(chain);
    }

    Ok(GraphPayload {
        nodes,
        edges,
        critical_path: critical_path_chain,
    })
}

/// Computes the longest directed dependency chain in the `depends_on` subgraph of included stories.
/// Ties are broken deterministically by lexicographical comparison of candidate story ID sequences.
fn compute_critical_path(nodes: &[GraphNode], edges: &[GraphEdge]) -> Vec<String> {
    let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut has_depends_on = false;

    for edge in edges {
        if edge.relation == "depends_on" {
            has_depends_on = true;
            adj.entry(edge.source.as_str())
                .or_default()
                .push(edge.target.as_str());
        }
    }

    if !has_depends_on {
        return Vec::new();
    }

    for neighbors in adj.values_mut() {
        neighbors.sort();
    }

    let mut memo: HashMap<&str, Vec<String>> = HashMap::new();
    let mut visiting: HashSet<&str> = HashSet::new();

    let mut best_path: Vec<String> = Vec::new();

    for node in nodes {
        let path = dfs_longest_path(node.id.as_str(), &adj, &mut memo, &mut visiting);
        if is_path_better(&path, &best_path) {
            best_path = path;
        }
    }

    if best_path.len() < 2 {
        Vec::new()
    } else {
        best_path
    }
}

/// DFS helper to find the longest directed path from a given node.
fn dfs_longest_path<'a>(
    node: &'a str,
    adj: &HashMap<&str, Vec<&'a str>>,
    memo: &mut HashMap<&'a str, Vec<String>>,
    visiting: &mut HashSet<&'a str>,
) -> Vec<String> {
    if visiting.contains(node) {
        return Vec::new();
    }
    if let Some(cached) = memo.get(node) {
        return cached.clone();
    }

    visiting.insert(node);

    let mut best_sub: Vec<String> = Vec::new();
    if let Some(neighbors) = adj.get(node) {
        for &neighbor in neighbors {
            let sub = dfs_longest_path(neighbor, adj, memo, visiting);
            if is_path_better(&sub, &best_sub) {
                best_sub = sub;
            }
        }
    }

    visiting.remove(node);

    let mut result = Vec::with_capacity(1 + best_sub.len());
    result.push(node.to_string());
    result.extend(best_sub);

    memo.insert(node, result.clone());
    result
}

/// Compares two candidate dependency paths: longer paths win; ties are broken lexicographically.
fn is_path_better(candidate: &[String], current: &[String]) -> bool {
    if candidate.is_empty() {
        return false;
    }
    if current.is_empty() {
        return true;
    }
    if candidate.len() > current.len() {
        return true;
    }
    if candidate.len() < current.len() {
        return false;
    }
    candidate < current
}

/// Escapes a value for use inside a double-quoted DOT string literal.
fn dot_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "")
        .replace('\n', "\\n")
}

/// Determines the DOT fillcolor and border style for a story status.
fn dot_status_style(status: Option<&str>) -> (&'static str, &'static str) {
    match status {
        Some("draft") => ("lightgray", "solid"),
        Some("ready") => ("lightblue", "solid"),
        Some("in-progress") => ("yellow", "solid"),
        Some("review") => ("orange", "solid"),
        Some("done") => ("green", "solid"),
        Some("superseded") | Some("abandoned") => ("gray45", "dashed"),
        _ => ("white", "solid"),
    }
}

/// Renders a GraphPayload as Graphviz DOT text.
pub fn render_graph_dot(payload: &GraphPayload) -> String {
    let mut dot = String::from("digraph qdev {\n");
    for node in &payload.nodes {
        let (color, style) = dot_status_style(node.status.as_deref());
        let mut label = match &node.title {
            Some(title) => format!("{}\\n{}", dot_escape(&node.id), dot_escape(title)),
            None => dot_escape(&node.id),
        };
        if node.blocked {
            label.push_str("\\n[BLOCKED]");
        }
        if let Some(holder) = &node.lease_holder {
            label.push_str(&format!("\\n[lease: {}]", dot_escape(holder)));
        }

        let mut attrs = vec![
            format!("label=\"{}\"", label),
            format!("style=\"filled,{}\"", style),
            format!("fillcolor=\"{}\"", color),
        ];
        if node.blocked {
            attrs.push("peripheries=2".to_string());
        }
        if node.critical_path {
            attrs.push("color=\"red\"".to_string());
            attrs.push("penwidth=2.0".to_string());
        }

        dot.push_str(&format!(
            "  \"{}\" [{}];\n",
            dot_escape(&node.id),
            attrs.join(", ")
        ));
    }
    for edge in &payload.edges {
        let mut attrs = vec![format!("label=\"{}\"", dot_escape(&edge.relation))];
        if edge.critical_path {
            attrs.push("color=\"red\"".to_string());
            attrs.push("penwidth=2.0".to_string());
        }
        dot.push_str(&format!(
            "  \"{}\" -> \"{}\" [{}];\n",
            dot_escape(&edge.source),
            dot_escape(&edge.target),
            attrs.join(", ")
        ));
    }
    dot.push_str("}\n");
    dot
}
