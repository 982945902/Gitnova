//! Graph Transformer GNN with policy head and scoring head for SeedER retrieval.
//!
//! Architecture (matches paper's modified SAN/Exphormer backbone):
//!   Input: node_features [N, D_in] + adjacency + query_embedding [D_in]
//!   Per layer:
//!     1. Multi-head self-attention with optional edge bias
//!     2. Query injection (learned projection added to each node)
//!     3. Feed-forward network (Linear → ReLU → Linear)
//!     4. Residual connections + LayerNorm after each sub-layer
//!   Policy head:  g_θ(h_u, z_q) = Linear(concat(h_u, z_q)) → [N, 1]
//!   Scoring head: ϕ_θ(h_v, z_q) = Linear(concat(h_v, z_q)) → sigmoid → [N, 1]
//!
//! The GNN operates on per-step induced subgraphs G_t[V_t ∪ U_t],
//! not the full bounded subgraph.

use candle_core::{Device, Result, Tensor};
#[cfg(test)]
use candle_core::DType;
use candle_nn::{layer_norm, linear, Linear, LayerNorm, Module, VarBuilder};

/// Trainer configuration matching inference SeedERConfig.
pub struct ModelConfig {
    pub input_dim: usize,
    pub hidden_dim: usize,
    pub num_layers: usize,
    pub num_heads: usize,     // attention heads per layer
    pub ff_dim: usize,         // feed-forward intermediate dim
    pub device: Device,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            input_dim: 64,
            hidden_dim: 128,
            num_layers: 3,
            num_heads: 4,
            ff_dim: 256,
            device: Device::Cpu,
        }
    }
}

/// One Graph Transformer layer.
struct TransformerLayer {
    // Self-attention
    q_proj: Linear,      // hidden_dim → hidden_dim
    k_proj: Linear,
    v_proj: Linear,
    out_proj: Linear,    // hidden_dim → hidden_dim
    // Query injection
    query_proj: Linear,  // hidden_dim → hidden_dim
    // Feed-forward
    ff1: Linear,         // hidden_dim → ff_dim
    ff2: Linear,         // ff_dim → hidden_dim
    // Layer norms (2: after attention, after FFN)
    ln1: LayerNorm,
    ln2: LayerNorm,
    num_heads: usize,
    head_dim: usize,
}

impl TransformerLayer {
    fn new(hidden_dim: usize, num_heads: usize, ff_dim: usize, vb: VarBuilder) -> Result<Self> {
        let head_dim = hidden_dim / num_heads;
        assert!(hidden_dim % num_heads == 0, "hidden_dim must be divisible by num_heads");
        Ok(Self {
            q_proj: linear(hidden_dim, hidden_dim, vb.pp("q"))?,
            k_proj: linear(hidden_dim, hidden_dim, vb.pp("k"))?,
            v_proj: linear(hidden_dim, hidden_dim, vb.pp("v"))?,
            out_proj: linear(hidden_dim, hidden_dim, vb.pp("o"))?,
            query_proj: linear(hidden_dim, hidden_dim, vb.pp("q_inj"))?,
            ff1: linear(hidden_dim, ff_dim, vb.pp("ff1"))?,
            ff2: linear(ff_dim, hidden_dim, vb.pp("ff2"))?,
            ln1: layer_norm(hidden_dim, 1e-5, vb.pp("ln1"))?,
            ln2: layer_norm(hidden_dim, 1e-5, vb.pp("ln2"))?,
            num_heads,
            head_dim,
        })
    }

