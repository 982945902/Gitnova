use gitnova_core::{CodeGraph, NodeKind};
use gitnova_rank::features::tokenize;
use gitnova_storage::StoredEmbedding;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const LOCAL_HASH_PROVIDER: &str = "local-hash";
const DIMENSIONS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSearchResult {
    pub node_id: String,
    pub qualified_name: String,
    pub path: String,
    pub similarity: f64,
}

pub fn build_local_hash_embeddings(graph: &CodeGraph) -> Vec<StoredEmbedding> {
    graph
        .nodes
        .iter()
        .filter(|node| !matches!(node.kind, NodeKind::Repository | NodeKind::Import))
        .map(|node| StoredEmbedding {
            node_id: node.id.clone(),
            provider: LOCAL_HASH_PROVIDER.into(),
            vector: embed_text(&format!(
                "{} {} {}",
                node.qualified_name, node.path, node.text
            )),
        })
        .collect()
}

pub fn similarity_map(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
) -> HashMap<String, f64> {
    let query_vector = embed_text(query);
    graph
        .nodes
        .iter()
        .filter_map(|node| {
            vectors
                .get(&node.id)
                .map(|vector| (node.id.clone(), cosine(&query_vector, vector)))
        })
        .collect()
}

pub fn search_embeddings(
    graph: &CodeGraph,
    vectors: &HashMap<String, Vec<f32>>,
    query: &str,
    limit: usize,
) -> Vec<EmbeddingSearchResult> {
    let scores = similarity_map(graph, vectors, query);
    let mut results = graph
        .nodes
        .iter()
        .filter_map(|node| {
            scores.get(&node.id).map(|score| EmbeddingSearchResult {
                node_id: node.id.clone(),
                qualified_name: node.qualified_name.clone(),
                path: node.path.clone(),
                similarity: *score,
            })
        })
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    results.truncate(limit);
    results
}

pub fn embed_text(text: &str) -> Vec<f32> {
    let mut vector = vec![0.0f32; DIMENSIONS];
    for token in tokenize(text) {
        let digest = Sha256::digest(token.as_bytes());
        let index = digest[0] as usize % DIMENSIONS;
        let sign = if digest[1] % 2 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    normalize(vector)
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(left, right)| *left as f64 * *right as f64)
        .sum::<f64>()
        .max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitnova_core::build_graph;
    use std::fs;

    #[test]
    fn local_hash_embeddings_are_stable() {
        assert_eq!(embed_text("auth validate"), embed_text("auth validate"));
    }

    #[test]
    fn embedding_search_returns_stable_results() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.rs"),
            "pub fn validate_token() -> bool { true }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        let embeddings = build_local_hash_embeddings(&graph);
        let vectors = embeddings
            .into_iter()
            .map(|embedding| (embedding.node_id, embedding.vector))
            .collect();
        let results = search_embeddings(&graph, &vectors, "validate token", 1);
        assert_eq!(results.len(), 1);
    }
}
