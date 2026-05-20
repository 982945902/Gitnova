use crate::error::Result;
use crate::extract::{extract_file, ExtractedSymbol};
use crate::language::is_test_path;
use crate::model::{
    current_unix, CodeGraph, Edge, EdgeKind, Node, NodeId, NodeKind, NodeMetrics, Span,
};
use crate::parser::parse_source_file;
use crate::scan::{scan_repository, SourceFile};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

pub fn build_graph(root: impl AsRef<Path>) -> Result<CodeGraph> {
    let files = scan_repository(root.as_ref())?;
    build_graph_from_entries(root, &files)
}

pub fn build_graph_from_entries(root: impl AsRef<Path>, files: &[SourceFile]) -> Result<CodeGraph> {
    let root = root.as_ref();
    let repo_root = root.to_string_lossy().to_string();
    let mut graph = CodeGraph::empty(repo_root.clone());
    graph.indexed_at_unix = current_unix();

    let repo_id = stable_id("repo", "", &repo_root, None);
    graph.nodes.push(Node {
        id: repo_id.clone(),
        kind: NodeKind::Repository,
        name: root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "repository".to_string()),
        qualified_name: repo_root,
        path: String::new(),
        span: None,
        language: None,
        text: String::new(),
        tags: vec!["repository".into()],
        metrics: NodeMetrics::default(),
    });

    let mut symbol_by_name: HashMap<String, Vec<NodeId>> = HashMap::new();
    let mut calls: Vec<(NodeId, Vec<String>)> = Vec::new();
    let mut pending_imports: Vec<(NodeId, String)> = Vec::new();
    let mut edge_set = HashSet::new();

    for file in files {
        let file_text = fs::read_to_string(&file.absolute_path).unwrap_or_default();
        let file_id = stable_id("file", &file.relative_path, &file.relative_path, None);
        graph.nodes.push(Node {
            id: file_id.clone(),
            kind: NodeKind::File,
            name: file
                .absolute_path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| file.relative_path.clone()),
            qualified_name: file.relative_path.clone(),
            path: file.relative_path.clone(),
            span: None,
            language: Some(file.language),
            text: file_text.clone(),
            tags: vec![file.language.as_str().to_string()],
            metrics: NodeMetrics {
                is_test: is_test_path(&file.relative_path),
                ..NodeMetrics::default()
            },
        });
        add_edge(
            &mut graph.edges,
            &mut edge_set,
            &repo_id,
            &file_id,
            EdgeKind::Contains,
            10_000,
        );

        let module_name = module_name_for(&file.relative_path);
        let module_id = stable_id("module", &file.relative_path, &module_name, None);
        graph.nodes.push(Node {
            id: module_id.clone(),
            kind: NodeKind::Module,
            name: module_name
                .split("::")
                .last()
                .unwrap_or(&module_name)
                .to_string(),
            qualified_name: module_name,
            path: file.relative_path.clone(),
            span: None,
            language: Some(file.language),
            text: String::new(),
            tags: vec!["module".into(), file.language.as_str().to_string()],
            metrics: NodeMetrics {
                is_test: is_test_path(&file.relative_path),
                ..NodeMetrics::default()
            },
        });
        add_edge(
            &mut graph.edges,
            &mut edge_set,
            &file_id,
            &module_id,
            EdgeKind::Defines,
            9_000,
        );
        add_edge(
            &mut graph.edges,
            &mut edge_set,
            &file_id,
            &module_id,
            EdgeKind::Contains,
            9_000,
        );

        let parsed = parse_source_file(file)?;
        let extraction = extract_file(&parsed);
        for import in extraction.imports {
            let import_id = stable_id(
                "import",
                &import.path,
                &import.qualified_name,
                Some(import.span),
            );
            graph.nodes.push(Node {
                id: import_id.clone(),
                kind: NodeKind::Import,
                name: import.name.clone(),
                qualified_name: import.qualified_name,
                path: import.path,
                span: Some(import.span),
                language: Some(file.language),
                text: import.text,
                tags: vec!["import".into()],
                metrics: NodeMetrics {
                    is_test: is_test_path(&file.relative_path),
                    ..NodeMetrics::default()
                },
            });
            add_edge(
                &mut graph.edges,
                &mut edge_set,
                &file_id,
                &import_id,
                EdgeKind::Imports,
                9_000,
            );
            pending_imports.push((file_id.clone(), import.name));
        }

        for symbol in extraction.symbols {
            let node_id = push_symbol_node(&mut graph, file, &symbol);
            add_edge(
                &mut graph.edges,
                &mut edge_set,
                &file_id,
                &node_id,
                EdgeKind::Defines,
                10_000,
            );
            add_edge(
                &mut graph.edges,
                &mut edge_set,
                &file_id,
                &node_id,
                EdgeKind::Contains,
                10_000,
            );
            symbol_by_name
                .entry(symbol.name.clone())
                .or_default()
                .push(node_id.clone());
            if let Some(last) = symbol.qualified_name.split("::").last() {
                symbol_by_name
                    .entry(last.to_string())
                    .or_default()
                    .push(node_id.clone());
            }
            for base in &symbol.base_classes {
                if let Some(targets) = symbol_by_name.get(base) {
                    for target in targets.iter().take(2) {
                        add_edge(
                            &mut graph.edges,
                            &mut edge_set,
                            &node_id,
                            target,
                            EdgeKind::Extends,
                            9_000,
                        );
                    }
                }
            }
            calls.push((node_id, symbol.calls));
        }
    }

    for (from, names) in calls {
        for name in names {
            if let Some(targets) = symbol_by_name.get(&name) {
                for target in targets.iter().take(2) {
                    if *target != from {
                        add_edge(
                            &mut graph.edges,
                            &mut edge_set,
                            &from,
                            target,
                            EdgeKind::Calls,
                            6_500,
                        );
                        add_edge(
                            &mut graph.edges,
                            &mut edge_set,
                            &from,
                            target,
                            EdgeKind::References,
                            5_500,
                        );
                    }
                }
            }
        }
    }

    for (file_id, import_name) in pending_imports {
        if let Some(targets) = symbol_by_name.get(&import_name) {
            for target in targets.iter().take(2) {
                add_edge(
                    &mut graph.edges,
                    &mut edge_set,
                    &file_id,
                    target,
                    EdgeKind::Imports,
                    7_000,
                );
            }
        }
    }

    recompute_degrees(&mut graph);
    Ok(graph)
}

