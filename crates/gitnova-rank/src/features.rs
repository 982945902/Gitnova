use gitnova_core::{CodeGraph, Node, NodeKind};
use regex::Regex;
use std::collections::HashSet;

pub fn tokenize(value: &str) -> Vec<String> {
    let camel = Regex::new(r"([a-z0-9])([A-Z])").unwrap();
    let normalized = camel
        .replace_all(value, "$1 $2")
        .replace(['_', '-', '/', ':', '.'], " ");
    normalized
        .split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|token| {
            let token = normalize_token(token);
            (token.len() > 1).then_some(token)
        })
        .collect()
}

pub fn normalize_token(token: &str) -> String {
    let mut value = token.to_ascii_lowercase();
    if matches!(value.as_str(), "validation" | "validated" | "validating") {
        return "validate".into();
    }
    for suffix in ["ing", "ed", "ion", "ions", "s"] {
        if value.len() > suffix.len() + 3 && value.ends_with(suffix) {
            value.truncate(value.len() - suffix.len());
            break;
        }
    }
    if value == "validation" || value == "validat" {
        "validate".into()
    } else {
        value
    }
}

pub fn query_overlap(query_tokens: &[String], node: &Node) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let node_tokens: HashSet<_> = tokenize(&format!(
        "{} {} {} {}",
        node.name, node.qualified_name, node.path, node.text
    ))
    .into_iter()
    .collect();
    let hits = query_tokens
        .iter()
        .filter(|token| node_tokens.contains(*token))
        .count();
    hits as f64 / query_tokens.len() as f64
}

pub fn lexical_match(query: &str, query_tokens: &[String], node: &Node) -> f64 {
    let haystack =
        format!("{} {} {}", node.name, node.qualified_name, node.path).to_ascii_lowercase();
    let query_lower = query.to_ascii_lowercase();
    if haystack.contains(&query_lower) {
        return 1.0;
    }
    let node_name_lower = node.name.to_ascii_lowercase();
    if query_lower.contains(&node_name_lower) {
        let name_tokens = tokenize(&node.name);
        let name_bonus: f64 = if name_tokens.len() > 1 || node_name_lower.len() >= 10 {
            1.0
        } else {
            0.70
        };
        return name_bonus.max(query_overlap(query_tokens, node));
    }
    query_overlap(query_tokens, node)
}

pub fn relation_importance(node: &Node) -> f64 {
    ((node.metrics.in_degree + node.metrics.out_degree) as f64 / 12.0).min(1.0)
}

pub fn domain_specificity(query_tokens: &[String], node: &Node) -> f64 {
    let path_tokens: HashSet<_> = tokenize(&node.path).into_iter().collect();
    let domain_hits = query_tokens
        .iter()
        .filter(|token| path_tokens.contains(*token))
        .count();
    let kind_bonus = match node.kind {
        NodeKind::Function
        | NodeKind::Method
        | NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Enum
        | NodeKind::Union
        | NodeKind::Typedef
        | NodeKind::Variable
        | NodeKind::Macro
        | NodeKind::Trait
        | NodeKind::Interface => 0.2,
        _ => 0.0,
    };
    (domain_hits as f64 / query_tokens.len().max(1) as f64 + kind_bonus).min(1.0)
}

pub fn churn(node: &Node) -> f64 {
    (node.metrics.churn_90d as f64 / 10.0).min(1.0)
}

pub fn hub_penalty(node: &Node) -> f64 {
    let degree = node.metrics.in_degree + node.metrics.out_degree;
    let base = if degree <= 4 {
        0.0
    } else {
        ((degree - 4) as f64 / 12.0).min(1.0)
    };
    // Macro and variable nodes are globally referenced, apply extra penalty
    if matches!(node.kind, NodeKind::Macro | NodeKind::Variable) {
        (base + 0.3).min(1.0)
    } else {
        base
    }
}

pub fn utility_penalty(node: &Node, overlap: f64) -> f64 {
    let haystack = format!("{} {}", node.name, node.path).to_ascii_lowercase();
    let generic_path = [
        "util",
        "utils",
        "logger",
        "types",
        "constants",
        "config",
        "format",
    ]
    .iter()
    .any(|needle| haystack.contains(needle));

    let generic_method_names = [
        "size",
        "c_str",
        "begin",
        "end",
        "empty",
        "Init",
        "clear",
        "get",
        "Get",
        "set",
        "push_back",
        "pop_back",
        "length",
        "data",
        "reset",
        "find",
        "insert",
        "toString",
        "to_string",
        "init",
        "destroy",
        "IsOK",
    ];
    let is_generic_method =
        generic_method_names.contains(&node.name.as_str()) && node.metrics.in_degree > 20;

    // Third-party library penalty: if the path contains third_party/ or thirdparty/,
    // and query overlap is low, penalize heavily.
    let is_third_party = node.path.to_ascii_lowercase().contains("third_party")
        || node.path.to_ascii_lowercase().contains("thirdparty");
    if is_third_party && overlap < 0.50 {
        return 0.5;
    }

    if !generic_path && !is_generic_method {
        return 0.0;
    }

    let penalty: f64 = if is_generic_method && !generic_path {
        0.5
    } else if is_generic_method && generic_path {
        1.0
    } else {
        1.0
    };

    if overlap >= 0.50 {
        (penalty * 0.25).min(penalty)
    } else {
        penalty
    }
}

pub fn graph_proximity(_graph: &CodeGraph, node: &Node) -> f64 {
    if node.metrics.in_degree > 0 || node.metrics.out_degree > 0 {
        0.5
    } else {
        0.0
    }
}
