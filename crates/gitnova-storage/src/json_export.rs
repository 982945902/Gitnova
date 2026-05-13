use anyhow::Result;
use gitnova_core::CodeGraph;
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
