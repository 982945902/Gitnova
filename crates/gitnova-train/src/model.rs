//! GraphSAGE GNN model with policy head and scoring head for SeedER retrieval.
//!
//! Architecture:
//!   Input: node_features [N, D_in] + adjacency + query_embedding [D_in]
//!   Per layer: h = LayerNorm(ReLU(W_self·h + W_nei·mean(h[u] for u in N(v)) + W_q·q))
//!   Policy head: Linear(D_hid, 1) → softmax over frontier
//!   Scoring head: Linear(D_hid, 1) → sigmoid

#[cfg(test)]
use candle_core::DType;
use candle_core::{Device, Result, Tensor};
use candle_nn::{layer_norm, linear, Linear, LayerNorm, Module, VarBuilder};

/// Trainer configuration matching inference SeedERConfig.
pub struct ModelConfig {
    pub input_dim: usize,
    pub hidden_dim: usize,
    pub num_layers: usize,
    pub device: Device,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            input_dim: 64,
            hidden_dim: 128,
            num_layers: 3,
            device: Device::Cpu,
        }
    }
}

/// One GraphSAGE convolution layer with query injection.
struct SAGELayer {
    self_linear: Linear,
    neighbor_linear: Linear,
    query_linear: Linear,
    layer_norm: LayerNorm,
}

impl SAGELayer {
    fn new(dim: usize, vb: VarBuilder) -> Result<Self> {
        let self_linear = linear(dim, dim, vb.pp("self"))?;
        let neighbor_linear = linear(dim, dim, vb.pp("neighbor"))?;
        let query_linear = linear(dim, dim, vb.pp("query"))?;
        let layer_norm = layer_norm(dim, 1e-5, vb.pp("ln"))?;
        Ok(Self { self_linear, neighbor_linear, query_linear, layer_norm })
    }

    fn forward(
        &self,
        h: &Tensor,             // [N, D]
        neighbor_mean: &Tensor, // [N, D]
        query_emb: &Tensor,     // [1, D]
    ) -> Result<Tensor> {
        let h_self = self.self_linear.forward(h)?;
        let h_nei = self.neighbor_linear.forward(neighbor_mean)?;
        let n = h.dims()[0];
        let d = h.dims()[1];
        let query_broadcast = self.query_linear.forward(query_emb)?.expand((n, d))?;
        let combined = h_self.add(&h_nei)?.add(&query_broadcast)?;
        let activated = combined.relu()?;
        self.layer_norm.forward(&activated)
    }
}

/// GraphSAGE model with policy head and scoring head.
pub struct GraphSAGEModel {
    input_proj: Option<Linear>,       // input_dim → hidden_dim (None if dims match)
    sage_layers: Vec<SAGELayer>,
    policy_head: Linear,              // hidden_dim → 1
    scoring_head: Linear,             // hidden_dim → 1
    config: ModelConfig,
}

impl GraphSAGEModel {
    /// Create a new randomly-initialized model.
    pub fn new(config: &ModelConfig, vb: VarBuilder) -> Result<Self> {
        let input_proj = if config.input_dim != config.hidden_dim {
            Some(linear(config.input_dim, config.hidden_dim, vb.pp("input_proj"))?)
        } else {
            None
        };

        let mut sage_layers = Vec::with_capacity(config.num_layers);
        for i in 0..config.num_layers {
            sage_layers.push(SAGELayer::new(config.hidden_dim, vb.pp(format!("sage_{i}")))?);
        }

        let policy_head = linear(config.hidden_dim, 1, vb.pp("policy_head"))?;
        let scoring_head = linear(config.hidden_dim, 1, vb.pp("scoring_head"))?;

        Ok(Self {
            input_proj,
            sage_layers,
            policy_head,
            scoring_head,
            config: ModelConfig {
                input_dim: config.input_dim,
                hidden_dim: config.hidden_dim,
                num_layers: config.num_layers,
                device: vb.device().clone(),
            },
        })
    }

    /// Full forward pass: node embeddings → hidden states.
    ///
    /// Returns (hidden_states [N, D_hid], policy_logits [N, 1], scores [N, 1]).
    pub fn forward(
        &self,
        node_features: &Tensor,  // [N, D_in]
        adjacency: &[Vec<usize>], // neighbor indices per node
        query_embedding: &Tensor, // [D_in]
    ) -> Result<(Tensor, Tensor, Tensor)> {
        let n = node_features.dims()[0];
        let q = query_embedding.unsqueeze(0)?; // [1, D_in]

        // Optional input projection
        let mut h = match &self.input_proj {
            Some(proj) => proj.forward(node_features)?,
            None => node_features.clone(),
        };

        // Build dense adjacency matrix for neighbor mean aggregation: [N, N]
        let adj_matrix = self.build_adj_matrix(adjacency, n)?;

        for layer in &self.sage_layers {
            // Neighbor mean: adj_matrix @ h → [N, D]
            let neighbor_mean = adj_matrix.matmul(&h)?;
            // Project query embedding: reuse input_proj if present
            let q_proj = match &self.input_proj {
                Some(proj) => proj.forward(&q)?,
                None => q.clone(),
            };
            h = layer.forward(&h, &neighbor_mean, &q_proj)?;
        }

        let policy_logits = self.policy_head.forward(&h)?;   // [N, 1]
        let scores = candle_nn::ops::sigmoid(&self.scoring_head.forward(&h)?)?; // [N, 1]

        Ok((h, policy_logits, scores))
    }

