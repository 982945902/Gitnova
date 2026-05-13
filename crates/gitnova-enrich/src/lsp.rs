use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspStatus {
    pub tool: String,
    pub available: bool,
}

pub fn detect_language_servers() -> Vec<LspStatus> {
    [
        "rust-analyzer",
        "typescript-language-server",
        "pyright-langserver",
    ]
    .into_iter()
    .map(|tool| LspStatus {
        tool: tool.to_string(),
        available: Command::new(tool).arg("--version").output().is_ok(),
    })
    .collect()
}

pub fn enrich_best_effort() -> Vec<LspStatus> {
    detect_language_servers()
}

pub fn apply_lsp_metadata(graph: &mut gitnova_core::CodeGraph) -> Vec<LspStatus> {
    let statuses = detect_language_servers();
    if let Some(repo) = graph
        .nodes
        .iter_mut()
        .find(|node| node.kind == gitnova_core::NodeKind::Repository)
    {
        for status in &statuses {
            let state = if status.available {
                "available"
            } else {
                "missing"
            };
            repo.tags.push(format!("lsp:{}:{state}", status.tool));
        }
    }
    statuses
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_lsp_tools_are_status_not_errors() {
        let statuses = detect_language_servers();
        assert_eq!(statuses.len(), 3);
    }

    #[test]
    fn lsp_metadata_is_attached_without_requiring_tools() {
        let mut graph = gitnova_core::CodeGraph::empty("repo".into());
        graph.nodes.push(gitnova_core::Node {
            id: "repo".into(),
            kind: gitnova_core::NodeKind::Repository,
            name: "repo".into(),
            qualified_name: "repo".into(),
            path: String::new(),
            span: None,
            language: None,
            text: String::new(),
            tags: Vec::new(),
            metrics: gitnova_core::NodeMetrics::default(),
        });
        let statuses = apply_lsp_metadata(&mut graph);
        assert_eq!(statuses.len(), 3);
        assert!(graph.nodes[0]
            .tags
            .iter()
            .any(|tag| tag.starts_with("lsp:")));
    }
}
