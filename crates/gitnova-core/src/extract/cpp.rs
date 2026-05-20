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
    let include_re =
        Regex::new(r#"^\s*#include\s+[<"]([^>"]+)[>"]"#).unwrap();
    let class_struct_re =
        Regex::new(r"^\s*(class|struct)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let namespace_re = Regex::new(r"^\s*namespace\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let mut extraction = FileExtraction::default();
    let mut symbols = Vec::new();
    let mut namespace_depth = 0isize;
    let mut namespaces: Vec<String> = Vec::new();

    for (index, line) in parsed.text.lines().enumerate() {
        if let Some(cap) = include_re.captures(line) {
            let name = cap[1]
                .rsplit('/')
                .next()
                .unwrap_or(cap.get(1).map(|m| m.as_str()).unwrap_or(""))
                .to_string();
            extraction
                .imports
                .push(import_from_line(path, index, line, name));
        }
        if let Some(cap) = namespace_re.captures(line) {
            let name = cap[1].to_string();
            let qualified = build_namespace_qualified(&namespaces, &name);
            push_symbol(
                &mut symbols,
                NodeKind::Module,
                path,
                index,
                line,
                &name,
                &qualified,
            );
            namespaces.push(name);
            if line.contains('{') {
                namespace_depth += 1;
            }
        }
        if let Some(cap) = class_struct_re.captures(line) {
            let keyword = cap[1].to_string();
            let name = cap[2].to_string();
            let kind = if keyword == "struct" {
                NodeKind::Struct
            } else {
                NodeKind::Class
            };
            let qualified = build_namespace_qualified(&namespaces, &name);
            push_symbol(&mut symbols, kind, path, index, line, &name, &qualified);
        }
        let opens = line.matches('{').count() as isize;
        let closes = line.matches('}').count() as isize;
        namespace_depth += opens - closes;
        if namespace_depth < namespaces.len() as isize {
            namespaces.truncate(namespace_depth.max(0) as usize);
        }
    }

    fill_symbol_text_and_calls(&mut symbols, &parsed.text);
    extraction.symbols = symbols;
    extraction
}

fn build_namespace_qualified(namespaces: &[String], name: &str) -> String {
    if namespaces.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", namespaces.join("::"), name)
    }
}

fn extract_syntax_symbols(parsed: &ParsedFile) -> Option<Vec<ExtractedSymbol>> {
    let mut parser = Parser::new();
    let language = tree_sitter_cpp::LANGUAGE.into();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&parsed.text, None)?;
    let mut symbols = Vec::new();
    walk_node(
        tree.root_node(),
        parsed.text.as_bytes(),
        &parsed.source.relative_path,
        &[],
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
    namespaces: &[String],
    current_class: Option<String>,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    let mut class_for_children = current_class.clone();
    let mut namespaces_for_children = namespaces.to_vec();

    match node.kind() {
        "class_specifier" => {
            if let Some(name) = child_name(node, source) {
                let qualified = build_namespace_qualified(namespaces, &name);
                push_syntax_symbol(symbols, NodeKind::Class, path, node, &name, &qualified);
                class_for_children = Some(name);
            }
        }
        "struct_specifier" => {
            if let Some(name) = child_name(node, source) {
                let qualified = build_namespace_qualified(namespaces, &name);
                push_syntax_symbol(symbols, NodeKind::Struct, path, node, &name, &qualified);
                class_for_children = Some(name);
            }
        }
        "function_definition" => {
            if let Some(name) = extract_function_name(node, source) {
                let kind = if current_class.is_some() {
                    NodeKind::Method
                } else {
                    NodeKind::Function
                };
                let mut parts: Vec<&str> = namespaces.iter().map(String::as_str).collect();
                if let Some(ref class) = current_class {
                    parts.push(class);
                }
                let qualified = if parts.is_empty() {
                    name.clone()
                } else {
                    format!("{}::{}", parts.join("::"), name)
                };
                push_syntax_symbol(symbols, kind, path, node, &name, &qualified);
            }
        }
        "namespace_definition" => {
            if let Some(name) = child_name(node, source) {
                let qualified = build_namespace_qualified(namespaces, &name);
                push_syntax_symbol(symbols, NodeKind::Module, path, node, &name, &qualified);
                namespaces_for_children.push(name);
            }
        }
        "template_declaration" => {
            // Pass-through: recurse into children to extract the inner declaration
        }
        _ => {}
    }

    for index in 0..node.named_child_count() {
        if let Some(child) = node.named_child(index) {
            walk_node(
                child,
                source,
                path,
                &namespaces_for_children,
                class_for_children.clone(),
                symbols,
            );
        }
    }
}

