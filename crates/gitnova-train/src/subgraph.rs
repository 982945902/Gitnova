use gitnova_core::model::{CodeGraph, EdgeKind, NodeKind};
use std::collections::{HashMap, HashSet, VecDeque};

/// A bounded subgraph extracted around seed nodes for efficient GNN computation.
pub struct BoundedSubgraph {
    /// All node IDs in the subgraph.
    pub node_ids: Vec<String>,
    /// Indices of seed nodes within node_ids.
    pub seed_indices: Vec<usize>,
    /// Adjacency list: for each node index, list of neighbor indices (undirected).
    pub adjacency: Vec<Vec<usize>>,
    /// Mapping from subgraph node index to original CodeGraph node index.
    pub node_index: Vec<usize>,
    /// Current frontier: node indices adjacent to selected but not in selected.
    pub frontier: Vec<usize>,
}

/// Edge kinds to follow during graph expansion.
const EXPANSION_EDGES: &[EdgeKind] = &[
    EdgeKind::Calls,
    EdgeKind::References,
    EdgeKind::Imports,
    EdgeKind::Extends,
];

/// Node kinds to skip when expanding (too broad).
const SKIP_KINDS: &[NodeKind] = &[NodeKind::Repository, NodeKind::File, NodeKind::Import];

/// Extract a bounded subgraph from seed nodes via K-hop BFS with filtering.
///
/// * `graph` - the full CodeGraph
/// * `seed_ids` - node IDs to start expansion from
/// * `max_nodes` - cap the subgraph to this many nodes
/// * `k` - number of BFS hops
pub fn extract_subgraph(
    graph: &CodeGraph,
    seed_ids: &[String],
    max_nodes: usize,
    k: usize,
) -> BoundedSubgraph {
    // Build node_id → index lookup for fast graph traversal
    let node_lookup: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // Build outgoing adjacency: src_idx → Vec<(dst_idx, edge_kind)>
    let mut outgoing: Vec<Vec<(usize, &EdgeKind)>> = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        if let (Some(&from), Some(&to)) = (node_lookup.get(edge.from.as_str()), node_lookup.get(edge.to.as_str())) {
            outgoing[from].push((to, &edge.kind));
            // Also add reverse for undirected traversal
            outgoing[to].push((from, &edge.kind));
        }
    }

    // BFS from seeds
    let mut visited: HashSet<usize> = HashSet::new();
    let mut queue: VecDeque<usize> = VecDeque::new();

    for seed_id in seed_ids {
        if let Some(&idx) = node_lookup.get(seed_id.as_str()) {
            if !is_skip_kind(&graph.nodes[idx].kind) {
                visited.insert(idx);
                queue.push_back(idx);
            }
        }
    }

    let mut current_hop = 0;
    while current_hop < k && visited.len() < max_nodes {
        let level_size = queue.len();
        for _ in 0..level_size {
            let node_idx = queue.pop_front().unwrap();
            for &(neighbor_idx, edge_kind) in &outgoing[node_idx] {
                if visited.contains(&neighbor_idx) {
                    continue;
                }
                if !is_expansion_edge(edge_kind) {
                    continue;
                }
                if visited.len() >= max_nodes {
                    break;
                }
                visited.insert(neighbor_idx);
                queue.push_back(neighbor_idx);
            }
            if visited.len() >= max_nodes {
                break;
            }
        }
        current_hop += 1;
    }

    // Build subgraph node list and index mapping
    let subgraph_node_indices: Vec<usize> = visited.into_iter().collect();
    let subgraph_to_global: Vec<usize> = subgraph_node_indices.clone();
    let global_to_subgraph: HashMap<usize, usize> = subgraph_node_indices
        .iter()
        .enumerate()
        .map(|(sub_idx, &global_idx)| (global_idx, sub_idx))
        .collect();

    let node_ids: Vec<String> = subgraph_node_indices
        .iter()
        .map(|&i| graph.nodes[i].id.clone())
        .collect();

    let seed_indices: Vec<usize> = seed_ids
        .iter()
        .filter_map(|sid| {
            node_lookup
                .get(sid.as_str())
                .and_then(|&gidx| global_to_subgraph.get(&gidx).copied())
        })
        .collect();

    // Build adjacency within subgraph
    let n = node_ids.len();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (sub_idx, &global_idx) in subgraph_node_indices.iter().enumerate() {
        for &(neighbor_idx, edge_kind) in &outgoing[global_idx] {
            if let Some(&neighbor_sub_idx) = global_to_subgraph.get(&neighbor_idx) {
                if is_expansion_edge(edge_kind) {
                    adjacency[sub_idx].push(neighbor_sub_idx);
                }
            }
        }
    }

    // Compute initial frontier: neighbors of seeds not in seed set
    let seed_set: HashSet<usize> = seed_indices.iter().copied().collect();
    let frontier = compute_frontier(&adjacency, &seed_set);

    BoundedSubgraph {
        node_ids,
        seed_indices,
        adjacency,
        node_index: subgraph_to_global,
        frontier,
    }
}

