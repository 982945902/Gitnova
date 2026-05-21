use crate::model::{CodeGraph, Edge, EdgeKind, Node, NodeKind};
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
    pub schema_version: u32,
    pub selector: String,
    pub symbol: Option<Node>,
    pub summary: Option<String>,
    pub incoming: Vec<NodeDigest>,
    pub outgoing: Vec<NodeDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    pub schema_version: u32,
    pub selector: String,
    pub symbol: Option<NodeDigest>,
    pub impacted: Vec<NodeDigest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphContext {
    pub schema_version: u32,
    pub selector: String,
    pub target: Option<Node>,
    pub summary: Option<String>,
    pub depth: usize,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub incoming: Vec<NodeDigest>,
    pub outgoing: Vec<NodeDigest>,
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
    let generic_names: &[&str] = &[
        "size", "c_str", "begin", "end", "empty", "Init",
        "clear", "get", "Get", "set", "push_back", "pop_back",
        "length", "data", "reset", "find", "insert",
        "toString", "to_string", "init", "destroy", "IsOK",
    ];
    hubs.retain(|node| {
        !(generic_names.contains(&node.name.as_str()) && node.in_degree > 100)
    });
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
    explain_node_from_found(graph, symbol, found)
}

pub fn explain_node(graph: &CodeGraph, selector: &str) -> ExplainSymbol {
    let found = find_node(graph, selector).cloned();
    explain_node_from_found(graph, selector, found)
}

fn explain_node_from_found(
    graph: &CodeGraph,
    selector: &str,
    found: Option<Node>,
) -> ExplainSymbol {
    let Some(node) = &found else {
        return ExplainSymbol {
            schema_version: graph.schema_version,
            selector: selector.to_string(),
            symbol: None,
            summary: None,
            incoming: Vec::new(),
            outgoing: Vec::new(),
        };
    };
    let by_id: HashMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let incoming: Vec<NodeDigest> = graph
        .edges
        .iter()
        .filter(|edge| edge.to == node.id)
        .filter_map(|edge| by_id.get(edge.from.as_str()).map(|node| digest(node)))
        .collect();
    let outgoing: Vec<NodeDigest> = graph
        .edges
        .iter()
        .filter(|edge| edge.from == node.id)
        .filter_map(|edge| by_id.get(edge.to.as_str()).map(|node| digest(node)))
        .collect();
    let incoming = unique_digests(incoming);
    let outgoing = unique_digests(outgoing);
    let summary = Some(node_summary(node, &incoming, &outgoing));
    ExplainSymbol {
        schema_version: graph.schema_version,
        selector: selector.to_string(),
        symbol: found,
        summary,
        incoming,
        outgoing,
    }
}

pub fn impact_analysis(graph: &CodeGraph, symbol: &str, limit: usize) -> ImpactAnalysis {
    let Some(target) = find_node(graph, symbol) else {
        return ImpactAnalysis {
            schema_version: graph.schema_version,
            selector: symbol.to_string(),
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
                    EdgeKind::Calls | EdgeKind::References | EdgeKind::Imports | EdgeKind::Defines | EdgeKind::Extends
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
        schema_version: graph.schema_version,
        selector: symbol.to_string(),
        symbol: Some(digest(target)),
        impacted,
    }
}

pub fn graph_context(
    graph: &CodeGraph,
    selector: &str,
    depth: usize,
    limit: usize,
) -> GraphContext {
    let Some(target) = find_node(graph, selector) else {
        return GraphContext {
            schema_version: graph.schema_version,
            selector: selector.to_string(),
            target: None,
            summary: None,
            depth,
            nodes: Vec::new(),
            edges: Vec::new(),
            incoming: Vec::new(),
            outgoing: Vec::new(),
        };
    };
    let by_id: HashMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect();
    let max_depth = depth.min(3);
    let max_nodes = limit.max(1);
    let mut selected = HashSet::from([target.id.clone()]);
    let mut queue = VecDeque::from([(target.id.clone(), 0usize)]);
    while let Some((current, current_depth)) = queue.pop_front() {
        if current_depth >= max_depth || selected.len() >= max_nodes {
            continue;
        }
        for edge in graph
            .edges
            .iter()
            .filter(|edge| edge.from == current || edge.to == current)
        {
            let next = if edge.from == current {
                edge.to.clone()
            } else {
                edge.from.clone()
            };
            if selected.insert(next.clone()) {
                queue.push_back((next, current_depth + 1));
                if selected.len() >= max_nodes {
                    break;
                }
            }
        }
    }
    let mut nodes = selected
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .cloned()
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        let left_target = left.id == target.id;
        let right_target = right.id == target.id;
        right_target
            .cmp(&left_target)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.kind_label().cmp(right.kind_label()))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });
    let edges = graph
        .edges
        .iter()
        .filter(|edge| selected.contains(&edge.from) && selected.contains(&edge.to))
        .cloned()
        .collect::<Vec<_>>();
    let incoming = graph
        .edges
        .iter()
        .filter(|edge| edge.to == target.id)
        .filter_map(|edge| by_id.get(edge.from.as_str()).map(|node| digest(node)))
        .collect::<Vec<_>>();
    let outgoing = graph
        .edges
        .iter()
        .filter(|edge| edge.from == target.id)
        .filter_map(|edge| by_id.get(edge.to.as_str()).map(|node| digest(node)))
        .collect::<Vec<_>>();
    let incoming = unique_digests(incoming);
    let outgoing = unique_digests(outgoing);
    GraphContext {
        schema_version: graph.schema_version,
        selector: selector.to_string(),
        target: Some(target.clone()),
        summary: Some(node_summary(target, &incoming, &outgoing)),
        depth: max_depth,
        nodes,
        edges,
        incoming,
        outgoing,
    }
}

