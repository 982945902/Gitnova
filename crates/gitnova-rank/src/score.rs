use crate::explain::ScoreExplanation;
use crate::features;
use gitnova_core::{CodeGraph, Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RankConfig {
    pub lexical_weight: f64,
    pub query_overlap_weight: f64,
    pub relation_weight: f64,
    pub proximity_weight: f64,
    pub domain_weight: f64,
    pub churn_weight: f64,
    pub embedding_weight: f64,
    pub lsp_weight: f64,
    pub hub_penalty_weight: f64,
    pub utility_penalty_weight: f64,
    pub test_penalty_weight: f64,
}

impl Default for RankConfig {
    fn default() -> Self {
        Self {
            lexical_weight: 0.25,
            query_overlap_weight: 0.18,
            relation_weight: 0.12,
            proximity_weight: 0.10,
            domain_weight: 0.10,
            churn_weight: 0.08,
            embedding_weight: 0.12,
            lsp_weight: 0.05,
            hub_penalty_weight: 0.20,
            utility_penalty_weight: 0.20,
            test_penalty_weight: 0.08,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankResponse {
    pub schema_version: u32,
    pub repo_root: String,
    pub query: String,
    pub results: Vec<RankedNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedNode {
    pub node: Node,
    pub score: f64,
    pub explanation: ScoreExplanation,
}

pub fn rank_graph(graph: &CodeGraph, query: &str, limit: usize) -> RankResponse {
    rank_graph_with_embeddings(graph, query, limit, None)
}

pub fn rank_graph_with_fts(
    graph: &CodeGraph,
    query: &str,
    limit: usize,
    embedding_similarity: Option<&HashMap<String, f64>>,
    fts_candidates: Option<&std::collections::HashSet<String>>,
) -> RankResponse {
    rank_graph_with_embeddings_inner(graph, query, limit, embedding_similarity, fts_candidates)
}

pub fn rank_graph_with_embeddings(
    graph: &CodeGraph,
    query: &str,
    limit: usize,
    embedding_similarity: Option<&HashMap<String, f64>>,
) -> RankResponse {
    rank_graph_with_embeddings_inner(graph, query, limit, embedding_similarity, None)
}

fn rank_graph_with_embeddings_inner(
    graph: &CodeGraph,
    query: &str,
    limit: usize,
    embedding_similarity: Option<&HashMap<String, f64>>,
    fts_candidates: Option<&std::collections::HashSet<String>>,
) -> RankResponse {
    let mut config = RankConfig::default();
    if embedding_similarity.is_none() {
        config.lexical_weight += config.embedding_weight * 0.60;
        config.query_overlap_weight += config.embedding_weight * 0.40;
        config.embedding_weight = 0.0;
    }
    config.relation_weight += config.lsp_weight * 0.50;
    config.proximity_weight += config.lsp_weight * 0.50;
    config.lsp_weight = 0.0;

    let query_tokens = features::tokenize(query);
    // FTS boost: top FTS results get a positional bonus (signal, not filter)
    let fts_scores: std::collections::HashMap<String, f64> = fts_candidates
        .as_ref()
        .map(|ids| {
            let n = ids.len().max(1) as f64;
            ids.iter()
                .enumerate()
                .map(|(i, id)| (id.clone(), 0.08 * (1.0 - i as f64 / n)))
                .collect()
        })
        .unwrap_or_default();

    let mut results = graph
        .nodes
        .iter()
        .filter(|node| {
            !(matches!(node.kind, NodeKind::Repository | NodeKind::Import)
                || node.kind == NodeKind::File && node.text.len() > 40_000)
        })
        .map(|node| {
            let mut ranked = score_node(
                graph,
                node,
                query,
                &query_tokens,
                &config,
                embedding_similarity,
            );
            // FTS boost: nodes matching the full-text query get a small bonus
            if let Some(boost) = fts_scores.get(&node.id) {
                ranked.score += boost;
            }
            ranked
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.qualified_name.cmp(&b.node.qualified_name))
    });
    results.truncate(limit);
    RankResponse {
        schema_version: graph.schema_version,
        repo_root: graph.repo_root.clone(),
        query: query.to_string(),
        results,
    }
}

fn score_node(
    graph: &CodeGraph,
    node: &Node,
    query: &str,
    query_tokens: &[String],
    config: &RankConfig,
    embedding_similarity: Option<&HashMap<String, f64>>,
) -> RankedNode {
    let lexical = features::lexical_match(query, query_tokens, node);
    let overlap = features::query_overlap(query_tokens, node);
    let relation = features::relation_importance(node);
    let proximity = features::graph_proximity(graph, node);
    let domain = features::domain_specificity(query_tokens, node);
    let churn = features::churn(node);
    let embedding = embedding_similarity
        .and_then(|scores| scores.get(&node.id).copied())
        .unwrap_or(0.0);
    let hub_penalty = features::hub_penalty(node);
    let utility_penalty = features::utility_penalty(node, overlap);
    let test_penalty = if node.metrics.is_test { 1.0 } else { 0.0 };
    let kind_bias = match node.kind {
        NodeKind::Function | NodeKind::Method | NodeKind::Macro => 0.08,
        NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Enum
        | NodeKind::Union
        | NodeKind::Typedef
        | NodeKind::Trait
        | NodeKind::Interface => 0.05,
        NodeKind::Variable => -0.05,
        NodeKind::Module => -0.15,
        NodeKind::File => -0.02,
        _ => 0.0,
    };
    let score = config.lexical_weight * lexical
        + config.query_overlap_weight * overlap
        + config.relation_weight * relation
        + config.proximity_weight * proximity
        + config.domain_weight * domain
        + config.churn_weight * churn
        + config.embedding_weight * embedding
        - config.hub_penalty_weight * hub_penalty
        - config.utility_penalty_weight * utility_penalty
        - config.test_penalty_weight * test_penalty
        + kind_bias;
    let mut explanation = ScoreExplanation::new();
    if lexical > 0.6 {
        explanation.strong_signals.push("lexical match".into());
    }
    if overlap > 0.2 {
        explanation
            .strong_signals
            .push("query token overlap".into());
    }
    if relation > 0.3 {
        explanation.weak_signals.push("graph relations".into());
    }
    if churn > 0.0 {
        explanation.weak_signals.push("recent churn".into());
    }
    if embedding > 0.0 {
        explanation.weak_signals.push("embedding similarity".into());
    }
    if hub_penalty > 0.0 {
        explanation.penalties.push("hub penalty".into());
    }
    if utility_penalty > 0.0 {
        explanation.penalties.push("generic utility penalty".into());
    }
    if test_penalty > 0.0 {
        explanation.penalties.push("test penalty".into());
    }
    RankedNode {
        node: node.clone(),
        score,
        explanation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::{build_graph, EdgeKind};
    use std::fs;

    #[test]
    fn utility_is_downranked_unless_query_targets_it() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("utils.ts"),
            "export function formatDate(d: Date): string { return d.toISOString(); }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("auth.ts"),
            "import { formatDate } from './utils';\nexport interface Session { userId: string; expiresAt: Date; }\nexport function validateSession(s: Session): boolean { return formatDate(s.expiresAt).length > 0; }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        assert!(graph.edges.iter().any(|edge| edge.kind == EdgeKind::Calls));
        let auth = rank_graph(&graph, "change auth session validation", 3);
        assert!(auth.results[0]
            .node
            .qualified_name
            .contains("validateSession"));
        let validated = rank_graph(&graph, "Where is auth session validated?", 3);
        assert!(validated.results[0]
            .node
            .qualified_name
            .contains("validateSession"));
        let util = rank_graph(&graph, "formatDate utility", 3);
        assert!(util.results[0].node.qualified_name.contains("formatDate"));
    }
}
