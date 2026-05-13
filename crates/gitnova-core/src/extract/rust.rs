use super::{fill_symbol_text_and_calls, import_from_line, ExtractedSymbol, FileExtraction};
use crate::model::NodeKind;
use crate::parser::{line_span, ParsedFile};
use regex::Regex;

pub fn extract(parsed: &ParsedFile) -> FileExtraction {
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
