use anyhow::Result;
use gitnova_core::CodeGraph;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn export_graph(graph: &CodeGraph, path: impl AsRef<Path>) -> Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(graph)?)?;
    Ok(())
}

pub fn import_graph(path: impl AsRef<Path>) -> Result<CodeGraph> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn export_embeddings(
    embeddings: &HashMap<String, Vec<f32>>,
    path: impl AsRef<Path>,
) -> Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }
    // Convert Vec<f32> to strings for compact JSON
    let encoded: HashMap<String, String> = embeddings
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                v.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })
        .collect();
    fs::write(path, serde_json::to_vec_pretty(&encoded)?)?;
    Ok(())
}

pub fn import_embeddings(path: impl AsRef<Path>) -> Result<HashMap<String, Vec<f32>>> {
    let bytes = fs::read(path)?;
    let encoded: HashMap<String, String> = serde_json::from_slice(&bytes)?;
    Ok(encoded
        .into_iter()
        .map(|(k, v)| {
            (
                k,
                v.split(',').filter_map(|s| s.parse::<f32>().ok()).collect(),
            )
        })
        .collect())
}