    fn forward(&self, h: &Tensor, query_emb: &Tensor) -> Result<Tensor> {
        let n = h.dims()[0];
        let d = h.dims()[1];

        // ── Multi-head self-attention ──
        let q = self.q_proj.forward(h)?.reshape((n, self.num_heads, self.head_dim))?; // [N, H, D_h]
        let k = self.k_proj.forward(h)?.reshape((n, self.num_heads, self.head_dim))?; // [N, H, D_h]
        let v = self.v_proj.forward(h)?.reshape((n, self.num_heads, self.head_dim))?; // [N, H, D_h]

        // QK^T: [N, H, D_h] × [N, H, D_h]ᵀ via transpose
        // q: [N, H, D_h] → need [H, N, D_h] for bmm
        let q_t = q.permute((1, 0, 2))?; // [H, N, D_h]
        let k_t = k.permute((1, 0, 2))?; // [H, N, D_h]
        let v_t = v.permute((1, 0, 2))?; // [H, N, D_h]

        // Scale dot-product attention
        let scale = (self.head_dim as f64).sqrt();
        let attn_scores = q_t.matmul(&k_t.transpose(1, 2)?)?; // [H, N, N]
        let attn_scores = attn_scores.affine(1.0 / scale, 0.0)?;
        let attn_weights = candle_nn::ops::softmax_last_dim(&attn_scores)?; // [H, N, N]
        let attn_out = attn_weights.matmul(&v_t)?; // [H, N, D_h]
        let attn_out = attn_out.permute((1, 0, 2))?; // [N, H, D_h]
        let attn_out = attn_out.reshape((n, d))?; // [N, D]
        let attn_out = self.out_proj.forward(&attn_out)?;

        // ── Residual + LayerNorm ──
        let h = self.ln1.forward(&h.add(&attn_out)?)?;

        // ── Query injection ──
        let q_inj = self.query_proj.forward(query_emb)?; // [1, D]
        let q_broadcast = q_inj.expand((n, d))?;
        let h = h.add(&q_broadcast)?;

        // ── Feed-forward ──
        let ff = self.ff2.forward(&self.ff1.forward(&h)?.relu()?)?;
        let h = self.ln2.forward(&h.add(&ff)?)?;

        Ok(h)
    }
}

/// Graph Transformer model with policy head and scoring head.
pub struct GraphTransformerModel {
    input_proj: Option<Linear>,         // input_dim → hidden_dim (None if dims match)
    layers: Vec<TransformerLayer>,
    // Policy head: g_θ(h_u, z_q) — explicit dual input
    policy_proj_h: Linear,              // hidden_dim → hidden_dim
    policy_proj_q: Linear,              // hidden_dim → hidden_dim
    policy_out: Linear,                 // hidden_dim → 1
    // Scoring head: ϕ_θ(h_v, z_q) — explicit dual input
    scoring_proj_h: Linear,             // hidden_dim → hidden_dim
    scoring_proj_q: Linear,             // hidden_dim → hidden_dim
    scoring_out: Linear,                // hidden_dim → 1
    config: ModelConfig,
}

impl GraphTransformerModel {
    pub fn new(config: &ModelConfig, vb: VarBuilder) -> Result<Self> {
        let input_proj = if config.input_dim != config.hidden_dim {
            Some(linear(config.input_dim, config.hidden_dim, vb.pp("input_proj"))?)
        } else {
            None
        };

        let mut layers = Vec::with_capacity(config.num_layers);
        for i in 0..config.num_layers {
            layers.push(TransformerLayer::new(
                config.hidden_dim, config.num_heads, config.ff_dim,
                vb.pp(format!("layer_{i}")),
            )?);
        }

        Ok(Self {
            input_proj,
            layers,
            policy_proj_h: linear(config.hidden_dim, config.hidden_dim, vb.pp("pol_h"))?,
            policy_proj_q: linear(config.hidden_dim, config.hidden_dim, vb.pp("pol_q"))?,
            policy_out: linear(config.hidden_dim, 1, vb.pp("pol_out"))?,
            scoring_proj_h: linear(config.hidden_dim, config.hidden_dim, vb.pp("sc_h"))?,
            scoring_proj_q: linear(config.hidden_dim, config.hidden_dim, vb.pp("sc_q"))?,
            scoring_out: linear(config.hidden_dim, 1, vb.pp("sc_out"))?,
            config: ModelConfig {
                input_dim: config.input_dim,
                hidden_dim: config.hidden_dim,
                num_layers: config.num_layers,
                num_heads: config.num_heads,
                ff_dim: config.ff_dim,
                device: vb.device().clone(),
            },
        })
    }

