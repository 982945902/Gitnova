//! Self-supervised training data generation from CodeGraph.
//!
//! Strategy: for each meaningful node (Function, Method, Class, etc.), create a
//! training example where:
//!   - query_text = concatenation of qualified_name, path, text, kind
//!   - query_embedding = embed_text(query_text) using the existing embedding provider
//!   - positive_ids = the node itself + 2-hop BFS neighbors via Calls/References/Imports/Extends

use anyhow::Result;
use gitnova_core::model::{CodeGraph, EdgeKind, NodeKind};
use gitnova_enrich::embeddings::embed_text_for_provider;
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::{HashMap, HashSet, VecDeque};

/// A single training example for SeedER.
#[derive(Debug, Clone)]
pub struct TrainingExample {
    /// Query text derived from a "query node".
    pub query_text: String,
    /// Pre-computed embedding of the query.
    pub query_embedding: Vec<f32>,
    /// Node IDs that are relevant ground truth for this query.
    pub positive_ids: Vec<String>,
}

/// Edge kinds to follow when building positive sets via BFS.
const POSITIVE_EDGES: &[EdgeKind] = &[
    EdgeKind::Calls,
    EdgeKind::References,
    EdgeKind::Imports,
    EdgeKind::Extends,
];

/// Node kinds eligible to be training query nodes.
const QUERY_KINDS: &[NodeKind] = &[
    NodeKind::Function,
    NodeKind::Method,
    NodeKind::Class,
    NodeKind::Struct,
    NodeKind::Enum,
    NodeKind::Trait,
    NodeKind::Interface,
    NodeKind::Module,
    NodeKind::Macro,
    NodeKind::Typedef,
];

/// Node kinds to exclude from positive sets (too broad).
const SKIP_KINDS: &[NodeKind] = &[NodeKind::Repository, NodeKind::File, NodeKind::Import];

/// Generate training examples from a CodeGraph.
///
/// * `graph` - the indexed code graph
/// * `node_embeddings` - pre-computed node embeddings (node_id → vector)
/// * `provider` - embedding provider name (e.g. "local-hash")
/// * `max_examples` - cap on number of examples
pub fn generate_training_examples(
    graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>,
    provider: &str,
    max_examples: usize,
) -> Result<Vec<TrainingExample>> {
    // Build node lookup and adjacency
    let node_lookup: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); graph.nodes.len()];
    for edge in &graph.edges {
        if let (Some(&from), Some(&to)) = (
            node_lookup.get(edge.from.as_str()),
            node_lookup.get(edge.to.as_str()),
        ) {
            if is_positive_edge(&edge.kind) {
                outgoing[from].push(to);
                outgoing[to].push(from); // undirected for BFS
            }
        }
    }

    // Collect eligible query nodes
    let mut candidates: Vec<usize> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| {
            is_query_kind(&n.kind)
                && !n.metrics.is_test
                && !n.text.is_empty()
                && node_embeddings.contains_key(&n.id)
        })
        .map(|(i, _)| i)
        .collect();

    // Shuffle and cap
    candidates.shuffle(&mut thread_rng());
    candidates.truncate(max_examples);

    let mut examples = Vec::with_capacity(candidates.len());

    for &node_idx in &candidates {
        let node = &graph.nodes[node_idx];

        // Build query text (same format as embeddings)
        let query_text = format!(
            "{} {} {} {:?}",
            node.qualified_name, node.path, node.text, node.kind
        );

        // Embed the query
        let query_embedding = match embed_text_for_provider(provider, &query_text) {
            Ok(emb) => emb,
            Err(_) => continue,
        };

        if query_embedding.iter().all(|&v| v == 0.0) {
            continue;
        }

        // Build positive set: node itself + 2-hop BFS
        let mut positives: HashSet<String> = HashSet::new();
        positives.insert(node.id.clone());

        let mut visited: HashSet<usize> = HashSet::new();
        visited.insert(node_idx);
        let mut queue: VecDeque<(usize, usize)> = VecDeque::new(); // (idx, hop)
        queue.push_back((node_idx, 0));

        while let Some((current, hop)) = queue.pop_front() {
            if hop >= 2 {
                continue;
            }
            for &neighbor in &outgoing[current] {
                if visited.insert(neighbor) {
                    let neighbor_node = &graph.nodes[neighbor];
                    if !is_skip_kind(&neighbor_node.kind) {
                        positives.insert(neighbor_node.id.clone());
                    }
                    queue.push_back((neighbor, hop + 1));
                }
            }
        }

        // Minimum 3 positives for useful training signal
        if positives.len() < 3 {
            continue;
        }

        // Cap positives at 50 to keep reward computation tractable
        let positive_ids: Vec<String> = positives.into_iter().take(50).collect();

        examples.push(TrainingExample {
            query_text,
            query_embedding,
            positive_ids,
        });
    }

    Ok(examples)
}

/// Check if an edge kind should be followed for positive set construction.
fn is_positive_edge(kind: &EdgeKind) -> bool {
    POSITIVE_EDGES.contains(kind)
}

/// Check if a node kind can be a training query node.
fn is_query_kind(kind: &NodeKind) -> bool {
    QUERY_KINDS.contains(kind)
}

/// Check if a node kind should be excluded from positive sets.
fn is_skip_kind(kind: &NodeKind) -> bool {
    SKIP_KINDS.contains(kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::model::{Edge, Node, NodeMetrics};
    use gitnova_enrich::embeddings::LOCAL_HASH_PROVIDER;

    fn make_node(id: &str, name: &str, kind: NodeKind, text: &str) -> Node {
        Node {
            id: id.to_string(),
            kind,
            name: name.to_string(),
            qualified_name: name.to_string(),
            path: "test.rs".to_string(),
            span: None,
            language: None,
            text: text.to_string(),
            tags: vec![],
            metrics: NodeMetrics::default(),
        }
    }

    fn make_edge(from: &str, to: &str, kind: EdgeKind) -> Edge {
        Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            confidence_basis_points: 10000,
        }
    }

    #[test]
    fn test_generate_examples_includes_neighbors() {
        let graph = CodeGraph {
            schema_version: 1,
            repo_root: String::new(),
            indexed_at_unix: 0,
            nodes: vec![
                make_node("a", "fn_a", NodeKind::Function, "fn a() {}"),
                make_node("b", "fn_b", NodeKind::Function, "fn b() {}"),
                make_node("c", "fn_c", NodeKind::Function, "fn c() {}"),
            ],
            edges: vec![
                make_edge("a", "b", EdgeKind::Calls),
                make_edge("b", "c", EdgeKind::Calls),
            ],
        };

        // Create mock embeddings (64-dim for local-hash)
        let embeddings: HashMap<String, Vec<f32>> = graph
            .nodes
            .iter()
            .map(|n| (n.id.clone(), vec![0.1; 64]))
            .collect();

        // Use local-hash provider (pure computation, no external deps)
        let examples =
            generate_training_examples(&graph, &embeddings, LOCAL_HASH_PROVIDER, 100).unwrap();

        assert!(!examples.is_empty());
        // Each example should include the query node itself
        for ex in &examples {
            assert!(!ex.positive_ids.is_empty());
            assert!(!ex.query_embedding.is_empty());
        }
    }
}