fn unique_digests(digests: Vec<NodeDigest>) -> Vec<NodeDigest> {
    let mut seen = HashSet::new();
    digests
        .into_iter()
        .filter(|digest| seen.insert(digest.id.clone()))
        .collect()
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
    let generic_method_names: &[&str] = &[
        "size", "c_str", "begin", "end", "empty", "Init",
        "clear", "get", "Get", "set", "push_back", "pop_back",
        "length", "data", "reset", "find", "insert",
        "toString", "to_string", "init", "destroy", "IsOK",
    ];
    for area in areas.values_mut() {
        area.top_symbols.retain(|node| {
            !(generic_method_names.contains(&node.name.as_str()) && node.in_degree > 100)
        });
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

pub fn find_node<'a>(graph: &'a CodeGraph, selector: &str) -> Option<&'a Node> {
    if selector.trim().is_empty() {
        return None;
    }
    let selector_lower = selector.to_ascii_lowercase();
    graph
        .nodes
        .iter()
        .find(|node| node.id == selector)
        .or_else(|| find_symbol(graph, selector))
        .or_else(|| {
            graph.nodes.iter().find(|node| {
                node.path.eq_ignore_ascii_case(selector)
                    || node.name.eq_ignore_ascii_case(selector)
                    || node.qualified_name.eq_ignore_ascii_case(selector)
            })
        })
        .or_else(|| {
            graph.nodes.iter().find(|node| {
                !matches!(node.kind, NodeKind::Import)
                    && node
                        .qualified_name
                        .to_ascii_lowercase()
                        .contains(&selector_lower)
            })
        })
        .or_else(|| {
            graph.nodes.iter().find(|node| {
                node.qualified_name
                    .to_ascii_lowercase()
                    .contains(&selector_lower)
            })
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

trait NodeKindLabel {
    fn kind_label(&self) -> &'static str;
}

impl NodeKindLabel for Node {
    fn kind_label(&self) -> &'static str {
        match self.kind {
            NodeKind::Repository => "repository",
            NodeKind::File => "file",
            NodeKind::Module => "module",
            NodeKind::Function => "function",
            NodeKind::Method => "method",
            NodeKind::Class => "class",
            NodeKind::Struct => "struct",
            NodeKind::Trait => "trait",
            NodeKind::Interface => "interface",
            NodeKind::Import => "import",
            NodeKind::Unknown => "unknown",
        }
    }
}

fn node_summary(node: &Node, incoming: &[NodeDigest], outgoing: &[NodeDigest]) -> String {
    let role = match node.kind {
        NodeKind::Function | NodeKind::Method => {
            format!("implements {} behavior", humanize_identifier(&node.name))
        }
        NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => {
            format!("models {}", humanize_identifier(&node.name))
        }
        NodeKind::File => "groups source code in this file".to_string(),
        NodeKind::Module => "groups module-level code".to_string(),
        NodeKind::Import => "represents an imported dependency".to_string(),
        NodeKind::Repository => "represents the indexed repository".to_string(),
        NodeKind::Unknown => "represents an indexed code item".to_string(),
    };
    let mut parts = vec![format!(
        "{} `{}` in `{}` {}.",
        node.kind_label(),
        node.qualified_name,
        node.path,
        role
    )];
    parts.push(format!(
        "It has {} incoming and {} outgoing graph relationships.",
        incoming.len(),
        outgoing.len()
    ));
    if !outgoing.is_empty() {
        parts.push(format!("It uses {}.", digest_list(outgoing)));
    }
    if !incoming.is_empty() {
        parts.push(format!("It is reached from {}.", digest_list(incoming)));
    }
    parts.join(" ")
}

fn humanize_identifier(name: &str) -> String {
    let mut output = String::new();
    let mut previous_was_lower_or_digit = false;
    for ch in name.replace(['_', '-'], " ").chars() {
        if ch.is_uppercase() && previous_was_lower_or_digit {
            output.push(' ');
        }
        output.push(ch.to_ascii_lowercase());
        previous_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn digest_list(digests: &[NodeDigest]) -> String {
    digests
        .iter()
        .take(3)
        .map(|digest| format!("`{}`", digest.name))
        .collect::<Vec<_>>()
        .join(", ")
}
