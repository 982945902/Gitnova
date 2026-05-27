//! REINFORCE + BPR training loop for SeedER GNN model.
//!
//! Each expansion step builds the induced subgraph G_t[V_t ∪ U_t]
//! (per the paper's Algorithm 1 line 6), not the full bounded subgraph.

use crate::data::TrainingExample;
use crate::model::{GraphTransformerModel, ModelConfig};
use crate::subgraph::{compute_frontier, extract_subgraph};
use anyhow::Result;
use candle_core::{DType, Device, Tensor};
use candle_nn::{AdamW, Optimizer, ParamsAdamW, VarMap};
use gitnova_core::model::{CodeGraph, NodeKind};
use rand::seq::SliceRandom;
use rand::thread_rng;
use std::collections::{HashMap, HashSet};

pub struct TrainConfig {
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f64,
    pub num_trajectories: usize,
    pub expand_steps: usize,
    pub frontier_select: usize,
    pub reinforce_weight: f64,
    pub bpr_weight: f64,
    pub seed_k: usize,
    pub subgraph_max_nodes: usize,
    pub subgraph_hops: usize,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            epochs: 50, batch_size: 4, learning_rate: 0.005,
            num_trajectories: 8, expand_steps: 3, frontier_select: 5,
            reinforce_weight: 1.0, bpr_weight: 0.5,
            seed_k: 10, subgraph_max_nodes: 100, subgraph_hops: 2,
        }
    }
}

