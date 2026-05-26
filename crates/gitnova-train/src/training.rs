//! REINFORCE + BPR training loop for SeedER GNN model.
//!
//! Implements the SeedER paper's group-centered policy gradient with
//! M trajectories per query and BPR auxiliary ranking loss.

use crate::data::TrainingExample;
use crate::model::{GraphSAGEModel, ModelConfig};
use crate::subgraph::{compute_frontier, extract_subgraph};
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
    pub num_trajectories: usize,   // M trajectories per example (default 8)
    pub expand_steps: usize,       // T expansion steps per trajectory (default 3)
    pub frontier_select: usize,    // c nodes selected per step (default 5)
    pub reinforce_weight: f64,     // weight of REINFORCE loss (default 1.0)
    pub bpr_weight: f64,           // weight of BPR auxiliary loss (default 0.1)
    pub seed_k: usize,             // number of seed nodes from dense retrieval
    pub subgraph_max_nodes: usize,
    pub subgraph_hops: usize,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            epochs: 50,
            batch_size: 4,
            learning_rate: 0.001,
            num_trajectories: 8,
            expand_steps: 3,
            frontier_select: 5,
            reinforce_weight: 1.0,
            bpr_weight: 0.1,
            seed_k: 10,
            subgraph_max_nodes: 200,
            subgraph_hops: 2,
        }
    }
}

