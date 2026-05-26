//! Training loop for SeedER GNN model.
//!
//! Uses BPR (Bayesian Personalized Ranking) loss to train both the scoring head
//! (for final ranking) and the policy head (for frontier expansion decisions).
//! The GNN backbone learns useful representations from both signals.

use crate::data::TrainingExample;
use crate::model::{GraphSAGEModel, ModelConfig};
use crate::subgraph::extract_subgraph;
use anyhow::Result;
use candle_core::{DType, Device, Tensor};
use candle_nn::{AdamW, Optimizer, ParamsAdamW, VarMap};
use gitnova_core::model::{CodeGraph, NodeKind};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::{HashMap, HashSet};

/// Training configuration.
pub struct TrainConfig {
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f64,
    pub bpr_weight: f64,
    pub policy_weight: f64,
    pub seed_k: usize,
    pub subgraph_max_nodes: usize,
    pub subgraph_hops: usize,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            epochs: 50,
            batch_size: 4,
            learning_rate: 0.001,
            bpr_weight: 1.0,
            policy_weight: 0.3,
            seed_k: 10,
            subgraph_max_nodes: 200,
            subgraph_hops: 2,
        }
    }
}

/// Train a SeedER model using a pre-created VarMap (so caller can save weights after).
///
/// Returns () — the model and VarMap are mutated in place.
pub fn train(
    examples: &[TrainingExample],
    graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>,
    model: &mut GraphSAGEModel,
    varmap: &VarMap,
    model_config: &ModelConfig,
    train_config: &TrainConfig,
) -> Result<()> {
    let device = model.device();

    // Build node lookup for fast ID→index mapping
    let node_lookup: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // Build all-node embeddings as a flat Vec for cosine similarity seeding
    let all_embedding_ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let all_embeddings: Vec<&[f32]> = graph
        .nodes
        .iter()
        .map(|n| node_embeddings.get(&n.id).map(|v| v.as_slice()).unwrap_or(&[]))
        .collect();

    let adam_params = ParamsAdamW {
        lr: train_config.learning_rate,
        weight_decay: 0.0,
        ..Default::default()
    };
    let mut optimizer = AdamW::new(varmap.all_vars(), adam_params)?;

    let mut rng = thread_rng();
    let total_examples = examples.len();

    for epoch in 0..train_config.epochs {
        let mut indices: Vec<usize> = (0..total_examples).collect();
        indices.shuffle(&mut rng);

        let mut epoch_loss = 0.0f64;
        let mut batches = 0usize;

        for batch_start in (0..total_examples).step_by(train_config.batch_size) {
            let batch_end = (batch_start + train_config.batch_size).min(total_examples);
            let mut batch_loss: Option<Tensor> = None;

            for &idx in &indices[batch_start..batch_end] {
                let example = &examples[idx];

                // Step 1: Seed — top-k cosine similarity
                let seed_ids = top_k_cosine(
                    &example.query_embedding,
                    &all_embedding_ids,
                    &all_embeddings,
                    train_config.seed_k,
                    &node_lookup,
                    graph,
                );

                if seed_ids.is_empty() {
                    continue;
                }

                // Step 2: Extract bounded subgraph
                let subgraph = extract_subgraph(
                    graph,
                    &seed_ids,
                    train_config.subgraph_max_nodes,
                    train_config.subgraph_hops,
                );

                if subgraph.node_ids.len() < 5 {
                    continue;
                }

                // Step 3: Build feature tensor [N, D_in]
                let features = build_feature_tensor(
                    &subgraph.node_index,
                    graph,
                    node_embeddings,
                    model_config.input_dim,
                    device,
                );

                let Ok(features) = features else { continue };

                let Ok(query_tensor) = Tensor::from_vec(
                    example.query_embedding.clone(),
                    example.query_embedding.len(),
                    device,
                ) else { continue };

                // Build positive set mask for this subgraph
                let positive_set: HashSet<&str> =
                    example.positive_ids.iter().map(|s| s.as_str()).collect();

                let positive_indices: Vec<usize> = subgraph
                    .node_ids
                    .iter()
                    .enumerate()
                    .filter(|(_, id)| positive_set.contains(id.as_str()))
                    .map(|(i, _)| i)
                    .collect();

                if positive_indices.is_empty() {
                    continue;
                }

                // Step 4: Forward pass
                let (_h, policy_logits, scores) = model.forward(
                    &features,
                    &subgraph.adjacency,
                    &query_tensor,
                )?;

                // Step 5: BPR scoring loss
                let bpr = bpr_loss(&scores, &positive_indices, &subgraph.adjacency, device)?;

                // Step 6: Policy BPR loss — encourage frontier nodes adjacent to positives
                // to have higher policy logits
                let pol_loss = policy_bpr_loss(
                    &policy_logits,
                    &subgraph.frontier,
                    &positive_indices,
                    &subgraph.adjacency,
                    device,
                )?;

                let loss = (bpr * train_config.bpr_weight)?;
                let loss = loss.add(&(pol_loss * train_config.policy_weight)?)?;

                batch_loss = match batch_loss {
                    Some(bl) => Some(bl.add(&loss)?),
                    None => Some(loss),
                };
            }

            if let Some(ref loss) = batch_loss {
                epoch_loss += loss.to_vec0::<f64>()?;
                optimizer.backward_step(loss)?;
                batches += 1;
            }
        }

        if batches > 0 {
            eprintln!(
                "epoch {:3}/{:3}  avg_loss={:.6}",
                epoch + 1,
                train_config.epochs,
                epoch_loss / batches as f64
            );
        }
    }

    Ok(())
}