pub fn train(
    examples: &[TrainingExample],
    graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>,
    model: &mut GraphTransformerModel,
    varmap: &VarMap,
    model_config: &ModelConfig,
    train_config: &TrainConfig,
) -> Result<()> {
    let device = model.device();

    let node_lookup: HashMap<&str, usize> = graph
        .nodes.iter().enumerate().map(|(i, n)| (n.id.as_str(), i)).collect();

    let all_embedding_ids: Vec<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let all_embeddings: Vec<&[f32]> = graph.nodes.iter()
        .map(|n| node_embeddings.get(&n.id).map(|v| v.as_slice()).unwrap_or(&[]))
        .collect();

    let adam_params = ParamsAdamW { lr: train_config.learning_rate, weight_decay: 0.0, ..Default::default() };
    let mut optimizer = AdamW::new(varmap.all_vars(), adam_params)?;
    let mut rng = thread_rng();
    let total_examples = examples.len();

    for epoch in 0..train_config.epochs {
        let mut indices: Vec<usize> = (0..total_examples).collect();
        indices.shuffle(&mut rng);
        let mut epoch_rl = 0.0f64;
        let mut epoch_bpr = 0.0f64;
        let mut batches = 0usize;

        for batch_start in (0..total_examples).step_by(train_config.batch_size) {
            let batch_end = (batch_start + train_config.batch_size).min(total_examples);
            let mut batch_loss: Option<Tensor> = None;

            for &idx in &indices[batch_start..batch_end] {
                let example = &examples[idx];

                let seed_ids = top_k_cosine(&example.query_embedding, &all_embedding_ids,
                    &all_embeddings, train_config.seed_k, &node_lookup, graph);
                if seed_ids.is_empty() { continue; }

                let subgraph = extract_subgraph(graph, &seed_ids,
                    train_config.subgraph_max_nodes, train_config.subgraph_hops);
                if subgraph.node_ids.len() < 5 { continue; }

                // Pre-build full subgraph features [N_full, D]
                let full_features = match build_feature_tensor(
                    &subgraph.node_index, graph, node_embeddings, model_config.input_dim, device,
                ) { Ok(f) => f, Err(_) => continue };

                let query_tensor = match Tensor::from_vec(
                    example.query_embedding.clone(), example.query_embedding.len(), device,
                ) { Ok(q) => q, Err(_) => continue };

                let positive_set: HashSet<&str> =
                    example.positive_ids.iter().map(|s| s.as_str()).collect();
                let positive_indices: Vec<usize> = subgraph.node_ids.iter().enumerate()
                    .filter(|(_, id)| positive_set.contains(id.as_str()))
                    .map(|(i, _)| i).collect();
                if positive_indices.is_empty() { continue; }
                let pos_set: HashSet<usize> = positive_indices.iter().copied().collect();

                // ═══════ Sample M trajectories ═══════
                let num_traj = train_config.num_trajectories;
                let num_steps = train_config.expand_steps;
                let select_per_step = train_config.frontier_select;
                let mut traj_log_probs: Vec<Tensor> = Vec::with_capacity(num_traj);
                let mut traj_rewards: Vec<f32> = Vec::with_capacity(num_traj);

                for _traj in 0..num_traj {
                    // selected/frontier: indices into bounded subgraph nodes
                    let mut selected: HashSet<usize> =
                        subgraph.seed_indices.iter().copied().collect();
                    let mut frontier = subgraph.frontier.clone();
                    let mut log_prob_sum: Option<Tensor> = None;

                    for _step in 0..num_steps {
                        if frontier.is_empty() { break; }

                        // ── Build induced subgraph G_t = G_q[V_t ∪ U_t] ──
                        let induced_indices = build_induced_indices(&selected, &frontier);
                        let induced_features = gather_features(&full_features, &induced_indices)?;
                        // Map frontier to positions within induced subgraph
                        let frontier_local: Vec<usize> = frontier.iter()
                            .map(|&f| induced_indices.iter().position(|&i| i == f).unwrap())
                            .collect();

                        let (_, policy_logits, _) = model.forward(
                            &induced_features, &query_tensor,
                        )?;

                        let frontier_idx_tensor = Tensor::from_vec(
                            frontier_local.iter().map(|&i| i as i64).collect(),
                            frontier_local.len(), device,
                        )?;
                        let f_logits = policy_logits.index_select(&frontier_idx_tensor, 0)?;
                        let probs = match candle_nn::ops::softmax_last_dim(&f_logits) {
                            Ok(p) => p.flatten_all().unwrap().to_vec1::<f32>().unwrap(),
                            Err(_) => break,
                        };

                        let n_pick = select_per_step.min(frontier_local.len());
                        let sampled_local = sample_categorical_without_replacement(&probs, n_pick, &mut rng);
                        // Map back: local frontier index → subgraph node index
                        let picked: Vec<usize> = sampled_local.iter().map(|&li| frontier[li]).collect();

                        let lp: f32 = sampled_local.iter().map(|&li| probs[li].ln()).sum();
                        let lp_tensor = Tensor::new(&[lp], device).unwrap();
                        log_prob_sum = match log_prob_sum {
                            Some(prev) => Some(prev.add(&lp_tensor).unwrap()),
                            None => Some(lp_tensor),
                        };

                        for &idx in &picked { selected.insert(idx); }
                        frontier = compute_frontier(&subgraph.adjacency, &selected);
                    }

                    // ── Reward: Recall@Any (label-based) ──
                    let hits = selected.iter().filter(|&&i| pos_set.contains(&i)).count();
                    let reward = hits as f32 / pos_set.len().max(1) as f32;

                    if let Some(lp) = log_prob_sum {
                        traj_log_probs.push(lp);
                        traj_rewards.push(reward);
                    }
                }

                if traj_log_probs.is_empty() { continue; }

                // ═══════ REINFORCE ═══════
                let baseline: f32 = traj_rewards.iter().sum::<f32>() / traj_rewards.len() as f32;
                let mut rl_loss: Option<Tensor> = None;
                for (ti, lp) in traj_log_probs.iter().enumerate() {
                    let advantage = traj_rewards[ti] - baseline;
                    let term = lp.affine(advantage as f64, 0.0).unwrap().neg().unwrap();
                    rl_loss = match rl_loss {
                        Some(prev) => Some(prev.add(&term).unwrap()),
                        None => Some(term),
                    };
                }
                let rl_loss = rl_loss.unwrap()
                    .affine(1.0 / traj_log_probs.len() as f64, 0.0).unwrap()
                    .affine(train_config.reinforce_weight, 0.0).unwrap();

                // ═══════ BPR auxiliary (on FULL subgraph for cold-start signal) ═══════
                let (_, _, scores) = model.forward(&full_features, &query_tensor)?;
                let bpr_raw = bpr_loss(&scores, &positive_indices, device)?;
                let bpr = bpr_raw.affine(train_config.bpr_weight, 0.0).unwrap();
                let loss = rl_loss.add(&bpr).unwrap();

                let rl_val = rl_loss.flatten_all().unwrap().to_vec1::<f32>().unwrap_or_default().first().copied().unwrap_or(0.0);
                let bpr_val = bpr_raw.flatten_all().unwrap().to_vec1::<f32>().unwrap_or_default().first().copied().unwrap_or(0.0);
                epoch_rl += rl_val as f64;
                epoch_bpr += bpr_val as f64;

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
            eprintln!("epoch {:3}/{:3}  rl={:.4}  bpr={:.4}",
                epoch + 1, train_config.epochs,
                epoch_rl / batches as f64, epoch_bpr / batches as f64);
        }
    }

    Ok(())
}

/// Build the ordered list of node indices for induced subgraph G_t[V_t ∪ U_t].
fn build_induced_indices(selected: &HashSet<usize>, frontier: &[usize]) -> Vec<usize> {
    let mut indices: Vec<usize> = selected.iter().copied().collect();
    for &f in frontier {
        if !selected.contains(&f) {
            indices.push(f);
        }
    }
    indices
}

/// Gather feature rows for the given subset of node indices.
fn gather_features(full_features: &Tensor, indices: &[usize]) -> candle_core::Result<Tensor> {
    let idx_tensor = Tensor::from_vec(
        indices.iter().map(|&i| i as i64).collect::<Vec<_>>(),
        indices.len(), full_features.device(),
    )?;
    full_features.index_select(&idx_tensor, 0)
}

fn sample_categorical_without_replacement(
    probs: &[f32], k: usize, rng: &mut impl rand::Rng,
) -> Vec<usize> {
    let n = probs.len();
    let k = k.min(n);
    if k == 0 { return vec![]; }
    let total: f32 = probs.iter().sum();
    if total <= 0.0 {
        let mut indices: Vec<usize> = (0..n).collect();
        indices.shuffle(rng);
        indices.truncate(k);
        return indices;
    }
    let mut remaining: Vec<f32> = probs.to_vec();
    let mut picked = Vec::with_capacity(k);
    for _ in 0..k {
        let sum: f32 = remaining.iter().sum();
        if sum <= 0.0 { break; }
        let r: f32 = rng.gen::<f32>() * sum;
        let mut cum = 0.0;
        for (i, &p) in remaining.iter().enumerate() {
            cum += p;
            if r < cum { picked.push(i); remaining[i] = 0.0; break; }
        }
    }
    picked
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let len = a.len().min(b.len());
    if len == 0 { return 0.0; }
    let (dot, na, nb) = a[..len].iter().zip(&b[..len])
        .fold((0.0f64, 0.0f64, 0.0f64), |(d, na, nb), (&x, &y)| {
            (d + x as f64 * y as f64, na + x as f64 * x as f64, nb + y as f64 * y as f64)
        });
    if na == 0.0 || nb == 0.0 { return 0.0; }
    dot / (na.sqrt() * nb.sqrt())
}

fn top_k_cosine(
    query_emb: &[f32], all_ids: &[&str], all_embs: &[&[f32]],
    k: usize, node_lookup: &HashMap<&str, usize>, graph: &CodeGraph,
) -> Vec<String> {
    let mut scored: Vec<(f64, &str)> = all_ids.iter().enumerate()
        .filter_map(|(i, &id)| {
            let emb = all_embs[i];
            if emb.is_empty() { return None; }
            if let Some(&ni) = node_lookup.get(id) {
                if matches!(graph.nodes[ni].kind,
                    NodeKind::Repository | NodeKind::File | NodeKind::Import) { return None; }
            }
            Some((cosine_similarity(query_emb, emb), id))
        }).collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);
    scored.into_iter().map(|(_, id)| id.to_string()).collect()
}

