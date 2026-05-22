use super::{fill_symbol_text_and_calls, import_from_line, ExtractedSymbol, FileExtraction};
use crate::model::{NodeKind, Span};
use crate::parser::{line_span, ParsedFile};
use regex::Regex;
use std::collections::HashSet;
use tree_sitter::{Node, Parser, StreamingIterator};

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
    let source = parsed.text.as_bytes();
    let path = &parsed.source.relative_path;

    let query = tree_sitter::Query::new(&language, include_str!("cpp_queries.scm")).ok()?;
    let mut cursor = tree_sitter::QueryCursor::new();
    let mut symbols = Vec::new();

    let mut matches = cursor.matches(&query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        if m.captures.is_empty() {
            continue;
        }

        // Collect capture names and nodes for this match
        let mut name_text: Option<String> = None;
        let mut def_node: Option<Node> = None;
        let mut def_capture_name: Option<&str> = None;
        let mut qname_node: Option<Node> = None;

        for cap in m.captures {
            let cap_name = query.capture_names()[cap.index as usize];
            match cap_name {
                "name" => {
                    if let Ok(text) = cap.node.utf8_text(source) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            name_text = Some(trimmed.to_string());
                        }
                    }
                }
                "qname" => {
                    qname_node = Some(cap.node);
                }
                cn if cn.starts_with("def.") => {
                    def_node = Some(cap.node);
                    def_capture_name = Some(cn);
                }
                _ => {}
            }
        }

        // For out-of-line function definitions, extract name + class from qname
        let mut class_prefix: Option<String> = None;
        if def_capture_name == Some("def.function_outline") {
            if let Some(qn) = qname_node {
                if let Ok(text) = qn.utf8_text(source) {
                    let mut parts: Vec<&str> = text.split("::").collect();
                    if parts.len() >= 2 {
                        name_text = parts.pop().map(|s| s.to_string());
                        class_prefix = parts.pop().map(|s| s.to_string());
                    }
                }
            }
        }

        let def_node = match def_node {
            Some(n) => n,
            None => continue,
        };

        // For fallback class/struct captures, extract name from the def_node itself
        let name = match (name_text, def_capture_name) {
            (Some(n), _) => n,
            (None, Some("def.class_fallback")) => {
                child_name(def_node, source).unwrap_or_else(|| "unknown".to_string())
            }
            (None, Some("def.struct_fallback")) => {
                child_name(def_node, source).unwrap_or_else(|| "unknown".to_string())
            }
            _ => continue,
        };

        // Filter C++ keywords and literals that tree-sitter misidentifies as identifiers
        if is_cpp_keyword_or_literal(&name) {
            continue;
        }

        // Determine NodeKind
        let mut kind = match def_capture_name {
            Some("def.class") | Some("def.class_fallback") => NodeKind::Class,
            Some("def.struct") | Some("def.struct_fallback") => NodeKind::Struct,
            Some("def.enum") => NodeKind::Enum,
            Some("def.enum_value") => NodeKind::Variable,
            Some("def.union") => NodeKind::Union,
            Some("def.typedef") => NodeKind::Typedef,
            Some("def.variable") => NodeKind::Variable,
            Some("def.macro") => NodeKind::Macro,
            Some("def.label") => NodeKind::Variable,
            Some("def.namespace") => NodeKind::Module,
            Some("def.function_decl") => {
                // Determine function vs method based on enclosing class/struct
                if enclosing_class(def_node, source).is_some() {
                    NodeKind::Method
                } else {
                    NodeKind::Function
                }
            }
            Some("def.function_outline") => {
                // Out-of-line in .cpp: method if has class prefix, else function
                if class_prefix.is_some() {
                    NodeKind::Method
                } else {
                    NodeKind::Function
                }
            }
            _ => continue,
        };

        // Build qualified name from enclosing namespace + class chain
        // For function_decl, use class_from_qname (out-of-line) or enclosing class (inline)
        let class_for_qname = if class_prefix.is_some() {
            class_prefix.as_deref()
        } else if kind == NodeKind::Method {
            // For inline methods, the enclosing class name is found during parent walk
            None // build_qualified_from_enclosing will find it via parent chain
        } else {
            None
        };
        let qualified = build_qualified_from_enclosing(def_node, source, &name, &kind, class_for_qname);

        // Extract base classes for class/struct
        let base_classes = if matches!(kind, NodeKind::Class | NodeKind::Struct) {
            extract_base_classes(def_node, source)
        } else {
            Vec::new()
        };

        symbols.push(ExtractedSymbol {
            kind,
            name: name.clone(),
            qualified_name: qualified,
            path: path.to_string(),
            span: span_from_node(def_node),
            text: String::new(),
            calls: Vec::new(),      // filled later by fill_symbol_text_and_calls
            tags: vec!["tree-sitter".into(), "query".into()],
            base_classes,
        });
    }

    if symbols.is_empty() {
        None
    } else {
        Some(symbols)
    }
}