/// BPR loss: for each positive node, sample a negative and compute
/// -log(sigmoid(score_pos - score_neg)).
fn bpr_loss(
    scores: &Tensor,
    positive_indices: &[usize],
    _adjacency: &[Vec<usize>],
    device: &Device,
) -> candle_core::Result<Tensor> {
    let n = scores.dims()[0];
    if positive_indices.is_empty() || n < 2 {
        return Ok(Tensor::zeros((), DType::F32, device)?);
    }

    let mut rng = thread_rng();
    let mut total_loss = Tensor::zeros((), DType::F32, device)?;
    let mut count = 0;

    for &pos_idx in positive_indices {
        // Sample a negative: prefer non-neighbor of positive for harder examples
        let neg_candidates: Vec<usize> = (0..n)
            .filter(|&i| !positive_indices.contains(&i) && i != pos_idx)
            .collect();

        if neg_candidates.is_empty() {
            continue;
        }

        let neg_idx = *neg_candidates.choose(&mut rng).unwrap();

        let pos_score = scores.get(pos_idx)?;
        let neg_score = scores.get(neg_idx)?;
        let diff = pos_score.sub(&neg_score)?;
        let sigmoid_diff = candle_nn::ops::sigmoid(&diff)?;
        let neg_log = sigmoid_diff.log()?; // log(sigmoid(diff)), should be negative
        total_loss = total_loss.add(&neg_log)?;
        count += 1;
    }

    if count > 0 {
        total_loss = total_loss.neg()?; // -log(sigmoid) → positive loss
        total_loss = total_loss.affine(1.0 / count as f64, 0.0)?; // mean
    }

    Ok(total_loss)
}

/// Policy BPR loss: frontier nodes that are adjacent to positive nodes should
/// have higher policy logits than frontier nodes that are not.
fn policy_bpr_loss(
    policy_logits: &Tensor,
    frontier: &[usize],
    positive_indices: &[usize],
    adjacency: &[Vec<usize>],
    device: &Device,
) -> candle_core::Result<Tensor> {
    if frontier.is_empty() || positive_indices.is_empty() {
        return Ok(Tensor::zeros((), DType::F32, device)?);
    }

    let pos_set: HashSet<usize> = positive_indices.iter().copied().collect();

    // "Good" frontier nodes: adjacent to at least one positive
    let good: Vec<usize> = frontier
        .iter()
        .copied()
        .filter(|&f| adjacency[f].iter().any(|n| pos_set.contains(n)))
        .collect();

    // "Bad" frontier nodes: not adjacent to any positive
    let bad: Vec<usize> = frontier
        .iter()
        .copied()
        .filter(|&f| !adjacency[f].iter().any(|n| pos_set.contains(n)))
        .collect();

    if good.is_empty() || bad.is_empty() {
        return Ok(Tensor::zeros((), DType::F32, device)?);
    }

    let mut rng = thread_rng();
    let good_idx = *good.choose(&mut rng).unwrap();
    let bad_idx = *bad.choose(&mut rng).unwrap();

    let good_score = policy_logits.get(good_idx)?;
    let bad_score = policy_logits.get(bad_idx)?;
    let diff = good_score.sub(&bad_score)?;
    let sig = candle_nn::ops::sigmoid(&diff)?;
    sig.log()?.neg()
}

/// Top-k cosine similarity retrieval of seed nodes.
fn top_k_cosine(
    query_emb: &[f32],
    all_ids: &[&str],
    all_embs: &[&[f32]],
    k: usize,
    node_lookup: &HashMap<&str, usize>,
    graph: &CodeGraph,
) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = all_ids
        .iter()
        .enumerate()
        .filter_map(|(i, &id)| {
            let emb = all_embs[i];
            if emb.is_empty() {
                return None;
            }
            // Skip Repository, File, Import nodes
            if let Some(&node_idx) = node_lookup.get(id) {
                if matches!(
                    graph.nodes[node_idx].kind,
                    NodeKind::Repository | NodeKind::File | NodeKind::Import
                ) {
                    return None;
                }
            }
            let sim = cosine_similarity(query_emb, emb);
            Some((sim, id))
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(_, id)| id.to_string()).collect()
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    let (dot, norm_a, norm_b) = a[..len]
        .iter()
        .zip(&b[..len])
        .fold((0.0f64, 0.0f64, 0.0f64), |(d, na, nb), (&x, &y)| {
            (d + x as f64 * y as f64, na + x as f64 * x as f64, nb + y as f64 * y as f64)
        });
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Build a feature tensor [N, D] from node embeddings for the subgraph.
fn build_feature_tensor(
    node_indices: &[usize],
    graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>,
    dim: usize,
    device: &Device,
) -> candle_core::Result<Tensor> {
    let mut data = Vec::with_capacity(node_indices.len() * dim);
    for &global_idx in node_indices {
        let node_id = &graph.nodes[global_idx].id;
        match node_embeddings.get(node_id) {
            Some(emb) => {
                for &v in emb.iter().take(dim) {
                    data.push(v);
                }
                // Pad with zeros if needed
                for _ in emb.len()..dim {
                    data.push(0.0);
                }
            }
            None => {
                for _ in 0..dim {
                    data.push(0.0);
                }
            }
        }
    }
    Tensor::from_vec(data, (node_indices.len(), dim), device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &c) - 0.0).abs() < 0.001);
    }
}