fn bpr_loss(
    scores: &Tensor, positive_indices: &[usize], device: &Device,
) -> candle_core::Result<Tensor> {
    let n = scores.dims()[0];
    if positive_indices.is_empty() || n < 2 {
        return Ok(Tensor::zeros((1,), DType::F32, device)?);
    }
    let mut rng = thread_rng();
    let mut total = Tensor::zeros((1,), DType::F32, device)?;
    let mut count = 0usize;
    for &pos in positive_indices {
        let negs: Vec<usize> = (0..n).filter(|&i| !positive_indices.contains(&i) && i != pos).collect();
        if negs.is_empty() { continue; }
        let neg = *negs.choose(&mut rng).unwrap();
        let pos_score = scores.get(pos)?;
        let neg_score = scores.get(neg)?;
        let diff = pos_score.sub(&neg_score)?;
        total = total.add(&candle_nn::ops::sigmoid(&diff)?.log()?.neg()?)?;
        count += 1;
    }
    if count > 0 { total = total.affine(1.0 / count as f64, 0.0)?; }
    Ok(total)
}

fn build_feature_tensor(
    node_indices: &[usize], graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>, dim: usize, device: &Device,
) -> candle_core::Result<Tensor> {
    let mut data = Vec::with_capacity(node_indices.len() * dim);
    for &gi in node_indices {
        let nid = &graph.nodes[gi].id;
        match node_embeddings.get(nid) {
            Some(emb) => {
                for &v in emb.iter().take(dim) { data.push(v); }
                for _ in emb.len()..dim { data.push(0.0); }
            }
            None => { for _ in 0..dim { data.push(0.0); } }
        }
    }
    Tensor::from_vec(data, (node_indices.len(), dim), device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine() {
        assert!((cosine_similarity(&[1.0,0.0,0.0], &[1.0,0.0,0.0]) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_categorical() {
        let mut rng = thread_rng();
        let p = sample_categorical_without_replacement(&[0.1, 0.5, 0.4], 2, &mut rng);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn test_bpr_loss_nonzero() {
        let dev = &Device::Cpu;
        let scores = Tensor::from_vec(vec![0.1f32, 0.2, 0.9, 0.3], (4, 1), dev).unwrap();
        let pos = vec![2usize];
        let loss = bpr_loss(&scores, &pos, dev).unwrap();
        let val = loss.flatten_all().unwrap().to_vec1::<f32>().unwrap()[0];
        assert!(val > 0.0, "BPR loss should be > 0, got {val}");
    }

    #[test]
    fn test_model_scores_are_not_uniform() {
        use crate::model::{GraphTransformerModel, ModelConfig};
        use candle_core::DType;
        use candle_nn::{VarBuilder, VarMap};
        let config = ModelConfig {
            input_dim: 8, hidden_dim: 16, num_layers: 2,
            num_heads: 2, ff_dim: 32, device: Device::Cpu,
        };
        let varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &config.device);
        let model = GraphTransformerModel::new(&config, vb).unwrap();
        let features = Tensor::randn(0f32, 1f32, (10, 8), &config.device).unwrap();
        let query = Tensor::randn(0f32, 1f32, 8, &config.device).unwrap();
        let (_, _, scores) = model.forward(&features, &query).unwrap();
        let sv: Vec<f32> = scores.flatten_all().unwrap().to_vec1().unwrap();
        let (min, max) = (sv.iter().cloned().fold(f32::INFINITY, f32::min),
                          sv.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
        let range = max - min;
        eprintln!("model scores range: {:.6}", range);
        assert!(range > 0.001, "Model scores should vary; all ~{:.6}", sv[0]);
    }
}