/// Filter C++ keywords and literal values that tree-sitter captures as identifiers.
fn is_cpp_keyword_or_literal(name: &str) -> bool {
    matches!(
        name,
        "true" | "false" | "nullptr" | "NULL" | "this"
            | "if" | "else" | "for" | "while" | "do" | "switch" | "case"
            | "return" | "break" | "continue" | "goto" | "throw"
            | "class" | "struct" | "enum" | "union" | "namespace"
            | "public" | "private" | "protected" | "virtual" | "static"
            | "const" | "volatile" | "inline" | "explicit" | "friend"
            | "template" | "typename" | "typedef"
            | "new" | "delete" | "sizeof" | "typeid"
            | "try" | "catch" | "noexcept" | "override" | "final"
            | "auto" | "decltype" | "constexpr" | "consteval" | "constinit"
    ) || name.starts_with("__") // compiler builtins
}

/// Walk parent chain to find enclosing class_specifier or struct_specifier.
fn enclosing_class(mut node: Node<'_>, source: &[u8]) -> Option<String> {
    while let Some(parent) = node.parent() {
        match parent.kind() {
            "class_specifier" | "struct_specifier" => {
                return child_name(parent, source);
            }
            "function_definition" | "compound_statement" | "field_declaration_list" => {
                // Keep walking up through these
            }
            "namespace_definition" | "translation_unit" => {
                return None;
            }
            _ => {}
        }
        node = parent;
    }
    None
}

/// Build qualified name from the enclosing namespace/class chain above a node.
fn build_qualified_from_enclosing(
    node: Node<'_>,
    source: &[u8],
    name: &str,
    kind: &NodeKind,
    class_from_qname: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = node;

    // Walk up parent chain collecting namespace and class names
    loop {
        if let Some(parent) = cur.parent() {
            match parent.kind() {
                "namespace_definition" => {
                    if let Some(ns) = child_name(parent, source) {
                        parts.push(ns);
                    }
                }
                "class_specifier" | "struct_specifier" => {
                    if let Some(cls) = child_name(parent, source) {
                        // Don't add the current node's own class name if
                        // the node itself IS the class/struct specifier
                        if node.kind() != "class_specifier" && node.kind() != "struct_specifier" {
                            parts.push(cls);
                        }
                    }
                }
                "template_declaration" => {
                    // Pass-through: class/struct may be inside template
                }
                "translation_unit" => break,
                _ => {}
            }
            cur = parent;
        } else {
            break;
        }
    }

    parts.reverse();

    // For out-of-line methods, add the class prefix from the qname
    if let Some(cls) = class_from_qname {
        parts.push(cls.to_string());
    }

    // Only add the name itself if it's not a module (namespace)
    if !matches!(kind, NodeKind::Module) {
        parts.push(name.to_string());
    }

    if parts.is_empty() {
        name.to_string()
    } else {
        parts.join("::")
    }
}

