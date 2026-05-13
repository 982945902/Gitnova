use crate::model::{CodeGraph, EdgeKind, Node, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSummary {
    pub schema_version: u32,
    pub repo_root: String,
    pub nodes: usize,
    pub edges: usize,
    pub files: usize,
    pub languages: BTreeMap<String, usize>,
    pub hubs: Vec<NodeDigest>,
    pub recent_churn: Vec<NodeDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeDigest {
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub path: String,
    pub in_degree: usize,
    pub out_degree: usize,
    pub churn_90d: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExplainSymbol {
    pub symbol: Option<Node>,
    pub incoming: Vec<NodeDigest>,
    pub outgoing: Vec<NodeDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    pub symbol: Option<NodeDigest>,
    pub impacted: Vec<NodeDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureMap {
    pub focus: Option<String>,
    pub areas: Vec<ArchitectureArea>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureArea {
    pub name: String,
    pub files: usize,
    pub symbols: usize,
    pub top_symbols: Vec<NodeDigest>,
}

pub fn summarize(graph: &CodeGraph) -> GraphSummary {
    let mut languages = BTreeMap::new();
    for node in graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
    {
        if let Some(language) = node.language {
            *languages.entry(language.as_str().to_string()).or_default() += 1;
        }
    }
    let mut hubs: Vec<_> = graph.nodes.iter().map(digest).collect();
    hubs.sort_by_key(|node| std::cmp::Reverse(node.in_degree + node.out_degree));
    hubs.truncate(10);
    let mut recent_churn: Vec<_> = graph.nodes.iter().map(digest).collect();
    recent_churn.sort_by_key(|node| std::cmp::Reverse(node.churn_90d));
    recent_churn.truncate(10);
    GraphSummary {
        schema_version: graph.schema_version,
        repo_root: graph.repo_root.clone(),
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        files: graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::File)
            .count(),
        languages,
        hubs,
        recent_churn,
    }
}

pub fn explain_symbol(graph: &CodeGraph, symbol: &str) -> ExplainSymbol {
    let found = find_symbol(graph, symbol).cloned();
    let Some(node) = &found else {
        return ExplainSymbol {
            symbol: None,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        };
    };
    let by_id: HashMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let incoming = graph
        .edges
        .iter()
        .filter(|edge| edge.to == node.id)
        .filter_map(|edge| by_id.get(edge.from.as_str()).map(|node| digest(node)))
        .collect();
    let outgoing = graph
        .edges
        .iter()
        .filter(|edge| edge.from == node.id)
        .filter_map(|edge| by_id.get(edge.to.as_str()).map(|node| digest(node)))
        .collect();
    ExplainSymbol {
        symbol: found,
        incoming,
        outgoing,
    }
}

pub fn impact_analysis(graph: &CodeGraph, symbol: &str, limit: usize) -> ImpactAnalysis {
    let Some(target) = find_symbol(graph, symbol) else {
        return ImpactAnalysis {
            symbol: None,
            impacted: Vec::new(),
        };
    };
    let by_id: HashMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let mut impacted = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([target.id.clone()]);
    while let Some(current) = queue.pop_front() {
        for edge in graph.edges.iter().filter(|edge| {
            edge.to == current
                && matches!(
                    edge.kind,
                    EdgeKind::Calls | EdgeKind::References | EdgeKind::Imports | EdgeKind::Defines
                )
        }) {
            if seen.insert(edge.from.clone()) {
                if let Some(node) = by_id.get(edge.from.as_str()) {
                    if node.kind != NodeKind::Import {
                        impacted.push(digest(node));
                    }
                }
                queue.push_back(edge.from.clone());
            }
            if impacted.len() >= limit {
                break;
            }
        }
        if impacted.len() >= limit {
            break;
        }
    }
    ImpactAnalysis {
        symbol: Some(digest(target)),
        impacted,
    }
}

pub fn architecture_map(graph: &CodeGraph, focus: Option<&str>) -> ArchitectureMap {
    let focus_lower = focus.map(|value| value.to_ascii_lowercase());
    let mut areas: BTreeMap<String, ArchitectureArea> = BTreeMap::new();
    for file in graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
    {
        if let Some(focus) = &focus_lower {
            let haystack = format!("{} {}", file.path, file.text).to_ascii_lowercase();
            if !haystack.contains(focus) {
                continue;
            }
        }
        let area_name = file.path.split('/').next().unwrap_or("root").to_string();
        areas
            .entry(area_name.clone())
            .or_insert_with(|| ArchitectureArea {
                name: area_name,
                files: 0,
                symbols: 0,
                top_symbols: Vec::new(),
            })
            .files += 1;
    }
    for node in graph.nodes.iter().filter(|node| {
        !matches!(
            node.kind,
            NodeKind::Repository | NodeKind::File | NodeKind::Import
        )
    }) {
        let area_name = node.path.split('/').next().unwrap_or("root").to_string();
        if let Some(area) = areas.get_mut(&area_name) {
            area.symbols += 1;
            area.top_symbols.push(digest(node));
        }
    }
    for area in areas.values_mut() {
        area.top_symbols
            .sort_by_key(|node| std::cmp::Reverse(node.in_degree + node.out_degree));
        area.top_symbols.truncate(5);
    }
    ArchitectureMap {
        focus: focus.map(ToOwned::to_owned),
        areas: areas.into_values().collect(),
    }
}

pub fn find_symbol<'a>(graph: &'a CodeGraph, symbol: &str) -> Option<&'a Node> {
    let symbol_lower = symbol.to_ascii_lowercase();
    graph.nodes.iter().find(|node| {
        !matches!(
            node.kind,
            NodeKind::Repository | NodeKind::File | NodeKind::Import
        ) && (node.name.eq_ignore_ascii_case(symbol)
            || node.qualified_name.eq_ignore_ascii_case(symbol)
            || node
                .qualified_name
                .to_ascii_lowercase()
                .contains(&symbol_lower))
    })
}

pub fn digest(node: &Node) -> NodeDigest {
    NodeDigest {
        id: node.id.clone(),
        kind: node.kind.clone(),
        name: node.name.clone(),
        qualified_name: node.qualified_name.clone(),
        path: node.path.clone(),
        in_degree: node.metrics.in_degree,
        out_degree: node.metrics.out_degree,
        churn_90d: node.metrics.churn_90d,
    }
}
