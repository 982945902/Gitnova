use super::{fill_symbol_text_and_calls, import_from_line, ExtractedSymbol, FileExtraction};
use crate::model::{Language, NodeKind, Span};
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
    let import_re = Regex::new(r#"^\s*import\s+\{?([^}"']+)\}?\s+from"#).unwrap();
    let fn_re =
        Regex::new(r"\b(?:export\s+)?(?:async\s+)?function\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let arrow_re =
        Regex::new(r"\b(?:export\s+)?(?:const|let)\s+([A-Za-z_][A-Za-z0-9_]*)\s*=").unwrap();
    let class_re = Regex::new(r"\b(?:export\s+)?class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let interface_re = Regex::new(r"\b(?:export\s+)?interface\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let method_re = Regex::new(
        r"^\s*(?:public\s+|private\s+|protected\s+)?(?:async\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*\(",
    )
    .unwrap();
    let mut current_class: Option<String> = None;
    let mut class_depth = 0isize;
    let mut symbols = Vec::new();

    for (index, line) in parsed.text.lines().enumerate() {
        if let Some(cap) = import_re.captures(line) {
            for name in cap[1].split(',') {
                let name = name.split_whitespace().next().unwrap_or("").to_string();
                if !name.is_empty() {
                    extraction
                        .imports
                        .push(import_from_line(path, index, line, name));
                }
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
            current_class = Some(name);
            class_depth = brace_delta(line).max(1);
            continue;
        }
        if let Some(cap) = interface_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Interface,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
            continue;
        }
        if let Some(cap) = fn_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Function,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
        } else if let Some(cap) = arrow_re.captures(line) {
            push_symbol(
                &mut symbols,
                NodeKind::Function,
                path,
                index,
                line,
                &cap[1],
                &cap[1],
            );
        } else if let (Some(parent), Some(cap)) = (&current_class, method_re.captures(line)) {
            let name = cap[1].to_string();
            let qualified = format!("{parent}::{name}");
            push_symbol(
                &mut symbols,
                NodeKind::Method,
                path,
                index,
                line,
                &name,
                &qualified,
            );
        }
        if current_class.is_some() {
            class_depth += brace_delta(line);
            if class_depth <= 0 {
                current_class = None;
                class_depth = 0;
            }
        }
    }

    fill_symbol_text_and_calls(&mut symbols, &parsed.text);
    extraction.symbols = symbols;
    extraction
}

fn extract_syntax_symbols(parsed: &ParsedFile) -> Option<Vec<ExtractedSymbol>> {
    let mut parser = Parser::new();
    let language = match parsed.source.language {
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => return None,
    };
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
        "function_declaration" | "generator_function_declaration" => {
            if let Some(name) = child_name(node, source) {
                push_syntax_symbol(symbols, NodeKind::Function, path, node, &name, &name);
            }
        }
        "class_declaration" => {
            if let Some(name) = child_name(node, source) {
                push_syntax_symbol(symbols, NodeKind::Class, path, node, &name, &name);
                class_for_children = Some(name);
            }
        }
        "interface_declaration" => {
            if let Some(name) = child_name(node, source) {
                push_syntax_symbol(symbols, NodeKind::Interface, path, node, &name, &name);
            }
        }
        "method_definition" | "method_signature" => {
            if let (Some(parent), Some(name)) = (current_class.as_deref(), child_name(node, source))
            {
                let qualified = format!("{parent}::{name}");
                push_syntax_symbol(symbols, NodeKind::Method, path, node, &name, &qualified);
            }
        }
        "variable_declarator" => {
            let value_kind = node
                .child_by_field_name("value")
                .map(|value| value.kind().to_string())
                .unwrap_or_default();
            if matches!(
                value_kind.as_str(),
                "arrow_function" | "function_expression" | "generator_function"
            ) {
                if let Some(name) = child_name(node, source) {
                    push_syntax_symbol(symbols, NodeKind::Function, path, node, &name, &name);
                }
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
        base_classes: Vec::new(),
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
        let key = (
            symbol.kind.clone(),
            symbol.qualified_name.clone(),
            symbol.span.start_line,
        );
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
        base_classes: Vec::new(),
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
    fn extracts_multiline_typescript_symbols_from_syntax_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.ts"),
            r#"
type Session = { token: string };

function checkToken(token: string): boolean {
  return token.length > 0;
}

export async function
validateSession(session: Session): Promise<boolean> {
  return checkToken(session.token);
}
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();
        let validate = graph
            .nodes
            .iter()
            .find(|node| node.name == "validateSession")
            .expect("multiline function declaration should be extracted");

        assert_eq!(validate.kind, NodeKind::Function);
        assert_eq!(validate.span.unwrap().start_line, 8);
        assert!(
            graph.edges.iter().any(|edge| {
                edge.from == validate.id
                    && graph
                        .node(&edge.to)
                        .map(|node| node.name == "checkToken")
                        .unwrap_or(false)
            }),
            "call edge from validateSession to checkToken should come from AST-backed text span"
        );
    }
}