fn child_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|child| child.utf8_text(source).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn extract_base_classes(node: Node<'_>, source: &[u8]) -> Vec<String> {
    // base_class_clause is a named child of class_specifier, not a field child
    let base_clause = (0..node.named_child_count())
        .filter_map(|i| node.named_child(i))
        .find(|child| child.kind() == "base_class_clause");
    let Some(base_clause) = base_clause else {
        return Vec::new();
    };
    let mut base_classes = Vec::new();
    for i in 0..base_clause.named_child_count() {
        if let Some(child) = base_clause.named_child(i) {
            if child.kind() == "type_identifier" {
                if let Ok(name) = child.utf8_text(source) {
                    base_classes.push(name.to_string());
                }
            }
        }
    }
    base_classes
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
    let mut merged: Vec<ExtractedSymbol> = Vec::new();
    for symbol in syntax_symbols.into_iter().chain(heuristic_symbols) {
        // When the same symbol (same path, name, kind, start_line) appears with
        // different qualified_names (namespace pollution), keep the shortest one.
        if let Some(existing) = merged.iter_mut().find(|s| {
            s.path == symbol.path
                && s.name == symbol.name
                && s.kind == symbol.kind
                && s.span.start_line == symbol.span.start_line
        }) {
            if symbol.qualified_name.len() < existing.qualified_name.len() {
                // Merge base_classes from the replacement
                let mut base: Vec<String> = std::mem::take(&mut existing.base_classes);
                base.extend(symbol.base_classes.clone());
                *existing = symbol;
                existing.base_classes = base;
            }
            continue;
        }
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

    #[test]
    fn extracts_inheritance_and_out_of_line_methods() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(
            temp.path().join("types.hpp"),
            r#"
namespace build {
class WorkItemBase {
public:
    virtual void doProcess() = 0;
};
class BuildWorkItem : public WorkItemBase {
public:
    void doProcess() override;
};
}
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join("worker.cpp"),
            r#"
#include "types.hpp"
namespace build {
void BuildWorkItem::doProcess() {}
}
"#,
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();

        // Out-of-line method in .cpp should be Method with class-qualified name
        let method = graph
            .nodes
            .iter()
            .find(|node| {
                node.qualified_name == "build::BuildWorkItem::doProcess"
                    && node.kind == NodeKind::Method
            })
            .expect("out-of-line method should have class-qualified name and Method kind");
        assert_eq!(method.kind, NodeKind::Method);

        // Inheritance edge should exist
        let extends_edges: Vec<_> = graph
            .edges
            .iter()
            .filter(|edge| edge.kind == crate::EdgeKind::Extends)
            .collect();
        assert!(
            !extends_edges.is_empty(),
            "should have at least one extends edge"
        );

        let build_work_item = graph
            .nodes
            .iter()
            .find(|node| node.name == "BuildWorkItem" && node.kind == NodeKind::Class)
            .unwrap();
        let work_item_base = graph
            .nodes
            .iter()
            .find(|node| node.name == "WorkItemBase" && node.kind == NodeKind::Class)
            .unwrap();
        assert!(extends_edges.iter().any(|edge| {
            edge.from == build_work_item.id && edge.to == work_item_base.id
        }));
    }

    #[test]
    fn preserves_class_after_cross_file_namespace_dedup() {
        let temp = tempfile::TempDir::new().unwrap();
        // File 1: full namespace qualifier
        fs::write(
            temp.path().join("tracer.h"),
            "namespace indexlib::index {\nclass InvertedIndexSearchTracer {\npublic:\n    void SetBitmapTerm();\n};\n}\n",
        )
        .unwrap();
        // File 2: shorter namespace (from "using namespace indexlib")
        fs::write(
            temp.path().join("posting.h"),
            "namespace indexlib {\nclass InvertedIndexSearchTracer {\npublic:\n    bool IsBitmapTerm();\n};\n}\n",
        )
        .unwrap();
        // File 3: same class name, different namespace
        fs::write(
            temp.path().join("other.h"),
            "namespace util {\nclass InvertedIndexSearchTracer {\npublic:\n    int count;\n};\n}\n",
        )
        .unwrap();

        let graph = build_graph(temp.path()).unwrap();

        // Should have at least one class node for InvertedIndexSearchTracer
        let tracer_classes: Vec<_> = graph
            .nodes
            .iter()
            .filter(|node| node.name == "InvertedIndexSearchTracer" && node.kind == NodeKind::Class)
            .collect();
        assert!(!tracer_classes.is_empty(), "InvertedIndexSearchTracer class must survive cross-file dedup");
        // util::InvertedIndexSearchTracer MUST NOT be merged into indexlib variants
        assert!(tracer_classes.iter().any(|n| n.qualified_name.contains("util")),
            "util::InvertedIndexSearchTracer should not be merged into indexlib:: variants");
    }
}