    /// Greedy frontier expansion: pick top-c nodes by policy logits.
    pub fn expand_step(
        &self,
        node_features: &Tensor,
        adjacency: &[Vec<usize>],
        query_embedding: &Tensor,
        frontier: &[usize],
        top_c: usize,
    ) -> Result<(Vec<usize>, Tensor)> {
        let (_h, policy_logits, _scores) =
            self.forward(node_features, adjacency, query_embedding)?;

        // Gather logits for frontier nodes only
        let _n = node_features.dims()[0];
        let frontier_indices = Tensor::from_vec(
            frontier.iter().map(|&i| i as i64).collect::<Vec<_>>(),
            frontier.len(),
            &self.config.device,
        )?;
        let frontier_logits = policy_logits.index_select(&frontier_indices, 0)?; // [F, 1]

        // Flatten and sort descending
        let flat: Vec<f32> = frontier_logits.flatten_all()?.to_vec1()?;
        let mut indexed: Vec<(usize, f32)> = frontier.iter().copied().zip(flat).collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let selected: Vec<usize> = indexed.iter().take(top_c).map(|(i, _)| *i).collect();

        // Return selected indices
        Ok((selected, frontier_logits))
    }

    /// Build a normalized adjacency matrix [N, N] for mean neighbor aggregation.
    fn build_adj_matrix(&self, adjacency: &[Vec<usize>], n: usize) -> Result<Tensor> {
        let mut data = vec![0.0f32; n * n];
        for (i, neighbors) in adjacency.iter().enumerate() {
            if !neighbors.is_empty() {
                let weight = 1.0 / neighbors.len() as f32;
                for &j in neighbors {
                    data[i * n + j] = weight;
                }
            }
        }
        Tensor::from_vec(data, (n, n), &self.config.device)
    }

    /// Compute frontier policy distribution (softmax over frontier logits).
    pub fn frontier_policy(
        &self,
        node_features: &Tensor,
        adjacency: &[Vec<usize>],
        query_embedding: &Tensor,
        frontier: &[usize],
    ) -> Result<Tensor> {
        // For small subgraphs, do full forward. For efficiency could cache hidden states.
        let (_h, policy_logits, _scores) =
            self.forward(node_features, adjacency, query_embedding)?;

        let frontier_indices = Tensor::from_vec(
            frontier.iter().map(|&i| i as i64).collect::<Vec<_>>(),
            frontier.len(),
            &self.config.device,
        )?;
        let frontier_logits = policy_logits.index_select(&frontier_indices, 0)?; // [F, 1]
        candle_nn::ops::softmax_last_dim(&frontier_logits)
    }

    /// Score all nodes for final ranking.
    pub fn score_nodes(
        &self,
        node_features: &Tensor,
        adjacency: &[Vec<usize>],
        query_embedding: &Tensor,
    ) -> Result<Vec<f32>> {
        let (_h, _policy, scores) = self.forward(node_features, adjacency, query_embedding)?;
        scores.flatten_all()?.to_vec1()
    }

    pub fn device(&self) -> &Device {
        &self.config.device
    }

    pub fn config(&self) -> &ModelConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use candle_nn::VarMap;

    #[test]
    fn test_forward_shapes() {
        let config = ModelConfig {
            input_dim: 8,
            hidden_dim: 16,
            num_layers: 2,
            device: Device::Cpu,
        };

        let varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &config.device);
        let model = GraphSAGEModel::new(&config, vb).unwrap();

        // 5 nodes, 8-dim features
        let features = Tensor::randn(0f32, 1f32, (5, 8), &config.device).unwrap();
        let query = Tensor::randn(0f32, 1f32, 8, &config.device).unwrap();
        let adj: Vec<Vec<usize>> = vec![
            vec![1, 2],
            vec![0, 3],
            vec![0],
            vec![1, 4],
            vec![3],
        ];

        let (h, policy_logits, scores) = model.forward(&features, &adj, &query).unwrap();
        assert_eq!(h.dims(), &[5, 16]);
        assert_eq!(policy_logits.dims(), &[5, 1]);
        assert_eq!(scores.dims(), &[5, 1]);
    }

    #[test]
    fn test_greedy_expansion() {
        let config = ModelConfig {
            input_dim: 4,
            hidden_dim: 8,
            num_layers: 1,
            device: Device::Cpu,
        };

        let varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &config.device);
        let model = GraphSAGEModel::new(&config, vb).unwrap();

        let features = Tensor::ones((3, 4), DType::F32, &config.device).unwrap();
        let query = Tensor::ones(4, DType::F32, &config.device).unwrap();
        let adj: Vec<Vec<usize>> = vec![vec![1], vec![0, 2], vec![1]];
        let frontier = vec![1, 2];

        let (selected, _) = model
            .expand_step(&features, &adj, &query, &frontier, 1)
            .unwrap();
        assert_eq!(selected.len(), 1);
    }
}