fn module_name_for(path: &str) -> String {
    path.rsplit_once('.')
        .map(|(without_ext, _)| without_ext)
        .unwrap_or(path)
        .replace('/', "::")
}

fn push_symbol_node(graph: &mut CodeGraph, file: &SourceFile, symbol: &ExtractedSymbol) -> NodeId {
    let id = stable_id(
        "symbol",
        &symbol.path,
        &symbol.qualified_name,
        Some(symbol.span),
    );
    graph.nodes.push(Node {
        id: id.clone(),
        kind: symbol.kind.clone(),
        name: symbol.name.clone(),
        qualified_name: if symbol.qualified_name.contains("::") {
            symbol.qualified_name.clone()
        } else {
            format!("{}::{}", symbol.path, symbol.qualified_name)
        },
        path: symbol.path.clone(),
        span: Some(symbol.span),
        language: Some(file.language),
        text: symbol.text.clone(),
        tags: symbol.tags.clone(),
        metrics: NodeMetrics {
            is_test: is_test_path(&symbol.path),
            ..NodeMetrics::default()
        },
    });
    id
}

pub fn add_edge(
    edges: &mut Vec<Edge>,
    seen: &mut HashSet<(String, String, EdgeKind)>,
    from: &str,
    to: &str,
    kind: EdgeKind,
    confidence_basis_points: u16,
) {
    if from == to {
        return;
    }
    let key = (from.to_string(), to.to_string(), kind.clone());
    if seen.insert(key) {
        edges.push(Edge {
            from: from.to_string(),
            to: to.to_string(),
            kind,
            confidence_basis_points,
        });
    }
}

pub fn stable_id(kind: &str, path: &str, name: &str, span: Option<Span>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(path.as_bytes());
    hasher.update(b"\0");
    hasher.update(name.as_bytes());
    if let Some(span) = span {
        hasher.update(format!(
            "\0{}:{}:{}:{}",
            span.start_line, span.start_col, span.end_line, span.end_col
        ));
    }
    format!("gn_{:x}", hasher.finalize())[..19].to_string()
}

pub fn recompute_degrees(graph: &mut CodeGraph) {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut out_degree: HashMap<&str, usize> = HashMap::new();
    for edge in &graph.edges {
        *out_degree.entry(&edge.from).or_default() += 1;
        *in_degree.entry(&edge.to).or_default() += 1;
    }
    for node in &mut graph.nodes {
        node.metrics.in_degree = *in_degree.get(node.id.as_str()).unwrap_or(&0);
        node.metrics.out_degree = *out_degree.get(node.id.as_str()).unwrap_or(&0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn ids_are_deterministic_for_fixture_symbols() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.rs"),
            "pub struct AuthService;\nimpl AuthService { pub fn validate(&self) -> bool { true } }\n",
        )
        .unwrap();
        let first = build_graph(temp.path()).unwrap();
        let second = build_graph(temp.path()).unwrap();
        let first_validate = first
            .nodes
            .iter()
            .find(|node| node.qualified_name.contains("validate"))
            .unwrap();
        let second_validate = second
            .nodes
            .iter()
            .find(|node| node.qualified_name.contains("validate"))
            .unwrap();
        assert_eq!(first_validate.id, second_validate.id);
    }

    #[test]
    fn graph_has_expected_edge_kinds() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("utils.rs"),
            "pub fn normalize_email(v: &str) -> String { v.into() }\n",
        )
        .unwrap();
        fs::write(
            temp.path().join("auth.rs"),
            "use crate::utils::normalize_email;\npub fn validate(v: &str) -> String { normalize_email(v) }\n",
        )
        .unwrap();
        let graph = build_graph(temp.path()).unwrap();
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Contains));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Defines));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::Imports));
        assert!(graph.edges.iter().any(|edge| edge.kind == EdgeKind::Calls));
        assert!(graph
            .edges
            .iter()
            .any(|edge| edge.kind == EdgeKind::References));
        assert!(graph.nodes.iter().any(|node| node.kind == NodeKind::Module));
    }
}