/// Train a SeedER model via REINFORCE + BPR.
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

    // Build node lookup
    let node_lookup: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    // Flatten all embeddings for cosine seeding
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

        let mut epoch_rl_loss = 0.0f64;
        let mut epoch_bpr_loss = 0.0f64;
        let mut batches = 0usize;

        for batch_start in (0..total_examples).step_by(train_config.batch_size) {
            let batch_end = (batch_start + train_config.batch_size).min(total_examples);
            let mut batch_loss: Option<Tensor> = None;

            for &idx in &indices[batch_start..batch_end] {
                let example = &examples[idx];

                // ── Step 1: Seed ──
                let seed_ids = top_k_cosine(
                    &example.query_embedding,
                    &all_embedding_ids,
                    &all_embeddings,
                    train_config.seed_k,
                    &node_lookup,
                    graph,
                );
                if seed_ids.is_empty() { continue; }

                // ── Step 2: Bounded subgraph ──
                let subgraph = extract_subgraph(
                    graph, &seed_ids,
                    train_config.subgraph_max_nodes,
                    train_config.subgraph_hops,
                );
                if subgraph.node_ids.len() < 5 { continue; }

                // ── Step 3: Build feature tensor ──
                let features = match build_feature_tensor(
                    &subgraph.node_index, graph, node_embeddings,
                    model_config.input_dim, device,
                ) {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                let query_tensor = match Tensor::from_vec(
                    example.query_embedding.clone(),
                    example.query_embedding.len(), device,
                ) {
                    Ok(q) => q,
                    Err(_) => continue,
                };

                // ── Positive set ──
                let positive_set: HashSet<&str> =
                    example.positive_ids.iter().map(|s| s.as_str()).collect();
                let positive_indices: Vec<usize> = subgraph
                    .node_ids.iter().enumerate()
                    .filter(|(_, id)| positive_set.contains(id.as_str()))
                    .map(|(i, _)| i)
                    .collect();
                if positive_indices.is_empty() { continue; }

                let pos_set: HashSet<usize> = positive_indices.iter().copied().collect();

                // ═══════════════════════════════════════════════
                // Step 4: Sample M trajectories via REINFORCE
                // ═══════════════════════════════════════════════

                let num_traj = train_config.num_trajectories;
                let num_steps = train_config.expand_steps;
                let select_per_step = train_config.frontier_select;

                let mut trajectory_log_probs: Vec<Tensor> = Vec::with_capacity(num_traj);
                let mut trajectory_rewards: Vec<f32> = Vec::with_capacity(num_traj);

                for _traj in 0..num_traj {
                    let mut selected: HashSet<usize> =
                        subgraph.seed_indices.iter().copied().collect();
                    let mut frontier = subgraph.frontier.clone();
                    let mut log_prob_sum: Option<Tensor> = None;

                    for _step in 0..num_steps {
                        if frontier.is_empty() { break; }

                        // Forward pass
                        let (_, policy_logits, _) = match model.forward(
                            &features, &subgraph.adjacency, &query_tensor,
                        ) {
                            Ok(v) => v,
                            Err(_) => break,
                        };

                        // Gather frontier logits → softmax → sample
                        let frontier_vec: Vec<i64> =
                            frontier.iter().map(|&i| i as i64).collect();
                        let frontier_indices = match Tensor::from_vec(
                            frontier_vec.clone(), frontier.len(), device,
                        ) {
                            Ok(t) => t,
                            Err(_) => break,
                        };

                        let frontier_logits = match policy_logits.index_select(&frontier_indices, 0) {
                            Ok(fl) => fl,
                            Err(_) => break,
                        };

                        let probs = match candle_nn::ops::softmax_last_dim(&frontier_logits) {
                            Ok(p) => p.flatten_all().unwrap().to_vec1::<f32>().unwrap(),
                            Err(_) => break,
                        };

                        // Sample c nodes without replacement from frontier
                        let n_pick = select_per_step.min(frontier.len());
                        let sampled_local: Vec<usize> =
                            sample_categorical_without_replacement(&probs, n_pick, &mut rng);

                        let picked: Vec<usize> = sampled_local.iter().map(|&li| frontier[li]).collect();

                        // Accumulate log-prob
                        let lp: f32 = sampled_local.iter().map(|&li| probs[li].ln()).sum();
                        let lp_tensor = Tensor::new(&[lp], device).unwrap();
                        log_prob_sum = match log_prob_sum {
                            Some(prev) => Some(prev.add(&lp_tensor).unwrap()),
                            None => Some(lp_tensor),
                        };

                        // Update selected + frontier
                        for &idx in &picked {
                            selected.insert(idx);
                        }
                        frontier = compute_frontier(&subgraph.adjacency, &selected);
                    }

                    // ── Reward: Recall@Any (label-based, no model involvement) ──
                    let hits = selected.iter().filter(|&&i| pos_set.contains(&i)).count();
                    let reward = hits as f32 / pos_set.len().max(1) as f32;

                    if let Some(lp) = log_prob_sum {
                        trajectory_log_probs.push(lp);
                        trajectory_rewards.push(reward);
                    }
                }

                if trajectory_log_probs.is_empty() { continue; }

                // ═══════════════════════════════════════════════
                // Step 5: Group-centered advantage + REINFORCE
                // ═══════════════════════════════════════════════

                let baseline: f32 = trajectory_rewards.iter().sum::<f32>()
                    / trajectory_rewards.len() as f32;

                let mut reinforce_loss: Option<Tensor> = None;
                for (traj_idx, lp_tensor) in trajectory_log_probs.iter().enumerate() {
                    let advantage = trajectory_rewards[traj_idx] - baseline;
                    let term = lp_tensor.affine(advantage as f64, 0.0).unwrap();
                    // Negative because REINFORCE maximizes, but optimizer minimizes
                    let term = term.neg().unwrap();
                    reinforce_loss = match reinforce_loss {
                        Some(prev) => Some(prev.add(&term).unwrap()),
                        None => Some(term),
                    };
                }
                let rl_loss = reinforce_loss.unwrap();
                let rl_loss = rl_loss
                    .affine(1.0 / trajectory_log_probs.len() as f64, 0.0).unwrap()
                    .affine(train_config.reinforce_weight, 0.0).unwrap();

                // ═══════════════════════════════════════════════
                // Step 6: BPR auxiliary loss
                // ═══════════════════════════════════════════════

                let (_, _, scores) = model.forward(
                    &features, &subgraph.adjacency, &query_tensor,
                )?;
                let bpr = bpr_loss(
                    &scores, &positive_indices, &subgraph.adjacency, device,
                )?;
                let bpr = bpr.affine(train_config.bpr_weight, 0.0).unwrap();

                let loss = rl_loss.add(&bpr).unwrap();

                epoch_rl_loss += rl_loss.to_vec0::<f64>().unwrap_or(0.0);
                epoch_bpr_loss += bpr.to_vec0::<f64>().unwrap_or(0.0);

                batch_loss = match batch_loss {
                    Some(bl) => Some(bl.add(&loss).unwrap()),
                    None => Some(loss),
                };
            }

            if let Some(ref loss) = batch_loss {
                optimizer.backward_step(loss)?;
                batches += 1;
            }
        }

        if batches > 0 {
            eprintln!(
                "epoch {:3}/{:3}  rl_loss={:.4}  bpr_loss={:.4}",
                epoch + 1,
                train_config.epochs,
                epoch_rl_loss / batches as f64,
                epoch_bpr_loss / batches as f64
            );
        }
    }

    Ok(())
}

