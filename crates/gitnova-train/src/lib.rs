pub mod data;
pub mod inference;
pub mod model;
pub mod subgraph;
pub mod training;

use anyhow::{Context, Result};
use candle_core::DType;
use candle_nn::{VarBuilder, VarMap};
use gitnova_core::model::CodeGraph;
use std::collections::HashMap;
use std::path::Path;

pub use model::ModelConfig;
use model::GraphTransformerModel;

#[derive(Debug, Clone)]
pub struct SeedERConfig {
    pub k_seed: usize,
    pub expand_steps: usize,
    pub frontier_select: usize,
    pub subgraph_max_nodes: usize,
    pub subgraph_hops: usize,
}

impl Default for SeedERConfig {
    fn default() -> Self {
        Self { k_seed: 10, expand_steps: 3, frontier_select: 5,
               subgraph_max_nodes: 200, subgraph_hops: 2 }
    }
}

pub struct SeedERRetriever {
    model: GraphTransformerModel,
    retriever_config: SeedERConfig,
}

impl SeedERRetriever {
    pub fn load(
        path: impl AsRef<Path>, model_config: &ModelConfig, retriever_config: SeedERConfig,
    ) -> Result<Self> {
        let device = &model_config.device;
        let mut varmap = VarMap::new();
        varmap.load(path.as_ref()).context("Failed to load model")?;
        let vb = VarBuilder::from_varmap(&varmap, DType::F32, device);
        let model = GraphTransformerModel::new(model_config, vb)?;
        Ok(Self { model, retriever_config })
    }

    pub fn retrieve(
        &self, graph: &CodeGraph, query_embedding: &[f32],
        node_embeddings: &HashMap<String, Vec<f32>>, limit: usize,
    ) -> Vec<String> {
        inference::seed_expand_retrieve(
            &self.model, graph, query_embedding, node_embeddings,
            self.retriever_config.k_seed, self.retriever_config.expand_steps,
            self.retriever_config.frontier_select,
            self.retriever_config.subgraph_max_nodes,
            self.retriever_config.subgraph_hops, limit,
        )
    }
}

pub fn train_and_save(
    examples: &[data::TrainingExample], graph: &CodeGraph,
    node_embeddings: &HashMap<String, Vec<f32>>,
    model_config: &ModelConfig, train_config: &training::TrainConfig,
    save_path: impl AsRef<Path>,
) -> Result<()> {
    let device = &model_config.device;
    let varmap = VarMap::new();
    let vb = VarBuilder::from_varmap(&varmap, DType::F32, device);
    let mut model = GraphTransformerModel::new(model_config, vb)?;
    training::train(examples, graph, node_embeddings, &mut model, &varmap, model_config, train_config)?;
    let save_dir = save_path.as_ref().parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(save_dir)?;
    varmap.save(save_path.as_ref())?;
    Ok(())
}
