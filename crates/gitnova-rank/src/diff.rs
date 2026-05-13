use gitnova_core::CodeGraph;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffContext {
    pub base: String,
    pub changed_paths: Vec<String>,
    pub ranked: crate::score::RankResponse,
}

pub fn changed_paths(repo: impl AsRef<Path>, base: &str) -> Vec<String> {
    let output = Command::new("git")
        .args([
            "-C",
            repo.as_ref().to_string_lossy().as_ref(),
            "diff",
            "--name-only",
            base,
        ])
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

pub fn diff_context(
    graph: &CodeGraph,
    repo: impl AsRef<Path>,
    base: &str,
    limit: usize,
) -> DiffContext {
    let changed_paths = changed_paths(repo, base);
    let changed: HashSet<_> = changed_paths.iter().collect();
    let query = if changed_paths.is_empty() {
        "recent changed code".to_string()
    } else {
        changed_paths.join(" ")
    };
    let mut ranked = crate::score::rank_graph(graph, &query, graph.nodes.len());
    for result in &mut ranked.results {
        if changed.contains(&result.node.path) {
            result.score += 0.5;
            result
                .explanation
                .strong_signals
                .push("changed in diff".into());
        }
    }
    ranked.results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.results.truncate(limit);
    DiffContext {
        base: base.to_string(),
        changed_paths,
        ranked,
    }
}