fn extract_function_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    drill_declarator(declarator, source)
}

fn drill_declarator(node: Node<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => node.utf8_text(source).ok().map(|s| s.to_string()),
        "qualified_identifier" => {
            let text = node.utf8_text(source).ok()?;
            text.split("::").last().map(|s| s.to_string())
        }
        "destructor_name" => Some(node.utf8_text(source).ok()?.to_string()),
        "function_declarator"
        | "pointer_declarator"
        | "reference_declarator"
        | "parenthesized_declarator"
        | "array_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|child| drill_declarator(child, source)),
        "template_function" => node
            .child_by_field_name("declarator")
            .and_then(|child| drill_declarator(child, source)),
        "operator_name" => {
            let text = node.utf8_text(source).ok()?;
            Some(format!("operator{}", text.trim_start_matches("operator")))
        }
        _ => None,
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
        base_classes: Vec::new(),
    });
}

#[cfg(test)]
mod tests {
    use crate::{build_graph, NodeKind};
    use std::fs;

    #[test]
    fn extracts_cpp_class_methods_and_free_functions() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("auth.cpp"),
            r#"
namespace auth {

class AuthService {
public:
    bool validate(const std::string& token) {
        return check(token);
    }
};

bool check(const std::string& token) {
    return token.size() > 0;
}

} // namespace auth
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();

        let ns = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "auth" && node.kind == NodeKind::Module)
            .expect("namespace should be extracted as Module");
        assert_eq!(ns.kind, NodeKind::Module);

        let svc = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "auth::AuthService")
            .expect("class should be extracted");
        assert_eq!(svc.kind, NodeKind::Class);

        let validate = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "auth::AuthService::validate")
            .expect("method should be extracted with class-qualified name");
        assert_eq!(validate.kind, NodeKind::Method);
        assert!(validate.tags.contains(&"tree-sitter".to_string()));

        let check = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "auth::check")
            .expect("free function should be extracted with namespace-qualified name");
        assert_eq!(check.kind, NodeKind::Function);

        assert!(graph.edges.iter().any(|edge| {
            edge.from == validate.id
                && graph
                    .node(&edge.to)
                    .map(|node| node.name == "check")
                    .unwrap_or(false)
        }));
    }

    #[test]
    fn extracts_cpp_struct_and_include() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("config.hpp"),
            r#"
#include <string>
#include "types.h"

struct Config {
    int timeout;
};
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();

        let cfg = graph
            .nodes
            .iter()
            .find(|node| node.name == "Config")
            .expect("struct should be extracted");
        assert_eq!(cfg.kind, NodeKind::Struct);

        let imports: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == crate::EdgeKind::Imports)
            .collect();
        assert!(imports.len() >= 2, "should have at least 2 include imports");
    }

    #[test]
    fn extracts_template_class_and_function() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("vec.hpp"),
            r#"
template <typename T>
class Vector {
public:
    void push_back(const T& value) {}
    size_t size() const { return 0; }
};

template <typename T>
T max(T a, T b) {
    return a > b ? a : b;
}
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();

        let vec = graph
            .nodes
            .iter()
            .find(|node| node.name == "Vector")
            .expect("template class should be extracted");
        assert_eq!(vec.kind, NodeKind::Class);

        let push = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == "Vector::push_back")
            .expect("template class method should be extracted");
        assert_eq!(push.kind, NodeKind::Method);

        let max_fn = graph
            .nodes
            .iter()
            .find(|node| node.name == "max")
            .expect("template function should be extracted");
        assert_eq!(max_fn.kind, NodeKind::Function);
    }
}