    /// Full forward pass on induced subgraph G_t[V_t ∪ U_t].
    ///
    /// Returns (hidden_states [N, D_hid], policy_logits [N, 1], scores [N, 1]).
    pub fn forward(
        &self,
        node_features: &Tensor,  // [N, D_in] — only V_t ∪ U_t nodes
        query_embedding: &Tensor, // [D_in]
    ) -> Result<(Tensor, Tensor, Tensor)> {
        let n = node_features.dims()[0];

        // Input projection
        let mut h = match &self.input_proj {
            Some(proj) => proj.forward(node_features)?,
            None => node_features.clone(),
        };

        // Project query to hidden dim once
        let q: Tensor = match &self.input_proj {
            Some(proj) => proj.forward(&query_embedding.unsqueeze(0)?)?,
            None => query_embedding.unsqueeze(0)?,
        };

        // Transformer layers
        for layer in &self.layers {
            h = layer.forward(&h, &q)?;
        }

        // ── Policy head: g_θ(h_u, z_q) ──
        let h_pol = self.policy_proj_h.forward(&h)?;
        let q_pol = self.policy_proj_q.forward(&q)?.expand((n, self.config.hidden_dim))?;
        let policy_logits = self.policy_out.forward(&(h_pol.add(&q_pol)?.relu()?))?;

        // ── Scoring head: ϕ_θ(h_v, z_q) ──
        let h_sc = self.scoring_proj_h.forward(&h)?;
        let q_sc = self.scoring_proj_q.forward(&q)?.expand((n, self.config.hidden_dim))?;
        let scores = candle_nn::ops::sigmoid(&self.scoring_out.forward(&(h_sc.add(&q_sc)?.relu()?))?)?;

        Ok((h, policy_logits, scores))
    }

    /// Greedy frontier expansion: score frontier, pick top-c.
    /// `frontier` contains indices into the current induced subgraph nodes.
    pub fn expand_step(
        &self,
        node_features: &Tensor,    // [N, D_in] — V_t ∪ U_t
        query_embedding: &Tensor,  // [D_in]
        frontier: &[usize],        // indices within node_features
        top_c: usize,
    ) -> Result<(Vec<usize>, Tensor)> {
        let (_h, policy_logits, _scores) =
            self.forward(node_features, query_embedding)?;

        let frontier_indices = Tensor::from_vec(
            frontier.iter().map(|&i| i as i64).collect::<Vec<_>>(),
            frontier.len(),
            &self.config.device,
        )?;
        let frontier_logits = policy_logits.index_select(&frontier_indices, 0)?;

        let flat: Vec<f32> = frontier_logits.flatten_all()?.to_vec1()?;
        let mut indexed: Vec<(usize, f32)> = frontier.iter().copied().zip(flat).collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let selected: Vec<usize> = indexed.iter().take(top_c).map(|(i, _)| *i).collect();
        Ok((selected, frontier_logits))
    }

    /// Compute frontier policy distribution (softmax over frontier logits).
    pub fn frontier_policy(
        &self,
        node_features: &Tensor,
        query_embedding: &Tensor,
        frontier: &[usize],
    ) -> Result<Tensor> {
        let (_h, policy_logits, _scores) =
            self.forward(node_features, query_embedding)?;

        let frontier_indices = Tensor::from_vec(
            frontier.iter().map(|&i| i as i64).collect::<Vec<_>>(),
            frontier.len(),
            &self.config.device,
        )?;
        let frontier_logits = policy_logits.index_select(&frontier_indices, 0)?;
        candle_nn::ops::softmax_last_dim(&frontier_logits)
    }

    /// Score all nodes for final ranking.
    pub fn score_nodes(
        &self,
        node_features: &Tensor,
        query_embedding: &Tensor,
    ) -> Result<Vec<f32>> {
        let (_h, _policy, scores) = self.forward(node_features, query_embedding)?;
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
            num_heads: 4,
            ff_dim: 32,
            device: Device::Cpu,
        };

        let varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &config.device);
        let model = GraphTransformerModel::new(&config, vb).unwrap();

        let features = Tensor::randn(0f32, 1f32, (5, 8), &config.device).unwrap();
        let query = Tensor::randn(0f32, 1f32, 8, &config.device).unwrap();

        let (h, policy_logits, scores) = model.forward(&features, &query).unwrap();
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
            num_heads: 2,
            ff_dim: 16,
            device: Device::Cpu,
        };

        let varmap = VarMap::new();
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, &config.device);
        let model = GraphTransformerModel::new(&config, vb).unwrap();

        let features = Tensor::ones((3, 4), DType::F32, &config.device).unwrap();
        let query = Tensor::ones(4, DType::F32, &config.device).unwrap();
        let frontier = vec![1usize, 2];

        let (selected, _) = model
            .expand_step(&features, &query, &frontier, 1)
            .unwrap();
        assert_eq!(selected.len(), 1);
    }
}
