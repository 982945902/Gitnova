//! Deterministic seed + expand retrieval for inference.
//! Each step builds induced subgraph G_t[V_t ∪ U_t], matching paper Algorithm 1.

use crate::model::GraphTransformerModel;
use crate::subgraph::{compute_frontier, extract_subgraph};
use candle_core::{Device, Tensor};
use gitnova_core::model::{CodeGraph, NodeKind};
use std::collections::{HashMap, HashSet};

pub fn seed_expand_retrieve(
    model: &GraphTransformerModel,
    graph: &CodeGraph,
    query_embedding: &[f32],
    node_embeddings: &HashMap<String, Vec<f32>>,
    k_seed: usize, expand_steps: usize, frontier_select: usize,
    subgraph_max_nodes: usize, subgraph_hops: usize, limit: usize,
) -> Vec<String> {
    let device = model.device();
    let config = model.config();

    let _node_lookup: HashMap<&str, usize> = graph.nodes.iter().enumerate()
        .map(|(i, n)| (n.id.as_str(), i)).collect();

    // Step 1: Seed
    let seed_ids = seeding_retrieve(query_embedding, node_embeddings, graph, k_seed);
    if seed_ids.is_empty() { return Vec::new(); }

    // Step 2: Bounded subgraph
    let subgraph = extract_subgraph(graph, &seed_ids, subgraph_max_nodes, subgraph_hops);
    if subgraph.node_ids.is_empty() { return seed_ids; }

    // Pre-build full subgraph features
    let full_features = match build_feature_tensor(
        &subgraph.node_index, graph, node_embeddings, config.input_dim, device,
    ) { Ok(f) => f, Err(_) => return seed_ids };

    let query_tensor = match Tensor::from_vec(
        query_embedding.to_vec(), query_embedding.len(), device,
    ) { Ok(q) => q, Err(_) => return seed_ids };

    // Step 3: Iterative greedy expansion with per-step induced subgraph
    let mut selected: HashSet<usize> = subgraph.seed_indices.iter().copied().collect();
    let mut frontier = subgraph.frontier.clone();

    for _step in 0..expand_steps {
        if frontier.is_empty() { break; }

        // Build induced subgraph G_t[V_t ∪ U_t]
        let induced_indices = build_induced_indices(&selected, &frontier);
        let induced_features = match gather_features(&full_features, &induced_indices) {
            Ok(f) => f, Err(_) => break,
        };
        // Map frontier to positions within induced subgraph
        let frontier_local: Vec<usize> = frontier.iter()
            .map(|&f| induced_indices.iter().position(|&i| i == f).unwrap())
            .collect();

        let (picked, _) = match model.expand_step(
            &induced_features, &query_tensor,
            &frontier_local,
            frontier_select.min(frontier.len()),
        ) {
            Ok(p) => p,
            Err(_) => break,
        };

        // Map local indices back to subgraph indices
        for &local in &picked {
            selected.insert(frontier[local]);
        }
        frontier = compute_frontier(&subgraph.adjacency, &selected);
    }

    // Step 4: Final scoring on V_T
    let final_sel: Vec<usize> = selected.into_iter().collect();
    let final_features = match gather_features(&full_features, &final_sel) {
        Ok(f) => f, Err(_) => return subgraph.node_ids.iter().take(limit).cloned().collect(),
    };
    let scores = match model.score_nodes(&final_features, &query_tensor) {
        Ok(s) => s,
        Err(_) => return subgraph.node_ids.iter().take(limit).cloned().collect(),
    };

    let mut ranked: Vec<(f32, &str)> = final_sel.iter().enumerate()
        .map(|(i, &si)| {
            let nid = &subgraph.node_ids[si];
            (scores.get(i).copied().unwrap_or(0.0), nid.as_str())
        }).collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(limit);
    ranked.into_iter().map(|(_, id)| id.to_string()).collect()
}

fn build_induced_indices(selected: &HashSet<usize>, frontier: &[usize]) -> Vec<usize> {
    let mut v: Vec<usize> = selected.iter().copied().collect();
    for &f in frontier { if !selected.contains(&f) { v.push(f); } }
    v
}

fn gather_features(full: &Tensor, indices: &[usize]) -> Result<Tensor, candle_core::Error> {
    let idx = Tensor::from_vec(
        indices.iter().map(|&i| i as i64).collect::<Vec<_>>(), indices.len(), full.device(),
    )?;
    full.index_select(&idx, 0)
}

fn seeding_retrieve(
    query_emb: &[f32], node_embeddings: &HashMap<String, Vec<f32>>,
    graph: &CodeGraph, k: usize,
) -> Vec<String> {
    let mut scored: Vec<(f64, String)> = Vec::new();
    for node in &graph.nodes {
        if matches!(node.kind, NodeKind::Repository | NodeKind::File | NodeKind::Import) { continue; }
        if let Some(emb) = node_embeddings.get(&node.id) {
            scored.push((cosine(query_emb, emb), node.id.clone()));
        }
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(_, id)| id).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let len = a.len().min(b.len());
    if len == 0 { return 0.0; }
    let (dot, na, nb) = a[..len].iter().zip(&b[..len])
        .fold((0.0f64, 0.0f64, 0.0f64), |(d, na, nb), (&x, &y)| {
            (d + x as f64 * y as f64, na + x as f64 * x as f64, nb + y as f64 * y as f64)
        });
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na.sqrt() * nb.sqrt()) }
}

fn build_feature_tensor(
    node_indices: &[usize], graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>, dim: usize, device: &Device,
) -> Result<Tensor, candle_core::Error> {
    let mut data = Vec::with_capacity(node_indices.len() * dim);
    for &gi in node_indices {
        match node_embeddings.get(&graph.nodes[gi].id) {
            Some(emb) => {
                for &v in emb.iter().take(dim) { data.push(v); }
                for _ in emb.len()..dim { data.push(0.0); }
            }
            None => { for _ in 0..dim { data.push(0.0); } }
        }
    }
    Tensor::from_vec(data, (node_indices.len(), dim), device)
}
