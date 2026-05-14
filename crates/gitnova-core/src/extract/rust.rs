use super::{fill_symbol_text_and_calls, import_from_line, ExtractedSymbol, FileExtraction};
use crate::model::{NodeKind, Span};
use crate::parser::{line_span, ParsedFile};
use regex::Regex;
use std::collections::HashSet;
use tree_sitter::{Node, Parser};

pub fn extract(parsed: &ParsedFile) -> FileExtraction {
    let mut heuristic = extract_heuristic(parsed);
    let Some(mut syntax_symbols) = extract_syntax_symbols(parsed) else {
        return heuristic;
    };

    syntax_symbols.sort_by_key(|symbol| (symbol.span.start_line, symbol.span.start_col));
    fill_symbol_text_and_calls(&mut syntax_symbols, &parsed.text);
    heuristic.symbols = merge_symbols(syntax_symbols, heuristic.symbols);
    heuristic
}

fn extract_heuristic(parsed: &ParsedFile) -> FileExtraction {
    let path = &parsed.source.relative_path;
    let mut extraction = FileExtraction::default();
    let struct_re = Regex::new(r"\bstruct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let trait_re = Regex::new(r"\btrait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let fn_re = Regex::new(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let impl_re =
        Regex::new(r"\bimpl(?:\s+[A-Za-z_][A-Za-z0-9_<>,\s]*\s+for)?\s+([A-Za-z_][A-Za-z0-9_]*)")
            .unwrap();
    let use_re = Regex::new(r"^\s*use\s+(.+?);").unwrap();
    let mut current_impl: Option<String> = None;
    let mut impl_depth = 0isize;
    let mut symbols = Vec::new();

    for (index, line) in parsed.text.lines().enumerate() {
        if let Some(cap) = use_re.captures(line) {
            let raw = cap[1].trim();
            let name = raw
                .split("::")
                .last()
                .unwrap_or(raw)
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_')
                .to_string();
            extraction
                .imports
                .push(import_from_line(path, index, line, name));
        }
        if let Some(cap) = impl_re.captures(line) {
            current_impl = Some(cap[1].to_string());
            impl_depth = brace_delta(line).max(1);
        }
        if let Some(cap) = struct_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Struct,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
        }
        if let Some(cap) = trait_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Trait,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
        }
        if let Some(cap) = fn_re.captures(line) {
            let name = cap[1].to_string();
            let qualified = current_impl
                .as_ref()
                .map(|parent| format!("{parent}::{name}"))
                .unwrap_or_else(|| name.clone());
            let kind = if current_impl.is_some() {
                NodeKind::Method
            } else {
                NodeKind::Function
            };
            push_symbol(&mut symbols, kind, path, index, line, &name, &qualified);
        }
        if current_impl.is_some() {
            impl_depth += brace_delta(line);
            if impl_depth <= 0 {
                current_impl = None;
                impl_depth = 0;
            }
        }
    }

    fill_symbol_text_and_calls(&mut symbols, &parsed.text);
    extraction.symbols = symbols;
    extraction
}

fn extract_syntax_symbols(parsed: &ParsedFile) -> Option<Vec<ExtractedSymbol>> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE.into();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&parsed.text, None)?;
    let mut symbols = Vec::new();
    walk_node(
        tree.root_node(),
        parsed.text.as_bytes(),
        &parsed.source.relative_path,
        None,
        &mut symbols,
    );
    if symbols.is_empty() {
        None
    } else {
        Some(symbols)
    }
}

fn walk_node(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    current_impl: Option<String>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut impl_for_children = current_impl.clone();
    match node.kind() {
        "struct_item" => {
            if let Some(name) = child_name(node, source) {
                push_syntax_symbol(symbols, NodeKind::Struct, path, node, &name, &name);
            }
        }
        "trait_item" => {
            if let Some(name) = child_name(node, source) {
                push_syntax_symbol(symbols, NodeKind::Trait, path, node, &name, &name);
                impl_for_children = Some(name);
            }
        }
        "impl_item" => {
            impl_for_children = impl_target(node, source);
        }
        "function_item" => {
            if let Some(name) = child_name(node, source) {
                let qualified = current_impl
                    .as_ref()
                    .map(|parent| format!("{parent}::{name}"))
                    .unwrap_or_else(|| name.clone());
                let kind = if current_impl.is_some() {
                    NodeKind::Method
                } else {
                    NodeKind::Function
                };
                push_syntax_symbol(symbols, kind, path, node, &name, &qualified);
            }
        }
        _ => {}
    }

    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index) {
            walk_node(child, source, path, impl_for_children.clone(), symbols);
        }
    }
}