/// Sample k indices without replacement from a categorical distribution.
fn sample_categorical_without_replacement(
    probs: &[f32],
    k: usize,
    rng: &mut impl rand::Rng,
) -> Vec<usize> {
    let n = probs.len();
    let k = k.min(n);
    if k == 0 { return vec![]; }

    // Build cumulative sum for sampling
    let total: f32 = probs.iter().sum();
    if total <= 0.0 {
        // Uniform if all probs are zero
        let mut indices: Vec<usize> = (0..n).collect();
        indices.shuffle(rng);
        indices.truncate(k);
        return indices;
    }

    let mut remaining_probs: Vec<f32> = probs.to_vec();
    let mut available: Vec<usize> = (0..n).collect();
    let mut picked = Vec::with_capacity(k);

    for _ in 0..k {
        let sum: f32 = remaining_probs.iter().sum();
        if sum <= 0.0 { break; }
        let r: f32 = rng.gen::<f32>() * sum;
        let mut cum = 0.0f32;
        let mut chosen_local = 0usize;
        for (local_idx, &p) in remaining_probs.iter().enumerate() {
            cum += p;
            if r < cum {
                chosen_local = local_idx;
                break;
            }
        }
        picked.push(available[chosen_local]);
        remaining_probs[chosen_local] = 0.0;
    }

    picked
}

/// Cosine similarity between two float slices.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let len = a.len().min(b.len());
    if len == 0 { return 0.0; }
    let (dot, norm_a, norm_b) = a[..len].iter().zip(&b[..len])
        .fold((0.0f64, 0.0f64, 0.0f64), |(d, na, nb), (&x, &y)| {
            (d + x as f64 * y as f64, na + x as f64 * x as f64, nb + y as f64 * y as f64)
        });
    if norm_a == 0.0 || norm_b == 0.0 { return 0.0; }
    dot / (norm_a.sqrt() * norm_b.sqrt())
}

/// Top-k cosine similarity retrieval for seed nodes.
fn top_k_cosine(
    query_emb: &[f32],
    all_ids: &[&str],
    all_embs: &[&[f32]],
    k: usize,
    node_lookup: &HashMap<&str, usize>,
    graph: &CodeGraph,
) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = all_ids.iter().enumerate()
        .filter_map(|(i, &id)| {
            let emb = all_embs[i];
            if emb.is_empty() { return None; }
            if let Some(&node_idx) = node_lookup.get(id) {
                if matches!(graph.nodes[node_idx].kind,
                    NodeKind::Repository | NodeKind::File | NodeKind::Import) {
                    return None;
                }
            }
            Some((cosine_similarity(query_emb, emb), id))
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(_, id)| id.to_string()).collect()
}

/// BPR loss: -log(sigmoid(score_pos - score_neg)) for each positive node.
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
        let neg_candidates: Vec<usize> = (0..n)
            .filter(|&i| !positive_indices.contains(&i) && i != pos_idx)
            .collect();
        if neg_candidates.is_empty() { continue; }
        let neg_idx = *neg_candidates.choose(&mut rng).unwrap();

        let pos_score = scores.get(pos_idx)?;
        let neg_score = scores.get(neg_idx)?;
        let diff = pos_score.sub(&neg_score)?;
        let sig = candle_nn::ops::sigmoid(&diff)?;
        total_loss = total_loss.add(&sig.log()?.neg()?)?;
        count += 1;
    }
    if count > 0 {
        total_loss = total_loss.affine(1.0 / count as f64, 0.0)?;
    }
    Ok(total_loss)
}

/// Build feature tensor [N, D] from node embeddings.
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
                for &v in emb.iter().take(dim) { data.push(v); }
                for _ in emb.len()..dim { data.push(0.0); }
            }
            None => {
                for _ in 0..dim { data.push(0.0); }
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
    }

    #[test]
    fn test_categorical_sampling() {
        let probs = vec![0.1f32, 0.5, 0.4];
        let mut rng = thread_rng();
        let picked = sample_categorical_without_replacement(&probs, 2, &mut rng);
        assert_eq!(picked.len(), 2);
        // Should all be valid indices
        for &p in &picked {
            assert!(p < 3);
        }
    }
}
