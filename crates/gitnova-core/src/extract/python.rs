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
    let import_re =
        Regex::new(r"^\s*(?:from\s+([A-Za-z0-9_\.]+)\s+import\s+(.+)|import\s+(.+))").unwrap();
    let class_re = Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let def_re = Regex::new(r"^\s*def\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let mut extraction = FileExtraction::default();
    let mut current_class: Option<(String, usize)> = None;
    let mut symbols = Vec::new();

    for (index, line) in parsed.text.lines().enumerate() {
        if let Some(cap) = import_re.captures(line) {
            let names = cap
                .get(2)
                .or_else(|| cap.get(3))
                .map(|m| m.as_str())
                .unwrap_or("");
            for name in names.split(',') {
                let name = name.split_whitespace().next().unwrap_or("").to_string();
                if !name.is_empty() {
                    extraction
                        .imports
                        .push(import_from_line(path, index, line, name));
                }
            }
        }
        let indent = line.chars().take_while(|ch| ch.is_whitespace()).count();
        if let Some((_, class_indent)) = &current_class {
            if !line.trim().is_empty() && indent <= *class_indent {
                current_class = None;
            }
        }
        if let Some(cap) = class_re.captures(line) {
            let name = cap[1].to_string();
            push_symbol(
                &mut symbols,
                NodeKind::Class,
                path,
                index,
                line,
                &name,
                &name,
            );
            current_class = Some((name, indent));
        } else if let Some(cap) = def_re.captures(line) {
            let name = cap[1].to_string();
            let qualified = current_class
                .as_ref()
                .map(|(class_name, _)| format!("{class_name}::{name}"))
                .unwrap_or_else(|| name.clone());
            let kind = if current_class.is_some() {
                NodeKind::Method
            } else {
                NodeKind::Function
            };
            push_symbol(&mut symbols, kind, path, index, line, &name, &qualified);
        }
    }

    fill_symbol_text_and_calls(&mut symbols, &parsed.text);
    extraction.symbols = symbols;
    extraction
}

fn extract_syntax_symbols(parsed: &ParsedFile) -> Option<Vec<ExtractedSymbol>> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
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
    current_class: Option<String>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut class_for_children = current_class.clone();
    match node.kind() {
        "class_definition" => {
            if let Some(name) = child_name(node, source) {
                push_syntax_symbol(symbols, NodeKind::Class, path, node, &name, &name);
                class_for_children = Some(name);
            }
        }
        "function_definition" => {
            if let Some(name) = child_name(node, source) {
                let qualified = current_class
                    .as_ref()
                    .map(|class_name| format!("{class_name}::{name}"))
                    .unwrap_or_else(|| name.clone());
                let kind = if current_class.is_some() {
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
            walk_node(child, source, path, class_for_children.clone(), symbols);
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

#[cfg(test)]
mod tests {
    use crate::{build_graph, NodeKind};
    use std::fs;

    #[test]
    fn extracts_decorated_python_symbols_from_syntax_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.py"),
            r#"
def normalize_token():
    return True

class AuthService:
    @classmethod
    async def validate(cls, session):
        return normalize_token()
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();
        let validate = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name.ends_with("AuthService::validate"))
            .expect("decorated async Python method should be extracted by AST");
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