/// Recompute frontier given updated selected set.
pub fn compute_frontier(adjacency: &[Vec<usize>], selected: &HashSet<usize>) -> Vec<usize> {
    let mut frontier: HashSet<usize> = HashSet::new();
    for &sel in selected {
        for &neighbor in &adjacency[sel] {
            if !selected.contains(&neighbor) {
                frontier.insert(neighbor);
            }
        }
    }
    frontier.into_iter().collect()
}

fn is_expansion_edge(kind: &EdgeKind) -> bool {
    EXPANSION_EDGES.contains(kind)
}

fn is_skip_kind(kind: &NodeKind) -> bool {
    SKIP_KINDS.contains(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::model::{Edge, Node, NodeMetrics};

    fn make_node(id: &str, name: &str, kind: NodeKind) -> Node {
        Node {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: name.to_string(),
            path: String::new(),
            span: None,
            language: None,
            text: String::new(),
            tags: vec![],
            metrics: NodeMetrics::default(),
        }
    }

    fn make_edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge { from: from.to_string(), to: to.to_string(), kind, confidence_basis_points: 10000 }
    }

    #[test]
    fn test_extract_small_subgraph() {
        let graph = CodeGraph {
            schema_version: 1,
            repo_root: String::new(),
            indexed_at_unix: 0,
            nodes: vec![
                make_node("a", "fn_a", NodeKind::Function),
                make_node("b", "fn_b", NodeKind::Function),
                make_node("c", "fn_c", NodeKind::Function),
                make_node("d", "fn_d", NodeKind::Function),
            ],
            edges: vec![
                make_edge("a", "b", EdgeKind::Calls),
                make_edge("b", "c", EdgeKind::Calls),
                make_edge("b", "d", EdgeKind::References),
            ],
        };

        let sub = extract_subgraph(&graph, &["a".into()], 100, 2);
        assert_eq!(sub.node_ids.len(), 4);
        assert!(sub.seed_indices.len() == 1);
        assert!(!sub.frontier.is_empty());
    }

    #[test]
    fn test_extract_respects_max_nodes() {
        let mut nodes = vec![];
        let mut edges = vec![];
        for i in 0..50 {
            nodes.push(make_node(&format!("n{i}"), &format!("node_{i}"), NodeKind::Function));
            if i > 0 {
                edges.push(make_edge(&format!("n{i}"), &format!("n{}", i - 1), EdgeKind::Calls));
            }
        }
        let graph = CodeGraph {
            schema_version: 1,
            repo_root: String::new(),
            indexed_at_unix: 0,
            nodes,
            edges,
        };

        let sub = extract_subgraph(&graph, &["n25".into()], 10, 5);
        assert!(sub.node_ids.len() <= 10);
    }

    #[test]
    fn test_skip_repository_and_file_nodes() {
        let graph = CodeGraph {
            schema_version: 1,
            repo_root: String::new(),
            indexed_at_unix: 0,
            nodes: vec![
                make_node("repo", "repo", NodeKind::Repository),
                make_node("file", "main.rs", NodeKind::File),
                make_node("func", "main", NodeKind::Function),
            ],
            edges: vec![
                make_edge("repo", "file", EdgeKind::Contains),
                make_edge("file", "func", EdgeKind::Defines),
            ],
        };

        let sub = extract_subgraph(&graph, &["repo".into()], 100, 2);
        // Repository node is skipped as seed, contains edges aren't expansion edges
        assert!(sub.node_ids.is_empty());
    }
}