fn child_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|child| child.utf8_text(source).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn impl_target(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(target) = node.child_by_field_name("type") {
        return node_text(target, source).map(clean_type_name);
    }
    for index in 0..node.named_child_count() {
        let child = node.named_child(index)?;
        if matches!(
            child.kind(),
            "type_identifier"
                | "generic_type"
                | "scoped_type_identifier"
                | "reference_type"
                | "tuple_type"
        ) {
            return node_text(child, source).map(clean_type_name);
        }
    }
    None
}

fn node_text<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    node.utf8_text(source)
        .ok()
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

fn clean_type_name(value: &str) -> String {
    value
        .split('<')
        .next()
        .unwrap_or(value)
        .trim()
        .trim_start_matches('&')
        .trim()
        .to_string()
}

fn push_syntax_symbol(
    symbols: &mut Vec<ExtractedSymbol>,
    kind: NodeKind,
    path: &str,
    node: Node<'_>,
    name: &str,
    qualified_name: &str,
) {
    symbols.push(ExtractedSymbol {
        kind,
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        path: path.to_string(),
        span: span_from_node(node),
        text: String::new(),
        calls: Vec::new(),
        tags: vec!["tree-sitter".into()],
    });
}

fn span_from_node(node: Node<'_>) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span {
        start_line: start.row + 1,
        start_col: start.column + 1,
        end_line: end.row + 1,
        end_col: end.column + 1,
    }
}

fn merge_symbols(
    syntax_symbols: Vec<ExtractedSymbol>,
    heuristic_symbols: Vec<ExtractedSymbol>,
) -> Vec<ExtractedSymbol> {
    let mut seen = HashSet::new();
    let mut merged = Vec::new();
    for symbol in syntax_symbols.into_iter().chain(heuristic_symbols) {
        let key = (symbol.kind.clone(), symbol.qualified_name.clone());
        if seen.insert(key) {
            merged.push(symbol);
        }
    }
    merged.sort_by_key(|symbol| (symbol.span.start_line, symbol.span.start_col));
    merged
}

fn push_symbol(
    symbols: &mut Vec<ExtractedSymbol>,
    kind: NodeKind,
    path: &str,
    index: usize,
    line: &str,
    name: &str,
    qualified_name: &str,
) {
    symbols.push(ExtractedSymbol {
        kind,
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        path: path.to_string(),
        span: line_span(index, line),
        text: String::new(),
        calls: Vec::new(),
        tags: Vec::new(),
    });
}

fn brace_delta(line: &str) -> isize {
    line.chars().filter(|ch| *ch == '{').count() as isize
        - line.chars().filter(|ch| *ch == '}').count() as isize
}

#[cfg(test)]
mod tests {
    use crate::{build_graph, NodeKind};
    use std::fs;

    #[test]
    fn extracts_multiline_rust_items_from_syntax_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("lib.rs"),
            r#"
pub struct AuthService;

impl AuthService {
    pub fn
    validate(&self) -> bool {
        normalize_token();
        true
    }
}

pub fn normalize_token() {}
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();
        let validate = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name.ends_with("AuthService::validate"))
            .expect("multiline Rust method should be extracted by AST");
        assert_eq!(validate.kind, NodeKind::Method);
        assert!(validate.tags.contains(&"tree-sitter".to_string()));
        assert!(graph.edges.iter().any(|edge| {
            edge.from == validate.id
                && graph
                    .node(&edge.to)
                    .map(|node| node.name == "normalize_token")
                    .unwrap_or(false)
        }));
    }
}
